use std::sync::Arc;

use ::http::StatusCode;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest};
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};

use crate::http::{Request, Response};
use crate::mcp::handler::Relay;
use crate::mcp::session::SessionManager;
use crate::*;

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

		match (method, self.config.stateful_mode) {
			(http::Method::POST, _) => self.handle_post(request).await,
			// if we're not in stateful mode, we don't support GET or DELETE because there is no session
			(http::Method::GET, true) => self.handle_get(request).await,
			(http::Method::DELETE, true) => self.handle_delete(request).await,
			_ => {
				// Handle other methods or return an error

				::http::Response::builder()
					.status(http::StatusCode::METHOD_NOT_ALLOWED)
					.header(http::header::ALLOW, allowed_methods)
					.body(http::Body::from("Method Not Allowed"))
					.expect("valid response")
			},
		}
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

		let limit = http::buffer_limit(&request);
		let (part, body) = request.into_parts();
		let message = match json::from_body_with_limit::<ClientJsonRpcMessage>(body, limit).await {
			Ok(b) => b,
			Err(e) => {
				return http_error(
					StatusCode::BAD_REQUEST,
					format!("fail to deserialize request body: {e}"),
				);
			},
		};

		if !self.config.stateful_mode {
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
			return session.stateless_send_and_initialize(part, message).await;
		}

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
			if let ClientJsonRpcMessage::Request(req) = &message
				&& !matches!(req.request, ClientRequest::InitializeRequest(_))
			{
				return http_error(
					StatusCode::UNPROCESSABLE_ENTITY,
					"session header is required for non-initialize requests",
				);
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
	}

	pub async fn handle_get(&self, request: Request) -> Response {
		// check accept header
		if !request
			.headers()
			.get(http::header::ACCEPT)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| header.contains(EVENT_STREAM_MIME_TYPE))
		{
			return http_error(
				StatusCode::NOT_ACCEPTABLE,
				"Not Acceptable: Client must accept text/event-stream",
			);
		}

		let Some(session_id) = request
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
		else {
			return http_error(StatusCode::UNPROCESSABLE_ENTITY, "Session ID is required");
		};

		let Some(session) = self.session_manager.get_session(session_id) else {
			return http_error(http::StatusCode::NOT_FOUND, "Session not found");
		};

		let (parts, _) = request.into_parts();
		session.get_stream(parts).await
	}

	pub async fn handle_delete(&self, request: Request) -> Response {
		// check session id
		let session_id = request
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok());
		let Some(session_id) = session_id else {
			// unauthorized
			return http_error(
				StatusCode::UNAUTHORIZED,
				"Unauthorized: Session ID is required",
			);
		};
		let session_id = session_id.to_string();
		let (parts, _) = request.into_parts();
		self
			.session_manager
			.delete_session(&session_id, parts)
			.await
			.unwrap_or_else(accepted_response)
	}
}

fn http_error(status: StatusCode, body: impl Into<http::Body>) -> Response {
	::http::Response::builder()
		.status(status)
		.body(body.into())
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

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
