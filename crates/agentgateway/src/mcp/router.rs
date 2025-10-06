use std::sync::Arc;

use agent_core::prelude::Strng;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::Method;
use itertools::Itertools;
use rmcp::transport::StreamableHttpServerConfig;
use tracing::warn;

use crate::cel::ContextBuilder;
use crate::http::jwt::Claims;
use crate::http::*;
use crate::json::from_body_with_limit;
use crate::mcp::MCPInfo;
use crate::mcp::handler::Relay;
use crate::mcp::session::SessionManager;
use crate::mcp::sse::LegacySSEService;
use crate::mcp::streamablehttp::StreamableHttpService;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::AsyncLog;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::{
	BackendName, McpAuthentication, McpBackend, McpIDP, McpTargetSpec, SimpleBackendReference,
};
use crate::{ProxyInputs, json};

#[derive(Debug, Clone)]
pub struct App {
	state: Stores,
	session: Arc<SessionManager>,
}

impl App {
	pub fn new(state: Stores) -> Self {
		let session: Arc<SessionManager> = Arc::new(Default::default());
		Self { state, session }
	}

	pub fn should_passthrough(
		&self,
		name: BackendName,
		backend: &McpBackend,
		req: &Request,
	) -> Option<SimpleBackendReference> {
		if backend.targets.len() != 1 {
			return None;
		}

		let binds = self.state.read_binds();
		let (_, authn) = binds.mcp_policies(name.clone());
		if authn.is_some() {
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
	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		name: BackendName,
		backend: McpBackend,
		mut req: Request,
		log: AsyncLog<MCPInfo>,
		start_time: String,
	) -> Response {
		let (backends, authorization_policies, authn) = {
			let binds = self.state.read_binds();
			let (authorization_policies, authn) = binds.mcp_policies(name.clone());
			let nt = backend
				.targets
				.iter()
				.map(|t| {
					let backend_policies = binds.backend_policies(name.clone(), None, Some(t.name.clone()));
					Arc::new(McpTarget {
						name: t.name.clone(),
						spec: t.spec.clone(),
						backend_policies,
					})
				})
				.collect_vec();
			(
				McpBackendGroup { targets: nt },
				authorization_policies,
				authn,
			)
		};
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
		let needs_body = ctx.with_request(&req, start_time);
		if needs_body && let Ok(body) = crate::http::inspect_body(&mut req).await {
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
			("/sse", _, _) => {
				// Assume this is streamable HTTP otherwise
				let sse = LegacySSEService::new(
					move || {
						Relay::new(
							pi.clone(),
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
						Relay::new(
							pi.clone(),
							backends.clone(),
							authorization_policies.clone(),
							client.clone(),
						)
						.map_err(|e| Error::new(e.to_string()))
					},
					sm,
					StreamableHttpServerConfig {
						stateful_mode: backend.stateful,
						..Default::default()
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
		let limit = crate::http::response_buffer_limit(&upstream);
		let mut resp: serde_json::Value = from_body_with_limit(upstream.into_body(), limit).await?;
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
}
