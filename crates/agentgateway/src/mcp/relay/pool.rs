use agent_core::prelude::*;
use anyhow::anyhow;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use http::Uri;
use http::header::CONTENT_TYPE;
use reqwest::header::ACCEPT;
use reqwest::{Client as HttpClient, IntoUrl, Url};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::service::{NotificationContext, Peer, serve_client_with_ct, serve_directly_with_ct};
use rmcp::transport::common::client_side_sse::BoxedSseResponse;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::sse_client::{SseClient, SseClientConfig, SseTransportError};
use rmcp::transport::streamable_http_client::{
	StreamableHttpClient, StreamableHttpClientTransportConfig, StreamableHttpError,
	StreamableHttpPostResponse,
};
use rmcp::transport::{SseClientTransport, StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceError};
use sse_stream::{Error as SseError, Sse, SseStream};

use super::*;
use crate::client::Client;
use crate::http::{Body, Error as HttpError, Response, auth};
use crate::mcp::sse::McpTarget;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::{Backend, McpBackend, McpTargetSpec, SimpleBackend, Target};
use crate::{ProxyInputs, client, json};

type McpError = ErrorData;

pub(crate) struct ConnectionPool {
	pi: Arc<ProxyInputs>,
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: HashMap<Strng, upstream::UpstreamTarget>,
}

impl ConnectionPool {
	pub(crate) fn new(pi: Arc<ProxyInputs>, client: PolicyClient, backend: McpBackendGroup) -> Self {
		Self {
			backend,
			client,
			pi,
			by_name: HashMap::new(),
		}
	}

	pub(crate) async fn get(
		&mut self,
		rq_ctx: &RqCtx,
		peer: &Peer<RoleServer>,
		name: &str,
	) -> anyhow::Result<&upstream::UpstreamTarget> {
		// If it doesn't exist, they haven't initialized yet
		if !self.by_name.contains_key(name) {
			return Err(anyhow::anyhow!(
				"requested target {name} is not initialized",
			));
		}
		let target = self.by_name.get(name);
		Ok(target.ok_or(McpError::invalid_request(
			format!("Service {name} not found"),
			None,
		))?)
	}

	pub(crate) async fn remove(&mut self, name: &str) -> Option<upstream::UpstreamTarget> {
		self.by_name.remove(name)
	}

	pub(crate) async fn initialize(
		&mut self,
		rq_ctx: &RqCtx,
		peer: &Peer<RoleServer>,
		request: InitializeRequestParam,
	) -> anyhow::Result<Vec<(Strng, &upstream::UpstreamTarget)>> {
		for tgt in self.backend.targets.clone() {
			if self.by_name.contains_key(&tgt.name) {
				anyhow::bail!("connection {} already initialized", tgt.name);
			}
			let ct = tokio_util::sync::CancellationToken::new(); //TODO
			debug!("initializing target: {}", tgt.name);
			self
				.connect(rq_ctx, &ct, &tgt, peer, request.clone())
				.await
				.map_err(|e| {
					error!("Failed to connect target {}: {}", tgt.name, e);
					e // Propagate error
				})?;
		}
		self.list().await
	}

	pub(crate) async fn list(&mut self) -> anyhow::Result<Vec<(Strng, &upstream::UpstreamTarget)>> {
		let results = self
			.backend
			.targets
			.iter()
			.filter_map(|(tgt)| {
				self
					.by_name
					.get(&tgt.name)
					.map(|target: &upstream::UpstreamTarget| (tgt.name.clone(), target))
			})
			.collect();

		Ok(results)
	}

	#[instrument(
        level = "debug",
        skip_all,
        fields(
        name=%target.name,
        ),
    )]
	pub(crate) async fn connect(
		&mut self,
		rq_ctx: &RqCtx,
		ct: &tokio_util::sync::CancellationToken,
		target: &McpTarget,
		peer: &Peer<RoleServer>,
		init_request: InitializeRequestParam,
	) -> Result<(), anyhow::Error> {
		// Already connected
		if let Some(_transport) = self.by_name.get(&target.name) {
			return Ok(());
		}
		trace!("connecting to target: {}", target.name);
		let transport: upstream::UpstreamTarget = match &target.spec {
			McpTargetSpec::Sse(sse) => {
				debug!("starting sse transport for target: {}", target.name);
				let path = match sse.path.as_str() {
					"" => "/sse",
					_ => sse.path.as_str(),
				};
				let be = crate::proxy::resolve_simple_backend(&sse.backend, &self.pi)?;
				let hostport = be.hostport();
				let client =
					ClientWrapper::new_with_client(be, self.client.clone(), target.backend_policies.clone());
				let transport = SseClientTransport::start_with_client(
					client,
					SseClientConfig {
						sse_endpoint: format!("http://{hostport}{path}").into(),
						..Default::default()
					},
				)
				.await
				.context("start sse client")?;

				upstream::UpstreamTarget {
					spec: upstream::UpstreamTargetSpec::Mcp(
						serve_client_with_ct(
							PeerClientHandler {
								peer: peer.clone(),
								peer_client: None,
								init_request,
							},
							transport,
							ct.child_token(),
						)
						.await?,
					),
				}
			},
			McpTargetSpec::Mcp(mcp) => {
				debug!(
					"starting streamable http transport for target: {}",
					target.name
				);
				let path = match mcp.path.as_str() {
					"" => "/mcp",
					_ => mcp.path.as_str(),
				};
				let be = crate::proxy::resolve_simple_backend(&mcp.backend, &self.pi)?;
				let client =
					ClientWrapper::new_with_client(be, self.client.clone(), target.backend_policies.clone());
				let mut transport = StreamableHttpClientTransport::with_client(
					client,
					StreamableHttpClientTransportConfig {
						uri: path.into(),
						..Default::default()
					},
				);

				upstream::UpstreamTarget {
					spec: upstream::UpstreamTargetSpec::Mcp(
						serve_client_with_ct(
							PeerClientHandler {
								peer: peer.clone(),
								peer_client: None,
								init_request,
							},
							transport,
							ct.child_token(),
						)
						.await?,
					),
				}
			},
			McpTargetSpec::Stdio { cmd, args, env } => {
				debug!("starting stdio transport for target: {}", target.name);
				let mut c = Command::new(cmd);
				c.args(args);
				for (k, v) in env {
					c.env(k, v);
				}
				upstream::UpstreamTarget {
					spec: upstream::UpstreamTargetSpec::Mcp(
						serve_client_with_ct(
							PeerClientHandler {
								peer: peer.clone(),
								peer_client: None,
								init_request,
							},
							TokioChildProcess::new(c).context(format!("failed to run command '{cmd}'"))?,
							ct.child_token(),
						)
						.await?,
					),
				}
			},
			McpTargetSpec::OpenAPI(open) => {
				// Renamed for clarity
				debug!("starting OpenAPI transport for target: {}", target.name);

				let tools = crate::mcp::openapi::parse_openapi_schema(&open.schema).map_err(|e| {
					anyhow::anyhow!(
						"Failed to parse tools from OpenAPI schema for target {}: {}",
						target.name,
						e
					)
				})?;

				let prefix = crate::mcp::openapi::get_server_prefix(&open.schema).map_err(|e| {
					anyhow::anyhow!(
						"Failed to get server prefix from OpenAPI schema for target {}: {}",
						target.name,
						e
					)
				})?;
				let be = crate::proxy::resolve_simple_backend(&open.backend, &self.pi)?;
				upstream::UpstreamTarget {
					spec: upstream::UpstreamTargetSpec::OpenAPI(Box::new(crate::mcp::openapi::Handler {
						backend: be,
						client: self.client.clone(),
						default_policies: target.backend_policies.clone(),
						tools,  // From parse_openapi_schema
						prefix, // From get_server_prefix
					})),
				}
			},
		};
		self.by_name.insert(target.name.clone(), transport);
		Ok(())
	}
}

#[derive(Debug, Clone)]
pub(crate) struct PeerClientHandler {
	peer: Peer<RoleServer>,
	peer_client: Option<Peer<RoleClient>>,
	init_request: InitializeRequestParam,
}

impl ClientHandler for PeerClientHandler {
	async fn create_message(
		&self,
		params: CreateMessageRequestParam,
		_context: RequestContext<RoleClient>,
	) -> Result<CreateMessageResult, McpError> {
		self.peer.create_message(params).await.map_err(|e| match e {
			ServiceError::McpError(e) => e,
			_ => McpError::internal_error(e.to_string(), None),
		})
	}

	async fn list_roots(
		&self,
		_context: RequestContext<RoleClient>,
	) -> Result<ListRootsResult, McpError> {
		self.peer.list_roots().await.map_err(|e| match e {
			ServiceError::McpError(e) => e,
			_ => McpError::internal_error(e.to_string(), None),
		})
	}

	async fn on_cancelled(
		&self,
		params: CancelledNotificationParam,
		_context: NotificationContext<RoleClient>,
	) {
		let _ = self.peer.notify_cancelled(params).await.inspect_err(|e| {
			error!("Failed to notify cancelled: {}", e);
		});
	}

	async fn on_progress(
		&self,
		params: ProgressNotificationParam,
		_context: NotificationContext<RoleClient>,
	) {
		let _ = self.peer.notify_progress(params).await.inspect_err(|e| {
			error!("Failed to notify progress: {}", e);
		});
	}

	async fn on_logging_message(
		&self,
		params: LoggingMessageNotificationParam,
		_context: NotificationContext<RoleClient>,
	) {
		let _ = self
			.peer
			.notify_logging_message(params)
			.await
			.inspect_err(|e| {
				error!("Failed to notify logging message: {}", e);
			});
	}

	async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_prompt_list_changed()
			.await
			.inspect_err(|e| {
				error!("Failed to notify prompt list changed: {}", e);
			});
	}

	async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_resource_list_changed()
			.await
			.inspect_err(|e| {
				error!("Failed to notify resource list changed: {}", e);
			});
	}

	async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self.peer.notify_tool_list_changed().await.inspect_err(|e| {
			error!("Failed to notify tool list changed: {}", e);
		});
	}

	async fn on_resource_updated(
		&self,
		params: ResourceUpdatedNotificationParam,
		_context: NotificationContext<RoleClient>,
	) {
		let _ = self
			.peer
			.notify_resource_updated(params)
			.await
			.inspect_err(|e| {
				error!("Failed to notify resource updated: {}", e);
			});
	}

	fn get_info(&self) -> ClientInfo {
		self.init_request.get_info()
	}
}

#[derive(Clone)]
pub struct ClientWrapper {
	backend: Arc<SimpleBackend>,
	client: PolicyClient,
	policies: BackendPolicies,
}

impl ClientWrapper {
	pub fn new_with_client(
		backend: SimpleBackend,
		client: PolicyClient,
		policies: BackendPolicies,
	) -> Self {
		Self {
			backend: Arc::new(backend),
			client,
			policies,
		}
	}

	fn parse_uri(
		uri: Arc<str>,
	) -> Result<String, StreamableHttpError<<ClientWrapper as StreamableHttpClient>::Error>> {
		uri
			.parse::<Uri>()
			.map(|u| {
				u.path_and_query()
					.map(|p| p.as_str().to_string())
					.unwrap_or_default()
			})
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))
	}
}

impl StreamableHttpClient for ClientWrapper {
	type Error = HttpError;

	async fn post_message(
		&self,
		uri: Arc<str>,
		message: ClientJsonRpcMessage,
		session_id: Option<Arc<str>>,
		auth_header: Option<String>,
	) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
		let client = self.client.clone();

		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;

		let body =
			serde_json::to_vec(&message).map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		let mut req = http::Request::builder()
			.uri(uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
			.body(body.into())
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		if let Some(session_id) = session_id {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id
					.as_ref()
					.parse()
					.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?,
			);
		}

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
				"received status code {}",
				resp.status()
			))));
		}

		let content_type = resp.headers().get(CONTENT_TYPE);
		let session_id = resp
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
			.map(|s| s.to_string());

		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, session_id))
			},
			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
				let message = json::from_body::<ServerJsonRpcMessage>(resp.into_body())
					.await
					.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
				Ok(StreamableHttpPostResponse::Json(message, session_id))
			},
			_ => {
				tracing::error!("unexpected content type: {:?}", content_type);
				Err(StreamableHttpError::UnexpectedContentType(
					content_type.map(|ct| String::from_utf8_lossy(ct.as_bytes()).to_string()),
				))
			},
		}
	}

	async fn delete_session(
		&self,
		uri: Arc<str>,
		session_id: Arc<str>,
		auth_header: Option<String>,
	) -> Result<(), StreamableHttpError<Self::Error>> {
		let client = self.client.clone();

		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;

		let mut req = http::Request::builder()
			.uri(uri)
			.method(http::Method::DELETE)
			.header(HEADER_SESSION_ID, session_id.as_ref())
			.body(Body::empty())
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		// If method not allowed, that's ok
		if resp.status() == http::StatusCode::METHOD_NOT_ALLOWED {
			tracing::debug!("this server doesn't support deleting session");
			return Ok(());
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
				"received status code {}",
				resp.status()
			))));
		}

		Ok(())
	}

	async fn get_stream(
		&self,
		uri: Arc<str>,
		session_id: Arc<str>,
		last_event_id: Option<String>,
		auth_header: Option<String>,
	) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
		let client = self.client.clone();

		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;

		let mut reqb = http::Request::builder()
			.uri(uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.header(HEADER_SESSION_ID, session_id.as_ref());

		if let Some(last_event_id) = last_event_id {
			reqb = reqb.header(HEADER_LAST_EVENT_ID, last_event_id);
		}

		let mut req = reqb
			.body(Body::empty())
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;

		if resp.status() == http::StatusCode::METHOD_NOT_ALLOWED {
			return Err(StreamableHttpError::SeverDoesNotSupportSse);
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
				"received status code {}",
				resp.status()
			))));
		}

		match resp.headers().get(CONTENT_TYPE) {
			Some(ct) => {
				if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
					return Err(StreamableHttpError::UnexpectedContentType(Some(
						String::from_utf8_lossy(ct.as_bytes()).to_string(),
					)));
				}
			},
			None => {
				return Err(StreamableHttpError::UnexpectedContentType(None));
			},
		}

		let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
		Ok(event_stream)
	}
}

impl SseClient for ClientWrapper {
	type Error = HttpError;

	async fn post_message(
		&self,
		uri: Uri,
		message: ClientJsonRpcMessage,
		auth_token: Option<String>,
	) -> Result<(), SseTransportError<Self::Error>> {
		let uri = "http://".to_string()
			+ &self.backend.hostport()
			+ uri.path_and_query().map(|p| p.as_str()).unwrap_or_default();
		let body =
			serde_json::to_vec(&message).map_err(|e| SseTransportError::Client(HttpError::new(e)))?;
		let mut req = http::Request::builder()
			.uri(uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
			.body(body.into())
			.map_err(|e| SseTransportError::Client(HttpError::new(e)))?;

		if let JsonRpcMessage::Request(request) = &message {
			match request.request.extensions().get::<RqCtx>() {
				Some(rq_ctx) => {
					let tracer = trcng::get_tracer();
					let _span = tracer
						.span_builder("sse_post")
						.with_kind(SpanKind::Client)
						.start_with_context(tracer, &rq_ctx.context);
					trcng::add_context_to_request(req.headers_mut(), &rq_ctx.context);
				},
				None => {
					trace!("No RqCtx found in extensions");
				},
			}
		}

		self
			.client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(|e| SseTransportError::Client(HttpError::new(e)))
			.and_then(|resp| {
				if resp.status().is_client_error() || resp.status().is_server_error() {
					Err(SseTransportError::Client(HttpError::new(anyhow!(
						"received status code {}",
						resp.status()
					))))
				} else {
					Ok(resp)
				}
			})
			.map(drop)
	}

	fn get_stream(
		&self,
		uri: Uri,
		last_event_id: Option<String>,
		auth_token: Option<String>,
	) -> impl Future<Output = Result<BoxedSseResponse, SseTransportError<Self::Error>>> + Send + '_ {
		Box::pin(async move {
			let uri = "http://".to_string()
				+ &self.backend.hostport()
				+ uri.path_and_query().map(|p| p.as_str()).unwrap_or_default();

			let mut reqb = http::Request::builder()
				.uri(uri)
				.method(http::Method::GET)
				.header(ACCEPT, EVENT_STREAM_MIME_TYPE);
			if let Some(last_event_id) = last_event_id {
				reqb = reqb.header(HEADER_LAST_EVENT_ID, last_event_id);
			}
			let req = reqb
				.body(Body::empty())
				.map_err(|e| SseTransportError::Client(HttpError::new(e)))?;

			let resp: Result<Response, ProxyError> = self
				.client
				.call_with_default_policies(req, &self.backend, self.policies.clone())
				.await;

			let resp = resp
				.map_err(|e| SseTransportError::Client(HttpError::new(e)))
				.and_then(|resp| {
					if resp.status().is_client_error() || resp.status().is_server_error() {
						Err(SseTransportError::Client(HttpError::new(anyhow!(
							"received status code {}",
							resp.status()
						))))
					} else {
						Ok(resp)
					}
				})?;
			match resp.headers().get(CONTENT_TYPE) {
				Some(ct) => {
					if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
						return Err(SseTransportError::UnexpectedContentType(Some(ct.clone())));
					}
				},
				None => {
					return Err(SseTransportError::UnexpectedContentType(None));
				},
			}

			let event_stream =
				sse_stream::SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
			Ok(event_stream)
		})
	}
}
