use std::sync::Arc;

use ::http::StatusCode;
use axum::extract::Query;
use axum::response::Sse;
use axum::response::sse::Event;
use axum_core::response::IntoResponse;
use futures_util::StreamExt;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest};
use rmcp::transport::sse_server::PostEventQuery;
use tokio_stream::wrappers::ReceiverStream;

use crate::http::{DropBody, Request, Response, filters};
use crate::mcp::handler::Relay;
use crate::mcp::session;
use crate::mcp::session::SessionManager;
use crate::*;

pub struct LegacySSEService {
	session_manager: Arc<SessionManager>,
	service_factory: Arc<dyn Fn() -> Result<Relay, http::Error> + Send + Sync>,
}

impl LegacySSEService {
	pub fn new(
		service_factory: impl Fn() -> Result<Relay, http::Error> + Send + Sync + 'static,
		session_manager: Arc<SessionManager>,
	) -> Self {
		Self {
			session_manager,
			service_factory: Arc::new(service_factory),
		}
	}

	pub async fn handle(&self, request: Request) -> Response {
		let method = request.method().clone();

		match method {
			http::Method::POST => self.handle_post(request).await,
			http::Method::GET => self.handle_get(request).await,
			_ => ::http::Response::builder()
				.status(http::StatusCode::METHOD_NOT_ALLOWED)
				.header(http::header::ALLOW, "GET, POST")
				.body(http::Body::from("Method Not Allowed"))
				.expect("valid response"),
		}
	}

	pub async fn handle_post(&self, request: Request) -> Response {
		// Extract query parameters
		let Ok(Query(PostEventQuery { session_id })) =
			Query::<PostEventQuery>::try_from_uri(request.uri())
		else {
			return http_error(StatusCode::BAD_REQUEST, "failed to process session_id");
		};
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

		let Some(session) = self.session_manager.get_session(&session_id) else {
			return http_error(http::StatusCode::NOT_FOUND, "Session not found");
		};

		// To proxy SSE to streamable HTTP, we need to establish a GET stream for notifications.
		// We need to do this *after* the upstream session is established.
		// Here, we wait until the InitializeRequest is sent, and then establish the GET stream once it is.
		let is_init = matches!(&message, ClientJsonRpcMessage::Request(r) if matches!(&r.request, &ClientRequest::InitializeRequest(_)));
		let init_parts = if is_init { Some(part.clone()) } else { None };
		let resp = session.send(part, message).await;
		if is_init {
			trace!("received initialize request, establishing get stream");
			let get_stream = session.get_stream(init_parts.unwrap()).await;
			if let Err(e) = session.forward_legacy_sse(get_stream).await {
				return http_error(
					StatusCode::INTERNAL_SERVER_ERROR,
					format!("fail to establish get stream: {e}"),
				);
			}
		}
		if let Err(e) = session.forward_legacy_sse(resp).await {
			return http_error(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("fail to send message: {e}"),
			);
		}
		accepted_response()
	}

	pub async fn handle_get(&self, request: Request) -> Response {
		let relay = match (self.service_factory)() {
			Ok(r) => r,
			Err(e) => {
				return http_error(
					StatusCode::INTERNAL_SERVER_ERROR,
					format!("fail to create relay: {e}"),
				);
			},
		};

		// GET requests establish an SSE stream.
		// We will return the sessionId, and all future responses will get sent on the rx channel to send to this channel.
		let (session, rx) = self.session_manager.create_legacy_session(relay);
		let mut base_url = request
			.extensions()
			.get::<filters::OriginalUrl>()
			.map(|u| u.0.clone())
			.unwrap_or_else(|| request.uri().clone());
		if let Err(e) = http::modify_url(&mut base_url, |url| {
			url.query_pairs_mut().append_pair("sessionId", &session.id);
			Ok(())
		}) {
			return http_error(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("fail to create SSE url: {e}"),
			);
		}
		let stream = futures::stream::once(futures::future::ok(
			Event::default().event("endpoint").data(
				base_url
					.path_and_query()
					.map(ToString::to_string)
					.unwrap_or_default(),
			),
		))
		.chain(
			ReceiverStream::new(rx).map(|message| match serde_json::to_string(&message) {
				Ok(bytes) => Ok(Event::default().event("message").data(&bytes)),
				Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
			}),
		);
		let (parts, _) = request.into_parts();
		Sse::new(stream).into_response().map(|b| {
			http::Body::new(DropBody::new(
				b,
				session::dropper(self.session_manager.clone(), session, parts),
			))
		})
	}
}

fn http_error(status: StatusCode, body: impl Into<http::Body>) -> Response {
	::http::Response::builder()
		.status(status)
		.body(body.into())
		.expect("valid response")
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
