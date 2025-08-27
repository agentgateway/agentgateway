use super::*;
use crate::http::{Body, Error as HttpError, Response};
use crate::mcp::sse::McpTarget;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::{McpTargetSpec, SimpleBackend};
use crate::{ProxyInputs, json};
use agent_core::prelude::*;
use anyhow::anyhow;
use arc_swap::ArcSwapOption;
use axum_core::BoxError;
use frozen_collections::MapIteration;
use futures::StreamExt;
use futures::stream::BoxStream;
use futures_core::Stream;
use futures_util::SinkExt;
use http::Uri;
use http::header::CONTENT_TYPE;
use reqwest::header::ACCEPT;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::service::{
	AtomicU32Provider, NotificationContext, Peer, RequestIdProvider, serve_client_with_ct,
};
use rmcp::transport::common::client_side_sse::BoxedSseResponse;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::sse_client::{SseClient, SseClientConfig, SseTransportError};
use rmcp::transport::streamable_http_client::{
	StreamableHttpClient, StreamableHttpClientTransportConfig, StreamableHttpError,
	StreamableHttpPostResponse,
};
use rmcp::transport::{SseClientTransport, StreamableHttpClientTransport};
use rmcp::{ClientHandler, ServiceError};
use sse_stream::{Error as SseError, Sse, SseStream};
use thiserror::Error;
use tracing_subscriber::filter::FilterExt;

type McpError = ErrorData;

#[derive(Debug)]
pub(crate) struct ConnectionPool {
	pi: Arc<ProxyInputs>,
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: HashMap<Strng, Arc<upstream::Upstream>>,
	stateful: bool,
}

impl ConnectionPool {
	pub(crate) fn new(
		pi: Arc<ProxyInputs>,
		client: PolicyClient,
		backend: McpBackendGroup,
		stateful: bool,
	) -> anyhow::Result<Self> {
		let mut s = Self {
			backend,
			client,
			pi,
			by_name: HashMap::new(),
			stateful,
		};
		s.setup_connections()?;
		Ok(s)
	}

	pub(crate) fn setup_connections(&mut self) -> anyhow::Result<()> {
		for tgt in &self.backend.targets {
			let ct = tokio_util::sync::CancellationToken::new(); //TODO
			debug!("initializing target: {}", tgt.name);
			let transport = self.setup_upstream(tgt.as_ref())?;
			self.by_name.insert(tgt.name.clone(), Arc::new(transport));
		}
		Ok(())
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = Arc<upstream::Upstream>> {
		self.by_name.values().cloned()
	}
	pub(crate) fn iter_named(&self) -> impl Iterator<Item = (Strng, Arc<upstream::Upstream>)> {
		self.by_name.iter().map(|(k, v)| (k.clone(), v.clone()))
	}
	pub(crate) fn get(&self, name: &str) -> anyhow::Result<&upstream::Upstream> {
		self
			.by_name
			.get(name)
			.map(|v| v.as_ref())
			.ok_or_else(|| anyhow::anyhow!("requested target {name} is not initialized",))
	}

	fn setup_upstream(&self, target: &McpTarget) -> Result<upstream::Upstream, anyhow::Error> {
		trace!("connecting to target: {}", target.name);
		let target = match &target.spec {
			McpTargetSpec::Sse(sse) => {
				todo!()
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
				let client = ClientWrapper::new_with_client(
					be,
					path.into(),
					self.client.clone(),
					target.backend_policies.clone(),
				);

				// client
				// 	.send_message(ClientRequest::InitializeRequest(InitializeRequest::new(init_request)))
				// 	.await?;

				upstream::Upstream::McpHttp(client)
			},
			McpTargetSpec::Stdio { cmd, args, env } => {
				todo!()
				// debug!("starting stdio transport for target: {}", target.name);
				// #[cfg(target_os = "windows")]
				// // Command has some weird behavior on Windows where it expects the executable extension to be
				// // .exe. The which create will resolve the actual command for us.
				// // See https://github.com/rust-lang/rust/issues/37519#issuecomment-1694507663
				// // for more context.
				// let cmd = which::which(cmd)?;
				// #[cfg(target_family = "unix")]
				// let mut c = Command::new(cmd);
				// #[cfg(target_os = "windows")]
				// let mut c = Command::new(&cmd);
				// c.args(args);
				// for (k, v) in env {
				// 	c.env(k, v);
				// }
				// upstream::Upstream {
				// 	spec: upstream::UpstreamTargetSpec::Mcp(
				// 		serve_client_with_ct(
				// 			PeerClientHandler {
				// 				peer: peer.clone(),
				// 				init_request,
				// 			},
				// 			TokioChildProcess::new(c).context(format!("failed to run command '{:?}'", &cmd))?,
				// 			ct.child_token(),
				// 		)
				// 		.await?,
				// 	),
				// }
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
				upstream::Upstream::OpenAPI(Box::new(crate::mcp::openapi::Handler {
					backend: be,
					client: self.client.clone(),
					default_policies: target.backend_policies.clone(),
					tools,  // From parse_openapi_schema
					prefix, // From get_server_prefix
				}))
			},
		};

		Ok(target)
	}
}

#[derive(Debug, Clone)]
pub(crate) struct PeerClientHandler {
	peer: Peer<RoleServer>,
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

#[derive(Clone, Debug)]
pub struct ClientWrapper {
	backend: Arc<SimpleBackend>,
	uri: Uri,
	idp: Arc<AtomicU32Provider>,
	client: PolicyClient,
	policies: BackendPolicies,
	headers: http::HeaderMap,
	session_id: AtomicOption<String>,
}

impl ClientWrapper {
	pub fn insert_headers(&self, req: &mut crate::http::Request) {
		for (k, v) in &self.headers {
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}
	}
}

#[derive(Error, Debug)]
pub enum ClientError {
	#[error("http request failed with code: {0}")]
	Status(http::StatusCode),
	#[error("http request failed: {0}")]
	General(Arc<HttpError>),
}

impl ClientError {
	pub fn new(error: impl Into<BoxError>) -> Self {
		Self::General(Arc::new(HttpError::new(error.into())))
	}
}

impl ClientWrapper {
	pub fn new_with_client(
		backend: SimpleBackend,
		path: Strng,
		client: PolicyClient,
		policies: BackendPolicies,
		headers: HeaderMap,
	) -> Self {
		let hp = backend.hostport();
		Self {
			backend: Arc::new(backend),
			uri: ("http://".to_string() + &hp + path.as_str())
				.parse()
				.expect("TODO"),
			idp: Arc::new(AtomicU32Provider::default()),
			client,
			policies,
			headers,
			session_id: Default::default(),
		}
	}
	pub fn set_session_id(&self, s: String) {
		self.session_id.store(Some(Arc::new(s)));
	}
	pub async fn expect_accepted(res: StreamableHttpPostResponse) -> Result<(), ClientError> {
		match res {
			StreamableHttpPostResponse::Accepted => Ok(()),
			StreamableHttpPostResponse::Json(_, _) => {
				Err(ClientError::new(anyhow!("unexpected 'json' response")).into())
			},
			StreamableHttpPostResponse::Sse(_, _) => {
				Err(ClientError::new(anyhow!("unexpected 'sse' response")).into())
			},
		}
	}
	pub async fn expect_single_response(
		res: StreamableHttpPostResponse,
	) -> Result<(ServerResult, Option<String>), ClientError> {
		match res {
			StreamableHttpPostResponse::Accepted => {
				Err(ClientError::new(anyhow!("unexpected 'accepted' response")).into())
			},
			StreamableHttpPostResponse::Json(r, sid) => r
				.into_response()
				.ok_or_else(|| ClientError::new(anyhow!("unexpected 'json' response")))
				.map(|t| (t.0, sid)),
			StreamableHttpPostResponse::Sse(mut sse, sid) => {
				loop {
					// Look for the first item
					let Some(item) = tokio_stream::StreamExt::next(&mut sse).await else {
						return Err(ClientError::new(anyhow!("no response on SSE stream")));
					};
					let item = item.map_err(ClientError::new)?;
					let Some(data) = item.data else { continue };
					let rpc =
						serde_json::from_str::<ServerJsonRpcMessage>(&data).map_err(ClientError::new)?;
					return rpc
						.into_response()
						.ok_or_else(|| ClientError::new(anyhow!("unexpected 'sse' response")))
						.map(|t| (t.0, sid));
				}
			},
		}
	}

	pub async fn expect_stream(
		res: StreamableHttpPostResponse,
	) -> Result<BoxStream<'static, Result<ServerJsonRpcMessage, ClientError>>, ClientError> {
		match res {
			StreamableHttpPostResponse::Accepted => {
				Err(ClientError::new(anyhow!("unexpected 'accepted' response")).into())
			},
			StreamableHttpPostResponse::Json(r, sid) => {
				Ok(futures::stream::once(async { Ok(r) }).boxed())
			},
			StreamableHttpPostResponse::Sse(mut sse, sid) => Ok(
				sse
					.filter_map(|item| async {
						item
							.map_err(ClientError::new)
							.and_then(|item| {
								item
									.data
									.map(|data| {
										serde_json::from_str::<ServerJsonRpcMessage>(&data).map_err(ClientError::new)
									})
									.transpose()
							})
							.transpose()
					})
					.boxed(),
			),
		}
	}

	pub async fn send_message(
		&self,
		req: ClientRequest,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::request(req, self.idp.next_request_id());
		Box::pin(self.internal_send_message(message)).await
	}
	pub async fn send_message2(
		&self,
		req: ClientJsonRpcMessage,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		Box::pin(self.internal_send_message(req)).await
	}
	fn internal_send_message(
		&self,
		req: ClientJsonRpcMessage,
	) -> Pin<Box<dyn Future<Output = Result<StreamableHttpPostResponse, ClientError>> + Send + '_>> {
		Box::pin(self.internal_send_message2(req))
	}
	async fn internal_send_message2(
		&self,
		message: ClientJsonRpcMessage,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let body = serde_json::to_vec(&message).map_err(ClientError::new)?;

		let mut req = http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
			.body(body.into())
			.map_err(ClientError::new)?;

		if let Some(session_id) = self.session_id.load().clone() {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id.as_ref().parse().map_err(ClientError::new)?,
			);
		}

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(resp.status()));
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
					.map_err(ClientError::new)?;
				Ok(StreamableHttpPostResponse::Json(message, session_id))
			},
			_ => Err(ClientError::new(anyhow!(
				"unexpected content type: {:?}",
				content_type
			))),
		}
	}
}

// impl StreamableHttpClient for ClientWrapper {
// 	type Error = HttpError;
//
// 	async fn post_message(
// 		&self,
// 		uri: Arc<str>,
// 		message: ClientJsonRpcMessage,
// 		session_id: Option<Arc<str>>,
// 		_auth_header: Option<String>,
// 	) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
// 		let client = self.client.clone();
//
// 		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;
//
// 		let body =
// 			serde_json::to_vec(&message).map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		let mut req = http::Request::builder()
// 			.uri(uri)
// 			.method(http::Method::POST)
// 			.header(CONTENT_TYPE, "application/json")
// 			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
// 			.body(body.into())
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		if let Some(session_id) = session_id {
// 			req.headers_mut().insert(
// 				HEADER_SESSION_ID,
// 				session_id
// 					.as_ref()
// 					.parse()
// 					.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?,
// 			);
// 		}
//
// 		let resp = client
// 			.call_with_default_policies(req, &self.backend, self.policies.clone())
// 			.await
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		if resp.status() == http::StatusCode::ACCEPTED {
// 			return Ok(StreamableHttpPostResponse::Accepted);
// 		}
//
// 		if resp.status().is_client_error() || resp.status().is_server_error() {
// 			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
// 				"received status code {}",
// 				resp.status()
// 			))));
// 		}
//
// 		let content_type = resp.headers().get(CONTENT_TYPE);
// 		let session_id = resp
// 			.headers()
// 			.get(HEADER_SESSION_ID)
// 			.and_then(|v| v.to_str().ok())
// 			.map(|s| s.to_string());
//
// 		match content_type {
// 			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
// 				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
// 				Ok(StreamableHttpPostResponse::Sse(event_stream, session_id))
// 			},
// 			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
// 				let message = json::from_body::<ServerJsonRpcMessage>(resp.into_body())
// 					.await
// 					.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
// 				Ok(StreamableHttpPostResponse::Json(message, session_id))
// 			},
// 			_ => {
// 				tracing::error!("unexpected content type: {:?}", content_type);
// 				Err(StreamableHttpError::UnexpectedContentType(
// 					content_type.map(|ct| String::from_utf8_lossy(ct.as_bytes()).to_string()),
// 				))
// 			},
// 		}
// 	}
//
// 	async fn delete_session(
// 		&self,
// 		uri: Arc<str>,
// 		session_id: Arc<str>,
// 		_auth_header: Option<String>,
// 	) -> Result<(), StreamableHttpError<Self::Error>> {
// 		let client = self.client.clone();
//
// 		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;
//
// 		let req = http::Request::builder()
// 			.uri(uri)
// 			.method(http::Method::DELETE)
// 			.header(HEADER_SESSION_ID, session_id.as_ref())
// 			.body(Body::empty())
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		let resp = client
// 			.call_with_default_policies(req, &self.backend, self.policies.clone())
// 			.await
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		// If method not allowed, that's ok
// 		if resp.status() == http::StatusCode::METHOD_NOT_ALLOWED {
// 			tracing::debug!("this server doesn't support deleting session");
// 			return Ok(());
// 		}
//
// 		if resp.status().is_client_error() || resp.status().is_server_error() {
// 			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
// 				"received status code {}",
// 				resp.status()
// 			))));
// 		}
//
// 		Ok(())
// 	}
//
// 	async fn get_stream(
// 		&self,
// 		uri: Arc<str>,
// 		session_id: Arc<str>,
// 		last_event_id: Option<String>,
// 		_auth_header: Option<String>,
// 	) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
// 		let client = self.client.clone();
//
// 		let uri = "http://".to_string() + &self.backend.hostport() + &Self::parse_uri(uri)?;
//
// 		let mut reqb = http::Request::builder()
// 			.uri(uri)
// 			.method(http::Method::GET)
// 			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
// 			.header(HEADER_SESSION_ID, session_id.as_ref());
//
// 		if let Some(last_event_id) = last_event_id {
// 			reqb = reqb.header(HEADER_LAST_EVENT_ID, last_event_id);
// 		}
//
// 		let req = reqb
// 			.body(Body::empty())
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		let resp = client
// 			.call_with_default_policies(req, &self.backend, self.policies.clone())
// 			.await
// 			.map_err(|e| StreamableHttpError::Client(HttpError::new(e)))?;
//
// 		if resp.status() == http::StatusCode::METHOD_NOT_ALLOWED {
// 			return Err(StreamableHttpError::ServerDoesNotSupportSse);
// 		}
//
// 		if resp.status().is_client_error() || resp.status().is_server_error() {
// 			return Err(StreamableHttpError::Client(HttpError::new(anyhow!(
// 				"received status code {}",
// 				resp.status()
// 			))));
// 		}
//
// 		match resp.headers().get(CONTENT_TYPE) {
// 			Some(ct) => {
// 				if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
// 					return Err(StreamableHttpError::UnexpectedContentType(Some(
// 						String::from_utf8_lossy(ct.as_bytes()).to_string(),
// 					)));
// 				}
// 			},
// 			None => {
// 				return Err(StreamableHttpError::UnexpectedContentType(None));
// 			},
// 		}
//
// 		let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
// 		Ok(event_stream)
// 	}
// }
//
// impl SseClient for ClientWrapper {
// 	type Error = HttpError;
//
// 	async fn post_message(
// 		&self,
// 		uri: Uri,
// 		message: ClientJsonRpcMessage,
// 		_auth_token: Option<String>,
// 	) -> Result<(), SseTransportError<Self::Error>> {
// 		let uri = "http://".to_string()
// 			+ &self.backend.hostport()
// 			+ uri.path_and_query().map(|p| p.as_str()).unwrap_or_default();
// 		let body =
// 			serde_json::to_vec(&message).map_err(|e| SseTransportError::Client(HttpError::new(e)))?;
// 		let mut req = http::Request::builder()
// 			.uri(uri)
// 			.method(http::Method::POST)
// 			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
// 			.body(body.into())
// 			.map_err(|e| SseTransportError::Client(HttpError::new(e)))?;
//
// 		if let JsonRpcMessage::Request(request) = &message {
// 			match request.request.extensions().get::<RqCtx>() {
// 				Some(rq_ctx) => {
// 					let tracer = trcng::get_tracer();
// 					let _span = tracer
// 						.span_builder("sse_post")
// 						.with_kind(SpanKind::Client)
// 						.start_with_context(tracer, &rq_ctx.context);
// 					trcng::add_context_to_request(req.headers_mut(), &rq_ctx.context);
// 				},
// 				None => {
// 					trace!("No RqCtx found in extensions");
// 				},
// 			}
// 		}
//
// 		self
// 			.client
// 			.call_with_default_policies(req, &self.backend, self.policies.clone())
// 			.await
// 			.map_err(|e| SseTransportError::Client(HttpError::new(e)))
// 			.and_then(|resp| {
// 				if resp.status().is_client_error() || resp.status().is_server_error() {
// 					Err(SseTransportError::Client(HttpError::new(anyhow!(
// 						"received status code {}",
// 						resp.status()
// 					))))
// 				} else {
// 					Ok(resp)
// 				}
// 			})
// 			.map(drop)
// 	}
//
// 	fn get_stream(
// 		&self,
// 		uri: Uri,
// 		last_event_id: Option<String>,
// 		_auth_token: Option<String>,
// 	) -> impl Future<Output = Result<BoxedSseResponse, SseTransportError<Self::Error>>> + Send + '_ {
// 		Box::pin(async move {
// 			let uri = "http://".to_string()
// 				+ &self.backend.hostport()
// 				+ uri.path_and_query().map(|p| p.as_str()).unwrap_or_default();
//
// 			let mut reqb = http::Request::builder()
// 				.uri(uri)
// 				.method(http::Method::GET)
// 				.header(ACCEPT, EVENT_STREAM_MIME_TYPE);
// 			if let Some(last_event_id) = last_event_id {
// 				reqb = reqb.header(HEADER_LAST_EVENT_ID, last_event_id);
// 			}
// 			let req = reqb
// 				.body(Body::empty())
// 				.map_err(|e| SseTransportError::Client(HttpError::new(e)))?;
//
// 			let resp: Result<Response, ProxyError> = self
// 				.client
// 				.call_with_default_policies(req, &self.backend, self.policies.clone())
// 				.await;
//
// 			let resp = resp
// 				.map_err(|e| SseTransportError::Client(HttpError::new(e)))
// 				.and_then(|resp| {
// 					if resp.status().is_client_error() || resp.status().is_server_error() {
// 						Err(SseTransportError::Client(HttpError::new(anyhow!(
// 							"received status code {}",
// 							resp.status()
// 						))))
// 					} else {
// 						Ok(resp)
// 					}
// 				})?;
// 			match resp.headers().get(CONTENT_TYPE) {
// 				Some(ct) => {
// 					if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
// 						return Err(SseTransportError::UnexpectedContentType(Some(ct.clone())));
// 					}
// 				},
// 				None => {
// 					return Err(SseTransportError::UnexpectedContentType(None));
// 				},
// 			}
//
// 			let event_stream =
// 				sse_stream::SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
// 			Ok(event_stream)
// 		})
// 	}
// }
