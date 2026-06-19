use std::sync::Arc;

use ::http::StatusCode;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ProtocolVersion, ServerJsonRpcMessage};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_MCP_PROTOCOL_VERSION, HEADER_SESSION_ID, JSON_MIME_TYPE,
};

use crate::http::{DropBody, Request, Response};
use crate::mcp::handler::RelayInputs;
use crate::mcp::session::SessionManager;
use crate::proxy::ProxyError;
use crate::*;

#[derive(Debug, Clone)]
pub struct StreamableHttpServerConfig {
	/// If true, the server will create a session for each request and keep it alive.
	pub stateful_mode: bool,
}

#[derive(Debug, Clone)]
pub struct ServerSseMessage {
	pub event_id: Option<String>,
	pub message: Arc<ServerJsonRpcMessage>,
}

type BoxedSseStream =
	futures::stream::BoxStream<'static, Result<sse_stream::Sse, sse_stream::Error>>;
#[allow(clippy::large_enum_variant)]
pub enum StreamableHttpPostResponse {
	Accepted,
	Json(ServerJsonRpcMessage, Option<String>),
	Sse(BoxedSseStream, Option<String>),
}

impl std::fmt::Debug for StreamableHttpPostResponse {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Accepted => write!(f, "Accepted"),
			Self::Json(arg0, arg1) => f.debug_tuple("Json").field(arg0).field(arg1).finish(),
			Self::Sse(_, arg1) => f.debug_tuple("Sse").field(arg1).finish(),
		}
	}
}
pub struct StreamableHttpService {
	config: StreamableHttpServerConfig,
	session_manager: Arc<SessionManager>,
}

impl StreamableHttpService {
	pub fn new(session_manager: Arc<SessionManager>, config: StreamableHttpServerConfig) -> Self {
		Self {
			config,
			session_manager,
		}
	}

	pub async fn handle(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		let method = request.method().clone();

		match (method, self.config.stateful_mode) {
			(http::Method::POST, _) => Box::pin(self.handle_post(request, inputs)).await,
			// if we're not in stateful mode, we don't support GET or DELETE because there is no session
			(http::Method::GET, true) => self.handle_get(request, inputs).await,
			(http::Method::DELETE, true) => self.handle_delete(request).await,
			_ => Err(ProxyError::MCP(mcp::Error::MethodNotAllowed)),
		}
	}

	pub async fn handle_post(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		// check accept header
		if !request
			.headers()
			.get(http::header::ACCEPT)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| {
				header.contains(JSON_MIME_TYPE) && header.contains(EVENT_STREAM_MIME_TYPE)
			}) {
			return mcp::Error::InvalidAccept.into();
		}

		// check content type
		if !request
			.headers()
			.get(http::header::CONTENT_TYPE)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| header.starts_with(JSON_MIME_TYPE))
		{
			return mcp::Error::InvalidContentType.into();
		}

		let limit = http::buffer_limit(&request);
		let (part, body) = request.into_parts();
		let message = match json::from_body_with_limit::<ClientJsonRpcMessage>(body, limit).await {
			Ok(b) => b,
			Err(e) => {
				return mcp::Error::Deserialize(e).into();
			},
		};
		let header_protocol_version = protocol_version_header(&part.headers)?;

		if !self.config.stateful_mode {
			let relay = inputs.build_new_connections()?;
			// Use stateless session - not registered in session manager
			let mut session = self.session_manager.create_stateless_session(relay);
			let response = Box::pin(session.stateless_send_and_initialize(part.clone(), message)).await;

			let (tx, rx) = tokio::sync::oneshot::channel::<()>();
			// Clean up upstream resources (e.g., stdio processes)
			tokio::task::spawn(async move {
				// Wait until the response is actually completed.
				let _ = rx.await;
				trace!("cleaning up stateless session");
				let _ = session.delete_session(part).await;
			});
			return response.map(|r| r.map(|b| DropBody::new(b, tx)));
		}

		let session_id = part
			.headers
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok());

		if let Some(session_id) = session_id {
			let Some(mut session) = self
				.session_manager
				.get_or_resume_session(session_id, inputs)?
			else {
				return mcp::Error::UnknownSession.into();
			};

			return Box::pin(session.send(part, message)).await;
		}

		// No session header... we need to create one, if it is an initialize.
		// Notifications and responses are subsequent-session messages too.
		let is_initialize_request = match &message {
			ClientJsonRpcMessage::Request(req) => {
				matches!(req.request, ClientRequest::InitializeRequest(_))
			},
			_ => false,
		};
		if !is_initialize_request {
			return mcp::Error::MissingSessionHeader.into();
		}
		// Legacy stable clients did not consistently send MCP-Protocol-Version on
		// initialize, so omission is accepted. If the header is present, it must
		// describe the same protocol version as the JSON-RPC initialize body.
		if let Some(header_protocol_version) = header_protocol_version.as_ref()
			&& let ClientJsonRpcMessage::Request(req) = &message
			&& let ClientRequest::InitializeRequest(init) = &req.request
			&& header_protocol_version != &init.params.protocol_version
		{
			return mcp::Error::InvalidProtocolVersion.into();
		}
		let idle_ttl = inputs.backend.session_idle_ttl;
		let relay = inputs.build_new_connections()?;
		let mut session = self.session_manager.create_session(relay);
		let mut resp = Box::pin(session.send(part, message)).await?;

		let Ok(sid) = session.id.parse() else {
			return mcp::Error::InvalidSessionIdHeader.into();
		};
		resp.headers_mut().insert(HEADER_SESSION_ID, sid);
		self.session_manager.insert_session(session, idle_ttl);
		Ok(resp)
	}

	pub async fn handle_get(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		// Just validate it
		let _header_protocol_version = protocol_version_header(request.headers())?;
		// check accept header
		if !request
			.headers()
			.get(http::header::ACCEPT)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| header.contains(EVENT_STREAM_MIME_TYPE))
		{
			return mcp::Error::InvalidAcceptGet.into();
		}

		let Some(session_id) = request
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
		else {
			return mcp::Error::SessionIdRequired.into();
		};

		let Some(session) = self.session_manager.get_session(session_id, inputs) else {
			return mcp::Error::UnknownSession.into();
		};

		let (parts, _) = request.into_parts();
		session.get_stream(parts).await
	}

	pub async fn handle_delete(&self, request: Request) -> Result<Response, ProxyError> {
		// Just validate it
		let _header_protocol_version = protocol_version_header(request.headers())?;
		// check session id
		let session_id = request
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok());
		let Some(session_id) = session_id else {
			return mcp::Error::SessionIdRequired.into();
		};
		let session_id = session_id.to_string();
		let (parts, _) = request.into_parts();
		Ok(
			self
				.session_manager
				.delete_session(&session_id, parts)
				.await
				.unwrap_or_else(accepted_response),
		)
	}
}

fn protocol_version_header(
	headers: &::http::HeaderMap,
) -> Result<Option<ProtocolVersion>, ProxyError> {
	let Some(value) = headers.get(HEADER_MCP_PROTOCOL_VERSION) else {
		return Ok(None);
	};
	let value = value
		.to_str()
		.map_err(|_| ProxyError::MCP(mcp::Error::InvalidProtocolVersion))?;
	let version = ProtocolVersion::KNOWN_VERSIONS
		.iter()
		.find(|version| version.as_str() == value)
		.cloned()
		.ok_or(ProxyError::MCP(mcp::Error::InvalidProtocolVersion))?;
	Ok(Some(version))
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
