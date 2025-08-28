use crate::http::{Request, Response};
use crate::mcp::relay::Relay;
use crate::mcp::relay::upstream::UpstreamError;
use crate::*;
use ::http::StatusCode;
use ::http::request::Parts;
use futures_util::{SinkExt, StreamExt};
use http_body::Body;
use http_body_util::Full;
use http_body_util::combinators::BoxBody;
use rmcp::{ErrorData, ServerHandler};
use rmcp::model::{
	ClientJsonRpcMessage, ClientRequest, JsonRpcMessage, RequestId, ServerJsonRpcMessage,
	ServerNotification, ServerRequest, ServerResult,
};
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::common::server_side_http::{ServerSseMessage, session_id};
use sse_stream::{KeepAlive, Sse, SseBody};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct LocalSession {
	relay: Arc<Relay>,
	pub id: Arc<str>,
}

impl LocalSession {
	pub async fn send(&self, parts: Parts, message: ClientJsonRpcMessage) -> Response {
		// TODO: propagate errors better
		self
			.send_internal(parts, message)
			.await
			.unwrap_or_else(|e| {
				http_error(
					StatusCode::INTERNAL_SERVER_ERROR,
					format!("failed to send message: {e}",),
				)
			})
	}

	async fn send_internal(
		&self,
		parts: Parts,
		message: ClientJsonRpcMessage,
	) -> Result<Response, UpstreamError> {
		match message {
			ClientJsonRpcMessage::Request(r) => {
				let req = r.request;
				let req_id = r.id;
				let method = req.method();
				let (_span, ref rq_ctx, log, cel) = mcp::relay::setup_request_log2(&parts, method);
				match req {
					ClientRequest::InitializeRequest(r) => stream(self.relay.initialize(r).await?, req_id),
					ClientRequest::ListToolsRequest(r) => {
						merge_to_response(self.relay.list_tools2(r,req_id, cel.clone()).await?)
					},
					ClientRequest::CallToolRequest(r) => {
						stream(self.relay.call_tool(r, &cel, log).await?, req_id);
						todo!()
					},
					_ => todo!(),
				}
			},
			ClientJsonRpcMessage::Notification(not) => {
				self.relay.notify(not.notification).await?;
				Ok(accepted_response())
			},
			_ => todo!(),
		}
	}
}

fn stream(resp: impl Into<ServerResult>, req_id: RequestId) -> Result<Response, UpstreamError> {
	let rpc = ServerJsonRpcMessage::response(resp.into(), req_id);
	let stream = futures::stream::once(async {
		ServerSseMessage {
			event_id: None,
			message: Arc::new(rpc),
		}
	});
	Ok(sse_stream_response(stream, None))
}

fn merge_to_response(stream: super::mergestream::MergeStream) -> Result<Response, UpstreamError> {
	let stream = stream.map(|rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			// TODO: do not hardcode number
			Err(e) => ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), RequestId::Number(2))
		};
		// TODO: is it ok to have no event_id here?
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(sse_stream_response(stream, None))
}

#[derive(Default, Debug)]
pub struct SessionManager {
	sessions: std::sync::RwLock<HashMap<String, LocalSession>>,
}

impl SessionManager {
	pub fn get_session(&self, id: &str) -> Option<LocalSession> {
		self.sessions.read().ok()?.get(id).cloned()
	}
	pub fn create_session(&self, relay: Relay) -> LocalSession {
		let id = session_id();
		let sess = LocalSession {
			id: id.clone(),
			relay: Arc::new(relay),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		sess
	}
}

#[allow(dead_code)]
fn require_send<T: Send>() {}
pub struct StreamableHttpService {
	config: StreamableHttpServerConfig,
	session_manager: Arc<SessionManager>,
	service_factory: Arc<dyn Fn() -> Result<Relay, http::Error> + Send + Sync>,
}

impl StreamableHttpService {
	pub fn new(
		service_factory: impl Fn() -> Result<Relay, http::Error> + Send + Sync + 'static,
		session_manager: Arc<SessionManager>,
		config: StreamableHttpServerConfig,
	) -> Self {
		require_send::<StreamableHttpService>();
		Self {
			config,
			session_manager,
			service_factory: Arc::new(service_factory),
		}
	}

	pub async fn handle(&self, request: Request) -> Response {
		let method = request.method().clone();
		let allowed_methods = match self.config.stateful_mode {
			true => "GET, POST, DELETE",
			false => "POST",
		};
		let result = match (method, self.config.stateful_mode) {
			(http::Method::POST, _) => self.handle_post(request).await,
			// if we're not in stateful mode, we don't support GET or DELETE because there is no session
			(http::Method::GET, true) => self.handle_get(request).await,
			(http::Method::DELETE, true) => self.handle_delete(request).await,
			_ => {
				// Handle other methods or return an error
				let response = ::http::Response::builder()
					.status(http::StatusCode::METHOD_NOT_ALLOWED)
					.header(http::header::ALLOW, allowed_methods)
					.body(http::Body::from("Method Not Allowed"))
					.expect("valid response");
				return response;
			},
		};
		result
	}

	pub async fn handle_post(&self, request: Request) -> Response {
		// check accept header
		if !request
			.headers()
			.get(http::header::ACCEPT)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| {
				header.contains(JSON_MIME_TYPE) && header.contains(EVENT_STREAM_MIME_TYPE)
			}) {
			return http_error(
				StatusCode::NOT_ACCEPTABLE,
				"Not Acceptable: Client must accept both application/json and text/event-stream",
			);
		}

		// check content type
		if !request
			.headers()
			.get(http::header::CONTENT_TYPE)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| header.starts_with(JSON_MIME_TYPE))
		{
			return http_error(
				StatusCode::UNSUPPORTED_MEDIA_TYPE,
				"Unsupported Media Type: Client must send application/json",
			);
		}

		let (part, body) = request.into_parts();
		let message = match json::from_body::<ClientJsonRpcMessage>(body).await {
			Ok(b) => b,
			Err(e) => {
				return http_error(
					StatusCode::BAD_REQUEST,
					format!("fail to deserialize request body: {e}"),
				);
			},
		};

		if self.config.stateful_mode {
			let session_id = part
				.headers
				.get(HEADER_SESSION_ID)
				.and_then(|v| v.to_str().ok());
			let (session, set_session_id) = if let Some(session_id) = session_id {
				let Some(session) = self.session_manager.get_session(session_id) else {
					return http_error(http::StatusCode::NOT_FOUND, "Session not found");
				};
				(session, false)
			} else {
				// No session header... we need to create one, if it is an initialize
				if let ClientJsonRpcMessage::Request(req) = &message {
					if !matches!(req.request, ClientRequest::InitializeRequest(_)) {
						return http_error(
							StatusCode::UNPROCESSABLE_ENTITY,
							"session header is required for non-initialize requests",
						);
					}
				}
				let relay = match (self.service_factory)() {
					Ok(r) => r,
					Err(e) => {
						return http_error(
							StatusCode::INTERNAL_SERVER_ERROR,
							format!("fail to create relay: {e}"),
						);
					},
				};
				let session = self.session_manager.create_session(relay);
				(session, true)
			};
			let mut resp = session.send(part, message).await;
			if set_session_id {
				let Ok(sid) = session.id.parse() else {
					return internal_error_response("create session id header");
				};
				resp.headers_mut().insert(HEADER_SESSION_ID, sid);
			}
			resp
		// todo!()
		} else {
			todo!()
		}
	}

	pub async fn handle_get(&self, request: Request) -> Response {
		todo!()
	}
	pub async fn handle_delete(&self, request: Request) -> Response {
		todo!()
	}
}

fn http_error(status: StatusCode, body: impl Into<http::Body>) -> Response {
	::http::Response::builder()
		.status(status)
		.body(body.into())
		.expect("valid response")
}

fn sse_stream_response(
	stream: impl futures::Stream<Item = ServerSseMessage> + Send + 'static,
	keep_alive: Option<Duration>,
) -> Response {
	use futures::StreamExt;
	let stream = SseBody::new(stream.map(|message| {
		let data = serde_json::to_string(&message.message).expect("valid message");
		let mut sse = Sse::default().data(data);
		sse.id = message.event_id;
		Result::<Sse, Infallible>::Ok(sse)
	}));
	let stream = match keep_alive {
		Some(duration) => {
			http::Body::new(stream.with_keep_alive::<TokioSseTimer>(KeepAlive::new().interval(duration)))
		},
		None => http::Body::new(stream),
	};
	::http::Response::builder()
		.status(StatusCode::OK)
		.header(http::header::CONTENT_TYPE, EVENT_STREAM_MIME_TYPE)
		.header(http::header::CACHE_CONTROL, "no-cache")
		.body(stream)
		.expect("valid response")
}

pin_project_lite::pin_project! {
		struct TokioSseTimer {
				#[pin]
				sleep: tokio::time::Sleep,
		}
}
impl Future for TokioSseTimer {
	type Output = ();

	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		let this = self.project();
		this.sleep.poll(cx)
	}
}
impl sse_stream::Timer for TokioSseTimer {
	fn from_duration(duration: Duration) -> Self {
		Self {
			sleep: tokio::time::sleep(duration),
		}
	}

	fn reset(self: std::pin::Pin<&mut Self>, when: std::time::Instant) {
		let this = self.project();
		this.sleep.reset(tokio::time::Instant::from_std(when));
	}
}
fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(http::Body::empty())
		.expect("valid response")
}

fn internal_error_response(context: &str) -> Response {
	::http::Response::builder()
		.status(StatusCode::INTERNAL_SERVER_ERROR)
		.body(http::Body::from(format!(
			"Encounter an error when {context}"
		)))
		.expect("valid response")
}
