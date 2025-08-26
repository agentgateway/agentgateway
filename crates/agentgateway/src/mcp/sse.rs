use std::collections::HashMap;
use std::sync::Arc;

use agent_core::drain::DrainWatcher;
use agent_core::prelude::Strng;
use anyhow::Result;
use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum_core::extract::FromRequest;
use bytes::Bytes;
use futures::stream::Stream;
use futures::{SinkExt, StreamExt};
use http::Method;
use itertools::Itertools;
use rmcp::model::{ClientJsonRpcMessage, GetExtensions};
use rmcp::service::serve_server_with_ct;
use rmcp::transport::common::server_side_http::session_id as generate_streamable_session_id;
use rmcp::transport::sse_server::PostEventQuery;
use rmcp::transport::streamable_http_server::SessionId;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::io::{self};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::cel::ContextBuilder;
use crate::http::jwt::Claims;
use crate::http::*;
use crate::json::from_body;
use crate::mcp::relay;
use crate::mcp::relay::Relay;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::AsyncLog;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::{BackendName, McpAuthentication, McpBackend, McpIDP};
use crate::{ProxyInputs, json};

type SseTxs =
	Arc<std::sync::RwLock<HashMap<SessionId, tokio::sync::mpsc::Sender<ClientJsonRpcMessage>>>>;

#[derive(Debug, Default, Clone)]
pub struct MCPInfo {
	pub tool_call_name: Option<String>,
	pub target_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct App {
	state: Stores,
	metrics: Arc<relay::metrics::Metrics>,
	_drain: DrainWatcher,
	session: Arc<LocalSessionManager>,

	sse_txs: SseTxs,
}

impl App {
	pub fn new(state: Stores, metrics: Arc<relay::metrics::Metrics>, drain: DrainWatcher) -> Self {
		let session: Arc<LocalSessionManager> = Arc::new(Default::default());
		Self {
			state,
			metrics,
			_drain: drain,
			session,
			sse_txs: Default::default(),
		}
	}

	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		name: BackendName,
		backend: McpBackend,
		mut req: Request,
		log: AsyncLog<MCPInfo>,
	) -> Response {
		let (backends, authorization_policies, authn) = {
			let binds = self.state.read_binds();
			let (authorization_policies, authn) = binds.mcp_policies(name.clone());
			let nt = backend
				.targets
				.iter()
				.map(|t| {
					let backend_policies = binds.backend_policies(name.clone(), None);
					Arc::new(McpTarget {
						name: t.name.clone(),
						spec: t.spec.clone(),
						backend_policies,
					})
				})
				.collect_vec();
			(
				McpBackendGroup {
					name: name.clone(),
					targets: nt,
				},
				authorization_policies,
				authn,
			)
		};
		let metrics = self.metrics.clone();
		let sm = self.session.clone();
		let client = PolicyClient { inputs: pi.clone() };

		// Store an empty value, we will populate each field async
		log.store(Some(MCPInfo::default()));
		req.extensions_mut().insert(log);

		// TODO: today we duplicate everything which is error prone. It would be ideal to re-use the parent one
		// The problem is that we decide whether to include various attributes before we pick the backend,
		// so we don't know to register the MCP policies
		let mut ctx = ContextBuilder::new();
		authorization_policies.register(&mut ctx);
		let needs_body = ctx.with_request(&req);
		if needs_body && let Ok(body) = crate::http::inspect_body(req.body_mut()).await {
			ctx.with_request_body(body);
		}
		if let Some(jwt) = req.extensions().get::<Claims>() {
			ctx.with_jwt(jwt);
		}
		ctx.with_source(
			req.extensions().get::<TCPConnectionInfo>().unwrap(),
			req.extensions().get::<TLSConnectionInfo>(),
		);

		// `response` is not valid here, since we run authz first
		// MCP context is added later
		req.extensions_mut().insert(Arc::new(ctx));

		// Check if authentication is required and JWT token is missing
		if let Some(_) = &authn
			&& req.extensions().get::<Claims>().is_none()
			&& !Self::is_well_known_endpoint(req.uri().path())
		{
			return Self::create_auth_required_response(&req).into_response();
		}

		match (req.uri().path(), req.method(), authn) {
			("/sse", m, _) if m == Method::GET => Self::sse_get_handler(
				self.sse_txs.clone(),
				Relay::new(
					pi.clone(),
					backends.clone(),
					metrics.clone(),
					authorization_policies.clone(),
					client.clone(),
					backend.stateful,
				),
			)
			.await
			.into_response(),
			("/sse", m, _) if m == Method::POST => self.sse_post_handler(req).await.into_response(),
			(path, _, Some(auth)) if path.ends_with("client-registration") => self
				.client_registration(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("client_registration error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
			(path, _, Some(auth)) if path.starts_with("/.well-known/oauth-protected-resource") => self
				.protected_resource_metadata(req, auth)
				.await
				.into_response(),
			(path, _, Some(auth)) if path.starts_with("/.well-known/oauth-authorization-server") => self
				.authorization_server_metadata(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("authorization_server_metadata error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
			_ => {
				// Assume this is streamable HTTP otherwise
				let streamable = StreamableHttpService::new(
					move || {
						Ok(Relay::new(
							pi.clone(),
							backends.clone(),
							metrics.clone(),
							authorization_policies.clone(),
							client.clone(),
							backend.stateful,
						))
					},
					sm,
					StreamableHttpServerConfig {
						stateful_mode: backend.stateful,
						..Default::default()
					},
				);
				streamable.handle(req).await.map(axum::body::Body::new)
			},
		}
	}

	fn is_well_known_endpoint(path: &str) -> bool {
		path.starts_with("/.well-known/oauth-protected-resource")
			|| path.starts_with("/.well-known/oauth-authorization-server")
	}
}

#[derive(Debug, Clone)]
pub struct McpBackendGroup {
	pub name: BackendName,
	pub targets: Vec<Arc<McpTarget>>,
}

impl McpBackendGroup {
	pub fn find(&self, name: &str) -> Option<Arc<McpTarget>> {
		self
			.targets
			.iter()
			.find(|target| target.name.as_str() == name)
			.cloned()
	}
}

#[derive(Debug)]
pub struct McpTarget {
	pub name: Strng,
	pub spec: crate::types::agent::McpTargetSpec,
	pub backend_policies: BackendPolicies,
}

impl App {
	fn create_auth_required_response(req: &Request) -> Response {
		let request_path = req.uri().path();
		let proxy_url = Self::get_redirect_url(req, request_path);
		let www_authenticate_value = format!(
			"Bearer resource_metadata=\"{proxy_url}/.well-known/oauth-protected-resource{request_path}\""
		);

		::http::Response::builder()
			.status(StatusCode::UNAUTHORIZED)
			.header("www-authenticate", www_authenticate_value)
			.header("content-type", "application/json")
			.body(axum::body::Body::from(Bytes::from(
				r#"{"error":"unauthorized","error_description":"JWT token required"}"#,
			)))
			.unwrap_or_else(|_| {
				::http::Response::builder()
					.status(StatusCode::INTERNAL_SERVER_ERROR)
					.body(axum::body::Body::empty())
					.unwrap()
			})
	}

	async fn protected_resource_metadata(&self, req: Request, auth: McpAuthentication) -> Response {
		let new_uri = Self::strip_oauth_protected_resource_prefix(&req);

		// Determine the issuer to use - either use the same request URL and path that it was initially with,
		// or else keep the auth.issuer
		let issuer = if auth.provider.is_some() {
			// When a provider is configured, use the same request URL with the well-known prefix stripped
			Self::strip_oauth_protected_resource_prefix(&req)
		} else {
			// No provider configured, use the original issuer
			auth.issuer
		};

		let json_body = auth.resource_metadata.to_rfc_json(new_uri, issuer);

		::http::Response::builder()
			.status(StatusCode::OK)
			.header("content-type", "application/json")
			.header("access-control-allow-origin", "*")
			.header("access-control-allow-methods", "GET, OPTIONS")
			.header("access-control-allow-headers", "content-type")
			.body(axum::body::Body::from(Bytes::from(
				serde_json::to_string(&json_body).unwrap_or_default(),
			)))
			.unwrap_or_else(|_| {
				::http::Response::builder()
					.status(StatusCode::INTERNAL_SERVER_ERROR)
					.body(axum::body::Body::empty())
					.unwrap()
			})
	}

	fn get_redirect_url(req: &Request, strip_base: &str) -> String {
		let uri = req
			.extensions()
			.get::<filters::OriginalUrl>()
			.map(|u| u.0.clone())
			.unwrap_or_else(|| req.uri().clone());

		uri
			.path()
			.strip_suffix(strip_base)
			.map(|p| uri.to_string().replace(uri.path(), p))
			.unwrap_or(uri.to_string())
	}

	fn strip_oauth_protected_resource_prefix(req: &Request) -> String {
		let uri = req
			.extensions()
			.get::<filters::OriginalUrl>()
			.map(|u| u.0.clone())
			.unwrap_or_else(|| req.uri().clone());

		let path = uri.path();
		const OAUTH_PREFIX: &str = "/.well-known/oauth-protected-resource";

		// Remove the oauth-protected-resource prefix and keep the remaining path
		if let Some(remaining_path) = path.strip_prefix(OAUTH_PREFIX) {
			uri.to_string().replace(path, remaining_path)
		} else {
			// If the prefix is not found, return the original URI
			uri.to_string()
		}
	}

	async fn authorization_server_metadata(
		&self,
		req: Request,
		auth: McpAuthentication,
		client: PolicyClient,
	) -> anyhow::Result<Response> {
		let ureq = ::http::Request::builder()
			.uri(format!(
				"{}/.well-known/oauth-authorization-server",
				auth.issuer
			))
			.body(Body::empty())?;
		let upstream = client.simple_call(ureq).await?;
		let mut resp: serde_json::Value = from_body(upstream.into_body()).await?;
		match &auth.provider {
			Some(McpIDP::Auth0 {}) => {
				// Auth0 does not support RFC 8707. We can workaround this by prepending an audience
				let Some(serde_json::Value::String(ae)) =
					json::traverse_mut(&mut resp, &["authorization_endpoint"])
				else {
					anyhow::bail!("authorization_endpoint missing");
				};
				ae.push_str(&format!("?audience={}", auth.audience));
			},
			Some(McpIDP::Keycloak { .. }) => {
				// Keycloak does not support RFC 8707.
				// We do not currently have a workload :-(
				// users will have to hardcode the audience.
				// https://github.com/keycloak/keycloak/issues/10169 and https://github.com/keycloak/keycloak/issues/14355

				// Keycloak doesn't do CORS for client registrations
				// https://github.com/keycloak/keycloak/issues/39629
				// We can workaround this by proxying it

				let current_uri = req
					.extensions()
					.get::<filters::OriginalUrl>()
					.map(|u| u.0.clone())
					.unwrap_or_else(|| req.uri().clone());
				let Some(serde_json::Value::String(re)) =
					json::traverse_mut(&mut resp, &["registration_endpoint"])
				else {
					anyhow::bail!("registration_endpoint missing");
				};
				*re = format!("{current_uri}/client-registration");
			},
			_ => {},
		}

		let response = ::http::Response::builder()
			.status(StatusCode::OK)
			.header("content-type", "application/json")
			.header("access-control-allow-origin", "*")
			.header("access-control-allow-methods", "GET, OPTIONS")
			.header("access-control-allow-headers", "content-type")
			.body(axum::body::Body::from(Bytes::from(serde_json::to_string(
				&resp,
			)?)))
			.map_err(|e| anyhow::anyhow!("Failed to build response: {}", e))?;

		Ok(response)
	}

	async fn client_registration(
		&self,
		req: Request,
		auth: McpAuthentication,
		client: PolicyClient,
	) -> anyhow::Result<Response> {
		let ureq = ::http::Request::builder()
			.uri(format!(
				"{}/clients-registrations/openid-connect",
				auth.issuer
			))
			.method(Method::POST)
			.body(req.into_body())?;

		let mut upstream = client.simple_call(ureq).await?;

		// Add CORS headers to the response
		let headers = upstream.headers_mut();
		headers.insert("access-control-allow-origin", "*".parse().unwrap());
		headers.insert(
			"access-control-allow-methods",
			"POST, OPTIONS".parse().unwrap(),
		);
		headers.insert(
			"access-control-allow-headers",
			"content-type".parse().unwrap(),
		);

		Ok(upstream)
	}

	async fn sse_post_handler(&self, req: Request) -> Result<StatusCode, StatusCode> {
		// Extract query parameters
		let Query(PostEventQuery { session_id }) =
			Query::<PostEventQuery>::try_from_uri(req.uri()).map_err(|_| StatusCode::BAD_REQUEST)?;

		let (part, body) = req.into_parts();
		let parts = part.clone();
		let req = Request::from_parts(part, body);
		let Json(mut message) = Json::<ClientJsonRpcMessage>::from_request(req, &())
			.await
			.map_err(|_| StatusCode::BAD_REQUEST)?;
		if let ClientJsonRpcMessage::Request(req) = &mut message {
			req.request.extensions_mut().insert(parts);
		}
		tracing::info!(session_id, ?message, "new client message for /sse");
		let tx = {
			let rg = self.sse_txs.read().expect("mutex poisoned");
			rg.get(session_id.as_str())
				.ok_or(StatusCode::NOT_FOUND)?
				.clone()
		};

		if tx.send(message).await.is_err() {
			tracing::error!("send message error");
			return Err(StatusCode::GONE);
		}
		Ok(StatusCode::ACCEPTED)
	}

	async fn sse_get_handler(
		sse_txs: SseTxs,
		relay: Relay,
	) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, StatusCode> {
		// it's 4KB

		let session = generate_streamable_session_id();
		tracing::debug!(%session,  "sse connection");

		use tokio_stream::wrappers::ReceiverStream;
		use tokio_util::sync::PollSender;
		let (from_client_tx, from_client_rx) = tokio::sync::mpsc::channel(64);
		let (to_client_tx, to_client_rx) = tokio::sync::mpsc::channel(64);
		sse_txs
			.write()
			.expect("mutex poisoned")
			.insert(session.clone(), from_client_tx);
		{
			let session = session.clone();
			let sse_txs = sse_txs.clone();
			let ct = CancellationToken::new();
			tokio::spawn(async move {
				let stream = ReceiverStream::new(from_client_rx);
				let sink = PollSender::new(to_client_tx.clone()).sink_map_err(std::io::Error::other);
				let result = serve_server_with_ct(relay.clone(), (sink, stream), ct.child_token())
					.await
					.inspect_err(|e| {
						tracing::error!("serving error: {:?}", e);
					});

				if let Err(e) = result {
					tracing::error!(error = ?e, "initialize error");
					sse_txs.write().expect("mutex poisoned").remove(&session);
					return;
				}
				// Add a listener drain channel here.
				tokio::select! {
					_ = to_client_tx.closed() =>{
						tracing::info!("client disconnected");
					}
				};
				sse_txs.write().expect("mutex poisoned").remove(&session);
			});
		}

		let stream = futures::stream::once(futures::future::ok(
			Event::default()
				.event("endpoint")
				.data(format!("?sessionId={session}")),
		))
		.chain(ReceiverStream::new(to_client_rx).map(|message| {
			match serde_json::to_string(&message) {
				Ok(bytes) => Ok(Event::default().event("message").data(&bytes)),
				Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
			}
		}));
		Ok(Sse::new(stream))
	}
}
