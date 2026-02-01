use std::sync::Arc;

use agent_core::prelude::Strng;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::Method;
use http::uri::PathAndQuery;
use tracing::{debug, warn};

use crate::cel::ContextBuilder;
use crate::http::authorization::RuleSets;
use crate::http::jwt::Claims;
use crate::http::sessionpersistence::Encoder;
use crate::http::*;
use crate::json::from_body_with_limit;
use crate::mcp::handler::Relay;
use crate::mcp::session::SessionManager;
use crate::mcp::sse::LegacySSEService;
use crate::mcp::streamablehttp::{GatewayStreamableHttpServerConfig, StreamableHttpService};
use crate::mcp::{MCPInfo, McpAuthorizationSet};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::AsyncLog;
use crate::types::agent::{
	BackendTargetRef, McpAuthentication, McpBackend, McpIDP, McpTargetSpec, ResourceName,
	SimpleBackend, SimpleBackendReference,
};
use crate::{ProxyInputs, json};

#[derive(Debug, Clone)]
pub struct App {
	state: Stores,
	session: Arc<SessionManager>,
}

impl App {
	pub fn new(state: Stores, encoder: Encoder) -> Self {
		let session: Arc<SessionManager> = Arc::new(crate::mcp::session::SessionManager::new(encoder));
		Self { state, session }
	}

	pub fn should_passthrough(
		&self,
		backend_policies: &BackendPolicies,
		backend: &McpBackend,
		req: &Request,
	) -> Option<SimpleBackendReference> {
		if backend.targets.len() != 1 {
			return None;
		}

		if backend_policies.mcp_authentication.is_some() {
			return None;
		}
		if !req.uri().path().contains("/.well-known/") {
			return None;
		}
		match backend.targets.first().map(|t| &t.spec) {
			Some(McpTargetSpec::Mcp(s)) => Some(s.backend.clone()),
			Some(McpTargetSpec::Sse(s)) => Some(s.backend.clone()),
			_ => None,
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		backend_group_name: ResourceName,
		backend: McpBackend,
		backend_policies: BackendPolicies,
		mut req: Request,
		log: AsyncLog<MCPInfo>,
	) -> Result<Response, ProxyError> {
		let backends = {
			let binds = self.state.read_binds();
			let nt = backend
				.targets
				.iter()
				.map(|t| {
					let be = t
						.spec
						.backend()
						.map(|b| crate::proxy::resolve_simple_backend_with_policies(b, &pi))
						.transpose()?;
					let inline_pols = be.as_ref().map(|pol| pol.inline_policies.as_slice());
					let sub_backend_target = BackendTargetRef::Backend {
						name: backend_group_name.name.as_ref(),
						namespace: backend_group_name.namespace.as_ref(),
						section: Some(t.name.as_ref()),
					};
					let backend_policies = backend_policies
						.clone()
						.merge(binds.sub_backend_policies(sub_backend_target, inline_pols));
					Ok::<_, ProxyError>(Arc::new(McpTarget {
						name: t.name.clone(),
						spec: t.spec.clone(),
						backend: be.map(|b| b.backend),
						backend_policies,
						always_use_prefix: backend.always_use_prefix,
					}))
				})
				.collect::<Result<Vec<_>, _>>()?;

			McpBackendGroup {
				targets: nt,
				stateful: backend.stateful,
			}
		};
		let sm = self.session.clone();
		let client = PolicyClient { inputs: pi.clone() };
		let authorization_policies = backend_policies
			.mcp_authorization
			.unwrap_or_else(|| McpAuthorizationSet::new(RuleSets::from(Vec::new())));
		let authn = backend_policies.mcp_authentication;

		// Store an empty value, we will populate each field async
		log.store(Some(MCPInfo::default()));
		req.extensions_mut().insert(log);

		// TODO: today we duplicate everything which is error prone. It would be ideal to re-use the parent one
		// The problem is that we decide whether to include various attributes before we pick the backend,
		// so we don't know to register the MCP policies
		let mut ctx = ContextBuilder::new();
		authorization_policies.register(&mut ctx);
		ctx.maybe_buffer_request_body(&mut req).await;

		// `response` is not valid here, since we run authz first
		// MCP context is added later. The context is inserted after
		// authentication so it can include verified claims

		// skip well-known OAuth endpoints for authn
		if !Self::is_well_known_endpoint(req.uri().path()) {
			let has_claims = req.extensions().get::<Claims>().is_some();

			match (authn.as_ref(), has_claims) {
				// if mcp authn is configured, has a validator, and has no claims yet, validate
				(Some(auth), false) => {
					debug!(
						"MCP auth configured; validating Authorization header (mode={:?})",
						auth.mode
					);
					auth
						.jwt_validator
						.apply(None, &mut req)
						.await
						.map_err(|e| {
							Self::create_auth_required_response(
								ProxyError::JwtAuthenticationFailure(e),
								&req,
								auth,
							)
						})?;
				},
				// if mcp authn is configured but JWT already validated (claims exist from previous layer),
				// reject because we cannot validate MCP-specific auth requirements
				(Some(auth), true) => {
					return Err(Self::create_auth_required_response(
						ProxyError::ProcessingString("MCP backend authentication configured but JWT token already validated and stripped by Gateway or Route level policy".to_string()),
						&req,
						auth
					));
				},
				// if no mcp authn is configured, do nothing
				(None, _) => {
					debug!(
						"No MCP authentication configured for backend; continuing without JWT enforcement"
					);
				},
			}
		}

		// Insert the finalized context (now potentially including verified JWT claims)
		req.extensions_mut().insert(Arc::new(ctx));

		match (req.uri().path(), req.method(), authn) {
			("/sse", _, _) => {
				// Assume this is streamable HTTP otherwise
				let sse = LegacySSEService::new(
					move || {
						Relay::new(
							backends.clone(),
							authorization_policies.clone(),
							client.clone(),
						)
						.map_err(|e| Error::new(e.to_string()))
					},
					sm,
				);
				sse.handle(req).await
			},
			// TODO: indicate this is a DirectResponse
			(path, _, Some(auth)) if path.ends_with("client-registration") => Ok(
				self
					.client_registration(req, auth, client.clone())
					.await
					.map_err(|e| {
						warn!("client_registration error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response(),
			),
			(path, _, Some(auth)) if path.starts_with("/.well-known/oauth-protected-resource") => Ok(
				self
					.protected_resource_metadata(req, auth)
					.await
					.into_response(),
			),
			(path, _, Some(auth)) if path.starts_with("/.well-known/oauth-authorization-server") => Ok(
				self
					.authorization_server_metadata(req, auth, client.clone())
					.await
					.map_err(|e| {
						warn!("authorization_server_metadata error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response(),
			),
			_ => {
				// Assume this is streamable HTTP otherwise
				let streamable = StreamableHttpService::new(
					move || {
						Relay::new(
							backends.clone(),
							authorization_policies.clone(),
							client.clone(),
						)
						.map_err(|e| Error::new(e.to_string()))
					},
					sm,
					GatewayStreamableHttpServerConfig {
						stateful_mode: backend.stateful,
					},
				);
				streamable.handle(req).await
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
	pub targets: Vec<Arc<McpTarget>>,
	pub stateful: bool,
}

#[derive(Debug)]
pub struct McpTarget {
	pub name: Strng,
	pub spec: crate::types::agent::McpTargetSpec,
	pub backend_policies: BackendPolicies,
	pub backend: Option<SimpleBackend>,
	pub always_use_prefix: bool,
}

impl App {
	fn create_auth_required_response(
		inner: ProxyError,
		req: &Request,
		auth: &McpAuthentication,
	) -> ProxyError {
		let request_path = req.uri().path();
		// If the `resource` is explicitly configured, use that as the base. otherwise, derive it from the
		// the request URL
		let proxy_url = auth
			.resource_metadata
			.extra
			.get("resource")
			.and_then(|v| v.as_str())
			.and_then(|u| http::uri::Uri::try_from(u).ok())
			.and_then(|uri| {
				let mut parts = uri.into_parts();
				parts.path_and_query = Some(PathAndQuery::from_static(""));
				Uri::from_parts(parts).ok()
			})
			.and_then(|uri| uri.to_string().strip_suffix("/").map(ToString::to_string))
			.unwrap_or_else(|| Self::get_redirect_url(req, request_path));
		let www_authenticate_value = format!(
			"Bearer resource_metadata=\"{proxy_url}/.well-known/oauth-protected-resource{request_path}\""
		);

		ProxyError::McpJwtAuthenticationFailure(Box::new(inner), www_authenticate_value)
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
	) -> Result<Response, ProxyError> {
		// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
		let issuer = auth.issuer.trim_end_matches('/');
		let ureq = ::http::Request::builder()
			.uri(format!("{issuer}/.well-known/oauth-authorization-server"))
			.body(Body::empty())?;
		let upstream = client.simple_call(ureq).await?;
		let limit = crate::http::response_buffer_limit(&upstream);
		let mut resp: serde_json::Value = from_body_with_limit(upstream.into_body(), limit)
			.await
			.map_err(ProxyError::Body)?;
		match &auth.provider {
			Some(McpIDP::Auth0 {}) => {
				// Auth0 does not support RFC 8707. We can workaround this by prepending an audience
				let Some(serde_json::Value::String(ae)) =
					json::traverse_mut(&mut resp, &["authorization_endpoint"])
				else {
					return Err(ProxyError::ProcessingString(
						"authorization_endpoint missing".to_string(),
					));
				};
				// If the user provided multiple audiences with auth0, just prepend the first one
				if let Some(aud) = auth.audiences.first() {
					ae.push_str(&format!("?audience={}", aud));
				}
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
					return Err(ProxyError::ProcessingString(
						"registration_endpoint missing".to_string(),
					));
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
			.body(axum::body::Body::from(Bytes::from(
				serde_json::to_string(&resp).map_err(|e| ProxyError::Body(crate::http::Error::new(e)))?,
			)))?;

		Ok(response)
	}

	async fn client_registration(
		&self,
		req: Request,
		auth: McpAuthentication,
		client: PolicyClient,
	) -> Result<Response, ProxyError> {
		// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
		let issuer = auth.issuer.trim_end_matches('/');
		let ureq = ::http::Request::builder()
			.uri(format!("{issuer}/clients-registrations/openid-connect"))
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
}
