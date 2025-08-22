// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::drain::DrainWatcher;
use agent_core::version::BuildInfo;
use agent_core::{signal, telemetry};
#[cfg(feature = "pat")]
use http_body_util::BodyExt;
use hyper::Request;
use hyper::body::Incoming;
#[cfg(feature = "pat")]
use hyper::header::CACHE_CONTROL;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use tokio::time;
#[cfg(feature = "pat")]
use tracing::error;
use tracing::{info, trace, warn}; // for .collect() on Incoming bodies (PAT token POST body parsing)
// token exposure now via ZeroToken wrapper's expose() method
use tracing_subscriber::filter;

use super::hyper_helpers::{Server, empty_response, plaintext_response};
use crate::Config;
use crate::client::Client;
use crate::http::Response; // for admin JWT construction

pub trait ConfigDumpHandler: Sync + Send {
	fn key(&self) -> &'static str;
	// sadly can't use async trait because no Sync
	// see: https://github.com/dtolnay/async-trait/issues/248, https://github.com/dtolnay/async-trait/issues/142
	// we can't use FutureExt::shared because our result is not clonable
	fn handle(&self) -> anyhow::Result<serde_json::Value>;
}

pub type AdminResponse = std::pin::Pin<Box<dyn Future<Output = crate::http::Response> + Send>>;

pub trait AdminFallback: Sync + Send {
	// sadly can't use async trait because no Sync
	// see: https://github.com/dtolnay/async-trait/issues/248, https://github.com/dtolnay/async-trait/issues/142
	// we can't use FutureExt::shared because our result is not clonable
	fn handle(&self, req: http::Request<Incoming>) -> AdminResponse;
}

struct State {
	stores: crate::store::Stores,
	config: Arc<Config>,
	shutdown_trigger: signal::ShutdownTrigger,
	config_dump_handlers: Vec<Arc<dyn ConfigDumpHandler>>,
	admin_fallback: Option<Arc<dyn AdminFallback>>,
	#[cfg(feature = "pat")]
	pat_db: Option<Arc<sqlx::PgPool>>, // optional PAT database pool for token CRUD
	#[cfg(feature = "pat")]
	pat_rate_limit: Arc<moka::future::Cache<String, Vec<std::time::Instant>>>, // rate limit cache
	/// Optional JwtAuth policy targeting ONLY the admin server (PolicyTarget::Admin)
	admin_jwt: Option<crate::http::jwt::Jwt>,
}

pub struct Service {
	s: Server<State>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDump {
	#[serde(flatten)]
	stores: crate::store::Stores,
	version: BuildInfo,
	config: Arc<Config>,
}

#[derive(serde::Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CertDump {
	// Not available via Envoy, but still useful.
	pem: String,
	serial_number: String,
	valid_from: String,
	expiration_time: String,
}

#[derive(serde::Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CertsDump {
	identity: String,
	state: String,
	cert_chain: Vec<CertDump>,
	root_certs: Vec<CertDump>,
}

impl Service {
	pub async fn new(
		config: Arc<Config>,
		stores: crate::store::Stores,
		shutdown_trigger: signal::ShutdownTrigger,
		drain_rx: DrainWatcher,
	) -> anyhow::Result<Self> {
		Server::<State>::bind(
			"admin",
			config.admin_addr,
			drain_rx,
			State {
				config,
				stores,
				shutdown_trigger,
				config_dump_handlers: vec![],
				admin_fallback: None,
				#[cfg(feature = "pat")]
				pat_db: None,
				#[cfg(feature = "pat")]
				pat_rate_limit: Arc::new(
					moka::future::Cache::builder()
						.time_to_live(std::time::Duration::from_secs(60))
						.build(),
				),
				admin_jwt: None,
			},
		)
		.await
		.map(|s| Service { s })
	}

	pub fn address(&self) -> SocketAddr {
		self.s.address()
	}

	pub fn add_config_dump_handler(&mut self, handler: Arc<dyn ConfigDumpHandler>) {
		self.s.state_mut().config_dump_handlers.push(handler);
	}

	pub fn set_admin_handler(&mut self, handler: Arc<dyn AdminFallback>) {
		self.s.state_mut().admin_fallback = Some(handler);
	}

	#[cfg(feature = "pat")]
	pub fn set_pat_db(&mut self, pool: Arc<sqlx::PgPool>) {
		self.s.state_mut().pat_db = Some(pool);
	}

	pub fn set_admin_jwt(&mut self, jwt: crate::http::jwt::Jwt) {
		self.s.state_mut().admin_jwt = Some(jwt);
	}

	pub fn spawn(self) {
		self.s.spawn(|state, mut req| async move {
			// Apply admin-scoped JWT (separate from proxy listener) if configured
			if let Some(jwt) = &state.admin_jwt {
				// Attempt Authorization: Bearer first
				let token_opt = req
					.headers()
					.get(http::header::AUTHORIZATION)
					.and_then(|hv| hv.to_str().ok())
					.and_then(|s| {
						let (scheme, rest) = s.split_once(' ')?;
						if scheme.eq_ignore_ascii_case("Bearer") {
							Some(rest.to_string())
						} else {
							None
						}
					});
				let token_opt = token_opt.or_else(|| {
					use std::sync::OnceLock;
					static ADDITIONAL_JWT_HEADERS: OnceLock<Vec<http::header::HeaderName>> = OnceLock::new();
					let headers = ADDITIONAL_JWT_HEADERS.get_or_init(|| {
						std::env::var("JWT_ASSERTION_HEADERS")
							.ok()
							.map(|v| {
								v.split(',')
									.map(|s| s.trim())
									.filter(|s| !s.is_empty())
									.filter_map(|name| {
										http::header::HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes()).ok()
									})
									.collect()
							})
							.unwrap_or_default()
					});
					for h in headers.iter() {
						if let Some(val) = req.headers().get(h) {
							if let Ok(s) = val.to_str() {
								if !s.is_empty() {
									return Some(s.to_string());
								}
							}
						}
					}
					None
				});
				if let Some(token) = token_opt {
					match jwt.validate_claims(&token) {
						Ok(claims) => {
							// Strip Authorization to avoid accidental propagation and insert claims
							req.headers_mut().remove(http::header::AUTHORIZATION);
							req.extensions_mut().insert(claims);
						},
						Err(e) => {
							trace!(error=?e, "admin jwt validation failed");
						},
					}
				} else {
					trace!("no admin jwt present in request");
				}
			}
			// Fast path: PAT token management endpoints (feature gated)
			#[cfg(feature = "pat")]
			if let Some(resp) = handle_pat_api(&state, &mut req).await {
				return Ok(resp);
			}
			match req.uri().path() {
				#[cfg(target_os = "linux")]
				"/debug/pprof/profile" => handle_pprof(req).await,
				#[cfg(target_os = "linux")]
				"/debug/pprof/heap" => handle_jemalloc_pprof_heapgen(req).await,
				"/quitquitquit" => Ok(
					handle_server_shutdown(
						state.shutdown_trigger.clone(),
						req,
						state.config.termination_min_deadline,
					)
					.await,
				),
				"/config_dump" => {
					handle_config_dump(
						&state.config_dump_handlers,
						ConfigDump {
							stores: state.stores.clone(),
							version: BuildInfo::new(),
							config: state.config.clone(),
						},
					)
					.await
				},
				"/logging" => Ok(handle_logging(req).await),
				_ => {
					if let Some(h) = &state.admin_fallback {
						Ok(h.handle(req).await)
					} else if req.uri().path() == "/" {
						Ok(handle_dashboard(req).await)
					} else {
						Ok(empty_response(hyper::StatusCode::NOT_FOUND))
					}
				},
			}
		})
	}
}

#[cfg(feature = "pat")]
#[derive(serde::Serialize)]
struct PublicToken {
	id: String, // internal identifier (not rendered in UI table)
	token_prefix: String,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	scopes: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	creator_email: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	creator_groups: Vec<String>,
	created_at: chrono::DateTime<chrono::Utc>,
	expires_at: Option<chrono::DateTime<chrono::Utc>>,
	revoked_at: Option<chrono::DateTime<chrono::Utc>>,
	last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(feature = "pat")]
#[derive(serde::Deserialize)]
struct CreateTokenRequest {
	#[serde(default)]
	scopes: Vec<String>,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	expires_at: Option<chrono::DateTime<chrono::Utc>>, // ISO8601
	                                                   // tenant_id/user_id overrides removed for safety; identity derived from authenticated principal
}

#[cfg(feature = "pat")]
async fn handle_pat_api(state: &State, req: &mut http::Request<Incoming>) -> Option<Response> {
	use hyper::Method;
	use hyper::header::AUTHORIZATION;
	// Runtime check: PAT must be compiled in AND enabled in config AND database must be configured
	if !state.config.pat.enabled || state.pat_db.is_none() {
		return None;
	}
	let path = req.uri().path();
	if !path.starts_with("/api/tokens") {
		return None;
	}
	// Basic request instrumentation (doesn't log full token, only prefix for correlation)
	let (has_authz, authz_prefix) = if let Some(h) = req
		.headers()
		.get(AUTHORIZATION)
		.and_then(|v| v.to_str().ok())
	{
		if let Some(rest) = h.strip_prefix("Bearer ") {
			(true, rest.chars().take(8).collect::<String>())
		} else {
			(true, "<non-bearer>".into())
		}
	} else {
		(false, String::new())
	};
	let has_claims = req.extensions().get::<crate::http::jwt::Claims>().is_some();
	tracing::debug!(target="audit", action="pat.api.req", path=%path, method=%req.method(), has_authz, authz_prefix=%authz_prefix, has_claims, "PAT API request");
	// Ensure DB availability
	let Some(pool) = state.pat_db.clone() else {
		return Some(plaintext_response(
			hyper::StatusCode::SERVICE_UNAVAILABLE,
			"token management unavailable".into(),
		));
	};
	// Derive identity from JWT claims if present (works for both JWT and PAT auth)
	let (tenant_id, user_id, creator_email, creator_groups) = if let Some(claims) =
		req.extensions().get::<crate::http::jwt::Claims>()
	{
		let tenant = claims
			.inner
			.get("tenant_id")
			.and_then(|v| v.as_str())
			.unwrap_or("default")
			.to_string();
		let user = match claims.inner.get("sub").and_then(|v| v.as_str()) {
			Some(u) => u,
			None => {
				tracing::warn!(target="audit", action="pat.api.auth", reason="missing_subject", has_authz, authz_prefix=%authz_prefix, "PAT API unauthorized: missing subject claim");
				return Some(json_error(
					hyper::StatusCode::UNAUTHORIZED,
					"missing subject in claims",
				));
			},
		};
		let email = claims
			.inner
			.get("email")
			.and_then(|v| v.as_str())
			.map(|s| s.to_string());
		let groups = claims
			.inner
			.get("groups")
			.and_then(|v| v.as_array())
			.map(|arr| {
				arr
					.iter()
					.filter_map(|v| v.as_str().map(|s| s.to_string()))
					.collect()
			})
			.unwrap_or_default();
		(tenant, user.to_string(), email, groups)
	} else {
		// Fallback to headers for backwards compatibility
		let tenant = req
			.headers()
			.get("x-tenant-id")
			.and_then(|v| v.to_str().ok())
			.unwrap_or("default")
			.to_string();
		let user = match req.headers().get("x-user-id").and_then(|v| v.to_str().ok()) {
			Some(u) => u,
			None => {
				tracing::warn!(target="audit", action="pat.api.auth", reason="missing_user_header", has_authz, authz_prefix=%authz_prefix, has_claims, "PAT API unauthorized: missing x-user-id header (no JWT claims)");
				return Some(json_error(
					hyper::StatusCode::UNAUTHORIZED,
					"missing user identity",
				));
			},
		};
		(tenant, user.to_string(), None, vec![])
	};

	// Authorization: currently any authenticated principal for the tenant may manage its own tokens.
	// (Future: add per-user or role-based restrictions here if needed.)
	const MAX_CREATES_PER_MIN: usize = 10;

	if path == "/api/tokens" && req.method() == Method::GET {
		// Query params: limit, offset, search
		let qp: HashMap<String, String> = req
			.uri()
			.query()
			.map(|v| {
				url::form_urlencoded::parse(v.as_bytes())
					.into_owned()
					.collect()
			})
			.unwrap_or_default();
		let limit = qp
			.get("limit")
			.and_then(|v| v.parse::<i64>().ok())
			.unwrap_or(50);
		let offset = qp
			.get("offset")
			.and_then(|v| v.parse::<i64>().ok())
			.unwrap_or(0);
		let search = qp.get("search").map(|s| s.as_str());
		let repo = crate::http::pat::TokenRepo::new(pool.as_ref().clone());
		match repo
			.list_public(&tenant_id, &user_id, limit, offset, search)
			.await
		{
			Ok(rows) => {
				let body: Vec<PublicToken> = rows
					.into_iter()
					.map(|r| PublicToken {
						id: r.id.to_string(),
						token_prefix: r.token_prefix,
						scopes: r.scopes,
						creator_email: r.creator_email,
						creator_groups: r.creator_groups,
						created_at: r.created_at,
						expires_at: r.expires_at,
						revoked_at: r.revoked_at,
						last_used_at: r.last_used_at,
					})
					.collect();
				let bytes = serde_json::to_vec(&body).unwrap();
				let resp = ::http::Response::builder()
					.status(hyper::StatusCode::OK)
					.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
					.header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
					.header(hyper::header::PRAGMA, HeaderValue::from_static("no-cache"))
					.body(bytes.into())
					.expect("valid response");
				return Some(resp);
			},
			Err(_e) => {
				return Some(json_error(
					hyper::StatusCode::INTERNAL_SERVER_ERROR,
					"list failed",
				));
			},
		}
	} else if path == "/api/tokens" && req.method() == Method::POST {
		// Rate limit check using moka cache
		{
			let key = format!("{tenant_id}:{user_id}");
			let now = std::time::Instant::now();
			let window_start = now - std::time::Duration::from_secs(60);

			let mut attempts = state.pat_rate_limit.get(&key).await.unwrap_or_default();
			attempts.retain(|t| *t >= window_start);

			if attempts.len() >= MAX_CREATES_PER_MIN {
				return Some(json_error(
					hyper::StatusCode::TOO_MANY_REQUESTS,
					"rate limit exceeded",
				));
			}

			attempts.push(now);
			state.pat_rate_limit.insert(key, attempts).await;
		}
		// Parse request body using axum's built-in JSON extraction
		let body_bytes = match req.body_mut().collect().await {
			Ok(collected) => collected.to_bytes(),
			Err(_) => return Some(json_error(hyper::StatusCode::BAD_REQUEST, "invalid body")),
		};

		// Size limit check
		if body_bytes.len() > 64 * 1024 {
			return Some(json_error(
				hyper::StatusCode::PAYLOAD_TOO_LARGE,
				"body too large",
			));
		}

		let payload: CreateTokenRequest = match serde_json::from_slice(&body_bytes) {
			Ok(p) => p,
			Err(e) => {
				return Some(json_error(
					hyper::StatusCode::BAD_REQUEST,
					&format!("invalid JSON: {}", e),
				));
			},
		};
		// Expiration policy: enforce maximum horizon from config
		let max_days = state.config.pat.max_expiry_days;
		if let Some(exp) = payload.expires_at {
			if exp > chrono::Utc::now() + chrono::Duration::days(max_days) {
				return Some(json_error(
					hyper::StatusCode::BAD_REQUEST,
					&format!("expires_at exceeds maximum of {} days", max_days),
				));
			}
		}
		let repo = crate::http::pat::TokenRepo::new(pool.as_ref().clone());
		let t = &tenant_id;
		let u = &user_id;
		// Creator metadata already captured from claims above
		let params = crate::http::pat::CreateTokenParams {
			creator: u,
			creator_email: creator_email.as_deref(),
			creator_groups: &creator_groups,
			tenant_id: t,
			user_id: u,
			name: payload.name.as_deref(),
			scopes: &payload.scopes,
			expires_at: payload.expires_at,
		};
		match repo.create(params).await {
			Ok((token, row)) => {
				#[derive(serde::Serialize)]
				struct CreateTokenResponse<'a> {
					token: &'a str,
					token_record: PublicToken,
				}
				let pub_row = PublicToken {
					id: row.id.to_string(),
					token_prefix: row.token_prefix,
					scopes: row.scopes,
					creator_email: row.creator_email,
					creator_groups: row.creator_groups,
					created_at: row.created_at,
					expires_at: row.expires_at,
					revoked_at: row.revoked_at,
					last_used_at: row.last_used_at,
				};
				info!(target: "audit", action = "pat.create", tenant = %t, user = %u, token_prefix = %pub_row.token_prefix, scopes = ?pub_row.scopes, expires_at = ?pub_row.expires_at, name = ?payload.name, "personal access token created");
				let json = serde_json::to_string(&CreateTokenResponse {
					token: token.expose(),
					token_record: pub_row,
				})
				.unwrap();
				let mut resp = plaintext_response(hyper::StatusCode::OK, json);
				resp
					.headers_mut()
					.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
				resp
					.headers_mut()
					.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
				resp
					.headers_mut()
					.insert(hyper::header::PRAGMA, HeaderValue::from_static("no-cache"));
				return Some(resp);
			},
			Err(e) => {
				error!(target: "audit", action = "pat.create", tenant = %t, user = %u, error = %e, "failed to create personal access token");
				return Some(json_error(
					hyper::StatusCode::INTERNAL_SERVER_ERROR,
					"create failed",
				));
			},
		}
	} else if let Some(id) = path.strip_prefix("/api/tokens/")
		&& req.method() == Method::DELETE
	{
		if let Ok(uuid) = id.parse::<sqlx::types::Uuid>() {
			let repo = crate::http::pat::TokenRepo::new(pool.as_ref().clone());
			match repo.revoke(&tenant_id, uuid).await {
				Ok(Some(prefix)) => {
					crate::http::pat::mark_pat_revoked(&prefix);
					info!(target: "audit", action = "pat.revoke", tenant = %tenant_id, user = %user_id, token_id = %uuid, token_prefix=%prefix, status = "revoked", "personal access token revoked");
				},
				Ok(None) => {
					info!(target: "audit", action = "pat.revoke", tenant = %tenant_id, user = %user_id, token_id = %uuid, status = "not_found", "personal access token revoke attempted (not found)");
				},
				Err(e) => {
					error!(target: "audit", action = "pat.revoke", tenant = %tenant_id, user = %user_id, token_id = %uuid, status = "error", error = %e, "personal access token revoke failed");
				},
			}
		} else {
			warn!(target: "audit", action = "pat.revoke", tenant = %tenant_id, user = %user_id, token_id = %id, status = "invalid_id", "personal access token revoke attempted with invalid id");
		}
		let mut resp = plaintext_response(hyper::StatusCode::NO_CONTENT, "".into());
		resp
			.headers_mut()
			.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
		resp
			.headers_mut()
			.insert(hyper::header::PRAGMA, HeaderValue::from_static("no-cache"));
		return Some(resp);
	}
	None
}

#[cfg(feature = "pat")]
fn json_error(status: hyper::StatusCode, msg: &str) -> Response {
	let body = serde_json::json!({"error": msg}).to_string();
	let mut resp = plaintext_response(status, body);
	resp
		.headers_mut()
		.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	resp
		.headers_mut()
		.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
	resp
}

async fn handle_dashboard(_req: Request<Incoming>) -> Response {
	let apis = &[
		(
			"debug/pprof/profile",
			"build profile using the pprof profiler (if supported)",
		),
		(
			"debug/pprof/heap",
			"collect heap profiling data (if supported, requires jmalloc)",
		),
		("quitquitquit", "shut down the server"),
		("config_dump", "dump the current agentgateway configuration"),
		("logging", "query/changing logging levels"),
	];

	let mut api_rows = String::new();

	for (index, (path, description)) in apis.iter().copied().enumerate() {
		api_rows.push_str(&format!(
            "<tr class=\"{row_class}\"><td class=\"home-data\"><a href=\"{path}\">{path}</a></td><td class=\"home-data\">{description}</td></tr>\n",
            row_class = if index % 2 == 1 { "gray" } else { "vert-space" },
            path = path,
            description = description
        ));
	}

	let html_str = include_str!("../assets/dashboard.html");
	let html_str = html_str.replace("<!--API_ROWS_PLACEHOLDER-->", &api_rows);

	let mut response = plaintext_response(hyper::StatusCode::OK, html_str);
	response.headers_mut().insert(
		CONTENT_TYPE,
		HeaderValue::from_static("text/html; charset=utf-8"),
	);

	response
}

#[cfg(target_os = "linux")]
async fn handle_pprof(_req: Request<Incoming>) -> anyhow::Result<Response> {
	use pprof::protos::Message;
	let guard = pprof::ProfilerGuardBuilder::default()
		.frequency(1000)
		// .blocklist(&["libc", "libgcc", "pthread", "vdso"])
		.build()?;

	tokio::time::sleep(Duration::from_secs(10)).await;
	let report = guard.report().build()?;
	let profile = report.pprof()?;

	let body = profile.write_to_bytes()?;

	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.body(body.into())
			.expect("builder with known status code should not fail"),
	)
}

async fn handle_server_shutdown(
	shutdown_trigger: signal::ShutdownTrigger,
	_req: Request<Incoming>,
	self_term_wait: Duration,
) -> Response {
	match *_req.method() {
		hyper::Method::POST => {
			match time::timeout(self_term_wait, shutdown_trigger.shutdown_now()).await {
				Ok(()) => info!("Shutdown completed gracefully"),
				Err(_) => warn!(
					"Graceful shutdown did not complete in {:?}, terminating now",
					self_term_wait
				),
			}
			plaintext_response(hyper::StatusCode::OK, "shutdown now\n".into())
		},
		_ => empty_response(hyper::StatusCode::METHOD_NOT_ALLOWED),
	}
}

async fn handle_config_dump(
	handlers: &[Arc<dyn ConfigDumpHandler>],
	dump: ConfigDump,
) -> anyhow::Result<Response> {
	let serde_json::Value::Object(mut kv) = serde_json::to_value(&dump)? else {
		anyhow::bail!("config dump is not a key-value pair")
	};

	for h in handlers {
		let x = h.handle()?;
		kv.insert(h.key().to_string(), x);
	}
	let body = serde_json::to_string_pretty(&kv)?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.header(hyper::header::CONTENT_TYPE, "application/json")
			.body(body.into())
			.expect("builder with known status code should not fail"),
	)
}

// mirror envoy's behavior: https://www.envoyproxy.io/docs/envoy/latest/operations/admin#post--logging
// NOTE: multiple query parameters is not supported, for example
// curl -X POST http://127.0.0.1:15000/logging?"tap=debug&router=debug"
static HELP_STRING: &str = "
usage: POST /logging\t\t\t\t\t\t(To list current level)
usage: POST /logging?level=<level>\t\t\t\t(To change global levels)
usage: POST /logging?level={mod1}:{level1},{mod2}:{level2}\t(To change specific mods' logging level)

hint: loglevel:\terror|warn|info|debug|trace|off
hint: mod_name:\tthe module name, i.e. ztunnel::agentgateway
";
async fn handle_logging(req: Request<Incoming>) -> Response {
	match *req.method() {
		hyper::Method::POST => {
			let qp: HashMap<String, String> = req
				.uri()
				.query()
				.map(|v| {
					url::form_urlencoded::parse(v.as_bytes())
						.into_owned()
						.collect()
				})
				.unwrap_or_default();
			let level = qp.get("level").cloned();
			let reset = qp.get("reset").cloned();
			if level.is_some() || reset.is_some() {
				change_log_level(reset.is_some(), &level.unwrap_or_default())
			} else {
				list_loggers()
			}
		},
		_ => plaintext_response(
			hyper::StatusCode::METHOD_NOT_ALLOWED,
			format!("Invalid HTTP method\n {HELP_STRING}"),
		),
	}
}

fn list_loggers() -> Response {
	match telemetry::get_current_loglevel() {
		Ok(loglevel) => plaintext_response(
			hyper::StatusCode::OK,
			format!("current log level is {loglevel}\n"),
		),
		Err(err) => plaintext_response(
			hyper::StatusCode::INTERNAL_SERVER_ERROR,
			format!("failed to get the log level: {err}\n {HELP_STRING}"),
		),
	}
}

fn validate_log_level(level: &str) -> anyhow::Result<()> {
	for clause in level.split(',') {
		// We support 2 forms, compared to the underlying library
		// <level>: supported, sets the default
		// <scope>:<level>: supported, sets a scope's level
		// <scope>: sets the scope to 'trace' level. NOT SUPPORTED.
		match clause {
			"off" | "error" | "warn" | "info" | "debug" | "trace" => continue,
			s if s.contains('=') => {
				filter::Targets::from_str(s)?;
			},
			s => anyhow::bail!("level {s} is invalid"),
		}
	}
	Ok(())
}

fn change_log_level(reset: bool, level: &str) -> Response {
	if !reset && level.is_empty() {
		return list_loggers();
	}
	if !level.is_empty()
		&& let Err(_e) = validate_log_level(level)
	{
		// Invalid level provided
		return plaintext_response(
			hyper::StatusCode::BAD_REQUEST,
			format!("Invalid level provided: {level}\n{HELP_STRING}"),
		);
	};
	match telemetry::set_level(reset, level) {
		Ok(_) => list_loggers(),
		Err(e) => plaintext_response(
			hyper::StatusCode::BAD_REQUEST,
			format!("Failed to set new level: {e}\n{HELP_STRING}"),
		),
	}
}

#[cfg(all(feature = "jemalloc", target_os = "linux"))]
#[cfg(all(target_os = "linux"))]
async fn handle_jemalloc_pprof_heapgen(_req: Request<Incoming>) -> anyhow::Result<Response> {
	let Some(prof_ctrl) = jemalloc_pprof::PROF_CTL.as_ref() else {
		return Ok(
			::http::Response::builder()
				.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
				.body("jemalloc profiling is not enabled".into())
				.expect("builder with known status code should not fail"),
		);
	};
	let mut prof_ctl = prof_ctrl.lock().await;
	if !prof_ctl.activated() {
		return Ok(
			::http::Response::builder()
				.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
				.body("jemalloc not enabled".into())
				.expect("builder with known status code should not fail"),
		);
	}
	let pprof = prof_ctl.dump_pprof()?;
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.body(bytes::Bytes::from(pprof).into())
			.expect("builder with known status code should not fail"),
	)
}

#[cfg(not(feature = "jemalloc"))]
async fn handle_jemalloc_pprof_heapgen(_req: Request<Incoming>) -> anyhow::Result<Response> {
	Ok(
		::http::Response::builder()
			.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
			.body("jemalloc not enabled".into())
			.expect("builder with known status code should not fail"),
	)
}

/// Load admin JWT from environment variables (optional).
/// This is intentionally separate from the proxy listener JWT to allow
/// differing audiences/issuer for the admin surface if desired.
pub async fn load_local_admin_jwt() -> Option<crate::http::jwt::Jwt> {
	let enabled = std::env::var("AG_ADMIN_JWT_ENABLED").unwrap_or_default();
	if enabled.is_empty()
		|| !matches!(
			enabled.to_ascii_lowercase().as_str(),
			"1" | "true" | "yes" | "on"
		) {
		return None;
	}
	let issuer = match std::env::var("AG_ADMIN_JWT_ISSUER") {
		Ok(v) if !v.is_empty() => v,
		_ => return None,
	};
	let audiences = std::env::var("AG_ADMIN_JWT_AUDIENCES")
		.unwrap_or_default()
		.split(',')
		.map(|s| s.trim())
		.filter(|s| !s.is_empty())
		.map(|s| s.to_string())
		.collect::<Vec<_>>();
	if audiences.is_empty() {
		return None;
	}
	let mode = match std::env::var("AG_ADMIN_JWT_MODE")
		.unwrap_or_else(|_| "strict".into())
		.to_ascii_lowercase()
		.as_str()
	{
		"strict" => crate::http::jwt::Mode::Strict,
		"optional" => crate::http::jwt::Mode::Optional,
		"permissive" => crate::http::jwt::Mode::Permissive,
		_ => crate::http::jwt::Mode::Strict,
	};
	let jwks_inline = std::env::var("AG_ADMIN_JWT_JWKS_INLINE").ok();
	let jwks_file = std::env::var("AG_ADMIN_JWT_JWKS_FILE").ok();
	let jwks = if let Some(inline) = jwks_inline.filter(|s| !s.is_empty()) {
		crate::serdes::FileInlineOrRemote::Inline(inline.into())
	} else if let Some(file) = jwks_file.filter(|s| !s.is_empty()) {
		crate::serdes::FileInlineOrRemote::File { file: file.into() }
	} else {
		return None;
	};
	let cfg = crate::http::jwt::LocalJwtConfig {
		mode,
		issuer,
		audiences,
		jwks,
	};
	// Construct a basic DNS client config. We cannot use Default on client::Config (not implemented),
	// so mirror the pattern used elsewhere by pulling from hickory_resolver defaults.
	let dns_cfg = crate::client::Config {
		resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
		resolver_opts: hickory_resolver::config::ResolverOpts::default(),
	};
	let client = Client::new(&dns_cfg, None);
	match cfg.try_into(client).await {
		Ok(jwt) => Some(jwt),
		Err(e) => {
			warn!(error=?e, "failed to build admin jwt from env");
			None
		},
	}
}
