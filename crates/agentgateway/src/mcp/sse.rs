use std::sync::Arc;

use ::http::StatusCode;
use axum::extract::Query;
use axum::response::Sse;
use axum::response::sse::Event;
use axum_core::response::IntoResponse;
use futures_util::StreamExt;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest};
use tokio_stream::wrappers::ReceiverStream;

use crate::http::{DropBody, Request, Response, filters};
use crate::mcp::handler::RelayInputs;
use crate::mcp::session;
use crate::mcp::session::SessionManager;
use crate::proxy::ProxyError;
use crate::*;

pub struct LegacySSEService {
	session_manager: Arc<SessionManager>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostEventQuery {
	pub session_id: String,
}

impl LegacySSEService {
	pub fn new(session_manager: Arc<SessionManager>) -> Self {
		Self { session_manager }
	}

	pub async fn handle(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		let method = request.method().clone();

		match method {
			http::Method::POST => self.handle_post(request, inputs).await,
			http::Method::GET => self.handle_get(request, inputs).await,
			_ => Err(ProxyError::MCP(mcp::Error::MethodNotAllowed)),
		}
	}

	pub async fn handle_post(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		// Extract query parameters
		let Ok(Query(PostEventQuery { session_id })) =
			Query::<PostEventQuery>::try_from_uri(request.uri())
		else {
			return mcp::Error::InvalidSessionIdQuery.into();
		};
		let limit = http::buffer_limit(&request);
		let (part, body) = request.into_parts();
		let message = json::from_body_with_limit::<ClientJsonRpcMessage>(body, limit)
			.await
			.map_err(mcp::Error::Deserialize)?;

		let Some(mut session) = self.session_manager.get_session(&session_id, inputs) else {
			return mcp::Error::UnknownSession.into();
		};

		// To proxy SSE to streamable HTTP, we need to establish a GET stream for notifications.
		// We need to do this *after* the upstream session is established.
		// Here, we wait until the InitializeRequest is sent, and then establish the GET stream once it is.
		let is_init = matches!(&message, ClientJsonRpcMessage::Request(r) if matches!(&r.request, &ClientRequest::InitializeRequest(_)));
		let init_parts = if is_init { Some(part.clone()) } else { None };
		let resp = session.send(part, message).await?;
		if is_init {
			trace!("received initialize request, establishing get stream");
			let get_stream = session.get_stream(init_parts.unwrap()).await?;
			if let Err(e) = session.forward_legacy_sse(get_stream).await {
				return mcp::Error::EstablishGetStream(e.to_string()).into();
			}
		}
		if let Err(e) = session.forward_legacy_sse(resp).await {
			return mcp::Error::ForwardLegacySse(e.to_string()).into();
		}
		Ok(accepted_response())
	}

	pub async fn handle_get(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		let relay = inputs.build_new_connections()?;

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
			return mcp::Error::CreateSseUrl(e.to_string()).into();
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
		Ok(Sse::new(stream).into_response().map(|b| {
			http::Body::new(DropBody::new(
				b,
				session::dropper(self.session_manager.clone(), session, parts),
			))
		}))
	}
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
