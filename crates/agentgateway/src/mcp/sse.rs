use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::ops::IndexMut;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_sdk::SendTaskStreamingResponseResult::Status;
use agent_core::drain::DrainWatcher;
use agent_core::prelude::Strng;
use agent_core::trcng;
use anyhow::Result;
use axum::extract::{ConnectInfo, OptionalFromRequestParts, Query, State};
use axum::http::StatusCode;
use axum::http::header::HeaderMap;
use axum::http::request::Parts;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, RequestPartsExt, Router};
use axum_core::extract::FromRequest;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use axum_extra::typed_header::TypedHeaderRejection;
use futures::stream::Stream;
use futures::{SinkExt, StreamExt};
use http::Method;
use http_body_util::BodyExt;
use itertools::Itertools;
use rmcp::RoleServer;
use rmcp::model::{ClientJsonRpcMessage, GetExtensions};
use rmcp::service::{TxJsonRpcMessage, serve_server_with_ct};
use rmcp::transport::async_rw::JsonRpcMessageCodec;
use rmcp::transport::common::server_side_http::session_id as generate_streamable_session_id;
use rmcp::transport::sse_server::{PostEventQuery, SseServerConfig};
use rmcp::transport::streamable_http_server::SessionId;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{SseServer, StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{Value, json};
use tokio::io::{self};
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tracing::warn;
use url::form_urlencoded;

use crate::cel::ContextBuilder;
use crate::http::jwt::Claims;
use crate::http::*;
use crate::json::{from_body, to_body};
use crate::llm::LLMRequest;
use crate::mcp::rbac::RuleSets;
use crate::mcp::relay::Relay;
use crate::mcp::{rbac, relay};
use crate::proxy::httpproxy::PolicyClient;
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::AsyncLog;
use crate::types::agent::{
	BackendName, McpAuthentication, McpBackend, McpIDP, McpTarget as TypeMcpTarget, McpTargetSpec,
	PolicyTarget, Target,
};
use crate::{ProxyInputs, client, json, mcp};

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
	drain: DrainWatcher,
	session: Arc<LocalSessionManager>,

	sse_txs: SseTxs,
}

impl App {
	pub fn new(state: Stores, metrics: Arc<relay::metrics::Metrics>, drain: DrainWatcher) -> Self {
		let session: Arc<LocalSessionManager> = Arc::new(Default::default());
		Self {
			state,
			metrics,
			drain,
			session,
			sse_txs: Default::default(),
		}
	}

	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		name: BackendName,
		backends: McpBackend,
		mut req: Request,
		log: AsyncLog<MCPInfo>,
	) -> Response {
		let (backends, authorization_policies, authn) = {
			let binds = self.state.read_binds();
			let (authorization_policies, authn) = binds.mcp_policies(name.clone());
			let nt = backends
				.targets
				.iter()
				.map(|t| {
					let backend_policies = binds.backend_policies(PolicyTarget::Backend(name.clone()));
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
		let state = self.state.clone();
		let metrics = self.metrics.clone();
		let sm = self.session.clone();
		let client = PolicyClient { inputs: pi.clone() };

		// Store an empty value, we will populate each field async
		log.store(Some(MCPInfo::default()));
		req.extensions_mut().insert(log);

		let mut ctx = ContextBuilder::new();
		authorization_policies.register(&mut ctx);
		let needs_body = ctx.with_request(&req);
		if needs_body {
			if let Ok(body) = crate::http::inspect_body(req.body_mut()).await {
				ctx.with_request_body(body);
			}
		}
		if let Some(jwt) = req.extensions().get::<Claims>() {
			ctx.with_jwt(jwt);
		}
		// `response` is not valid here, since we run authz first
		// MCP context is added later
		req.extensions_mut().insert(Arc::new(ctx));

		match (req.uri().path(), req.method(), authn) {
			("/sse", m, _) if m == Method::GET => Self::sse_get_handler(
				self.sse_txs.clone(),
				Relay::new(
					pi.clone(),
					backends.clone(),
					metrics.clone(),
					authorization_policies.clone(),
					client.clone(),
				),
			)
			.await
			.into_response(),
			("/sse", m, _) if m == Method::POST => self.sse_post_handler(req).await.into_response(),
			("/.well-known/oauth-protected-resource", _, Some(auth)) => self
				.protected_resource_metadata(req, auth)
				.await
				.into_response(),
			("/.well-known/oauth-authorization-server", _, Some(auth)) => self
				.authorization_server_metadata(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("authorization_server_metadata error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
			("/client-registration", _, Some(auth)) => self
				.client_registration(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("client_registration error: {}", e);
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
						))
					},
					sm,
					StreamableHttpServerConfig {
						..Default::default()
					},
				);
				streamable.handle(req).await.map(axum::body::Body::new)
			},
		}
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
	async fn protected_resource_metadata(
		&self,
		req: Request,
		auth: McpAuthentication,
	) -> Json<Value> {
		let new_uri = Self::get_redirect_url(&req, "/.well-known/oauth-protected-resource");
		// We will unconditionally redirect them back to our own proxy -- not the Authorization Server.
		Json(json!({
			"resource": format!("{}/mcp", new_uri),
			// "authorization_servers": ["https://dev-d1dqcqi4qgzwvabi.us.auth0.com"],
			"authorization_servers": [format!("{new_uri}")],
			"scopes_supported": auth.scopes,
			"bearer_methods_supported": ["header"],
			// "resource_documentation": "http://lo/docs",
			"mcp_protocol_version": "2025-06-18",
			"resource_type": "mcp-server"
		}))
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

	async fn authorization_server_metadata(
		&self,
		req: Request,
		auth: McpAuthentication,
		client: PolicyClient,
	) -> anyhow::Result<Json<Value>> {
		let new_uri = Self::get_redirect_url(&req, "/.well-known/oauth-authorization-server");
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
				let Some(serde_json::Value::String(re)) =
					json::traverse_mut(&mut resp, &["registration_endpoint"])
				else {
					anyhow::bail!("registration_endpoint missing");
				};
				*re = format!("{new_uri}/client-registration");
			},
			_ => {},
		}
		Ok(Json(resp))
	}

	async fn client_registration(
		&self,
		req: Request,
		auth: McpAuthentication,
		client: PolicyClient,
	) -> anyhow::Result<Response> {
		let new_uri = Self::get_redirect_url(&req, "/.well-known/oauth-authorization-server");
		let ureq = ::http::Request::builder()
			.uri(format!(
				"{}/clients-registrations/openid-connect",
				auth.issuer
			))
			.method(Method::POST)
			.body(req.into_body())?;
		let upstream = client.simple_call(ureq).await?;
		Ok(upstream)
	}

	async fn sse_post_handler(&self, req: Request) -> Result<StatusCode, StatusCode> {
		// Extract query parameters
		let uri = req.uri();
		let query = uri.query().unwrap_or("");
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
