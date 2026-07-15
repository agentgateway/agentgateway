use std::sync::Arc;

use ::http::{HeaderMap, StatusCode};
use agent_core::prelude::AssertSize;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, GetMeta, ProtocolVersion,
	RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_MCP_METHOD, HEADER_MCP_NAME, HEADER_MCP_PARAM_PREFIX,
	HEADER_MCP_PROTOCOL_VERSION, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::common::mcp_headers::{decode_header_value, encode_header_value};

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

#[derive(Debug, Clone)]
pub(crate) struct RequestProtocol {
	version: Option<ProtocolVersion>,
}

impl RequestProtocol {
	pub(crate) fn is_modern(&self) -> bool {
		self
			.version
			.as_ref()
			.is_some_and(|version| version.as_str() >= ProtocolVersion::STANDARD_HEADERS.as_str())
	}

	pub(crate) fn uses_sessions(&self) -> bool {
		!self.is_modern()
	}
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
			(http::Method::POST, _) => {
				Box::pin(
					self
						.handle_post(request, inputs)
						.assert_size::<{ 4 * 1024 }>(),
				)
				.await
			},
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
		let (mut part, body) = request.into_parts();
		let message = match json::from_body_with_limit::<ClientJsonRpcMessage>(body, limit).await {
			Ok(b) => b,
			Err(e) => return mcp::Error::Deserialize(e).into(),
		};
		let request_id = request_id(&message);
		let protocol = request_protocol(&part.headers, &message, request_id.clone())?;
		validate_standard_headers(&part.headers, &message, &protocol)?;
		part.extensions.insert(protocol.clone());

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
			if !protocol.uses_sessions() {
				return mcp::Error::InvalidSessionIdHeader.into();
			}
			let Some(mut session) = self
				.session_manager
				.get_or_resume_session(session_id, inputs)?
			else {
				return mcp::Error::UnknownSession.into();
			};

			return Box::pin(session.send(part, message)).await;
		}

		if !protocol.uses_sessions() {
			let relay = inputs.build_new_connections()?;
			let mut session = self.session_manager.create_stateless_session(relay);
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
		// The GET event stream is legacy-only (SEP-2567 removed it for modern).
		reject_modern_session_request(request.headers())?;
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
		// Session deletion is legacy-only (SEP-2567 removed sessions for modern).
		reject_modern_session_request(request.headers())?;
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

pub(crate) fn emit_standard_headers(headers: &mut HeaderMap, message: &ClientJsonRpcMessage) {
	match message_method(message) {
		Some(method) => {
			if let Ok(value) = ::http::HeaderValue::from_str(&encode_header_value(method)) {
				headers.insert(HEADER_MCP_METHOD, value);
			}
		},
		None => {
			headers.remove(HEADER_MCP_METHOD);
		},
	}
	match message_name(message) {
		Some(name) => {
			if let Ok(value) = ::http::HeaderValue::from_str(&encode_header_value(name)) {
				headers.insert(HEADER_MCP_NAME, value);
			}
		},
		None => {
			headers.remove(HEADER_MCP_NAME);
		},
	}
}

fn validate_standard_headers(
	headers: &HeaderMap,
	message: &ClientJsonRpcMessage,
	protocol: &RequestProtocol,
) -> Result<(), ProxyError> {
	let request_id = request_id(message);
	let modern = protocol.is_modern();

	validate_standard_header(
		headers,
		HEADER_MCP_METHOD,
		message_method(message),
		modern && message_method(message).is_some(),
		request_id.clone(),
	)?;
	validate_standard_header(
		headers,
		HEADER_MCP_NAME,
		message_name(message),
		modern && message_name(message).is_some(),
		request_id.clone(),
	)?;

	for (name, value) in headers {
		if name
			.as_str()
			.get(..HEADER_MCP_PARAM_PREFIX.len())
			.is_some_and(|prefix| prefix.eq_ignore_ascii_case(HEADER_MCP_PARAM_PREFIX))
		{
			let value = value.to_str().map_err(|_| {
				mcp::Error::InvalidRoutingHeader(request_id.clone(), HEADER_MCP_PARAM_PREFIX)
			})?;
			if decode_header_value(value).is_none() {
				return Err(
					mcp::Error::InvalidRoutingHeader(request_id.clone(), HEADER_MCP_PARAM_PREFIX).into(),
				);
			}
		}
	}

	Ok(())
}

fn validate_standard_header(
	headers: &HeaderMap,
	header_name: &'static str,
	body_value: Option<&str>,
	required: bool,
	request_id: Option<RequestId>,
) -> Result<(), ProxyError> {
	let Some(raw) = headers.get(header_name) else {
		if required {
			return Err(mcp::Error::InvalidRoutingHeader(request_id, header_name).into());
		}
		return Ok(());
	};
	let raw = raw
		.to_str()
		.map_err(|_| mcp::Error::InvalidRoutingHeader(request_id.clone(), header_name))?;
	let decoded = decode_header_value(raw)
		.ok_or_else(|| mcp::Error::InvalidRoutingHeader(request_id.clone(), header_name))?;
	if let Some(body_value) = body_value
		&& decoded != body_value
	{
		return Err(mcp::Error::HeaderBodyMismatch(request_id, header_name).into());
	}
	Ok(())
}

fn request_id(message: &ClientJsonRpcMessage) -> Option<RequestId> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.id.clone()),
		_ => None,
	}
}

fn message_method(message: &ClientJsonRpcMessage) -> Option<&str> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.request.method()),
		ClientJsonRpcMessage::Notification(notification) => Some(match &notification.notification {
			ClientNotification::CancelledNotification(n) => n.method.as_str(),
			ClientNotification::ProgressNotification(n) => n.method.as_str(),
			ClientNotification::InitializedNotification(n) => n.method.as_str(),
			ClientNotification::RootsListChangedNotification(n) => n.method.as_str(),
			ClientNotification::TaskStatusNotification(n) => n.method.as_str(),
			ClientNotification::CustomNotification(n) => n.method.as_str(),
		}),
		_ => None,
	}
}

fn message_name(message: &ClientJsonRpcMessage) -> Option<&str> {
	let ClientJsonRpcMessage::Request(req) = message else {
		return None;
	};
	match &req.request {
		ClientRequest::CallToolRequest(r) => Some(&r.params.name),
		ClientRequest::GetPromptRequest(r) => Some(&r.params.name),
		ClientRequest::ReadResourceRequest(r) => Some(&r.params.uri),
		ClientRequest::SubscribeRequest(r) => Some(&r.params.uri),
		ClientRequest::UnsubscribeRequest(r) => Some(&r.params.uri),
		_ => None,
	}
}

pub(crate) fn protocol_version_header(
	headers: &::http::HeaderMap,
	request_id: Option<RequestId>,
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
		.ok_or_else(|| {
			ProxyError::MCP(mcp::Error::UnsupportedVersion(
				request_id,
				value.to_string(),
			))
		})?;
	Ok(Some(version))
}

fn request_protocol(
	headers: &::http::HeaderMap,
	message: &ClientJsonRpcMessage,
	request_id: Option<RequestId>,
) -> Result<RequestProtocol, ProxyError> {
	let header_version = protocol_version_header(headers, request_id.clone())?;
	let body_version = message_protocol_version(message);
	let initialize = is_initialize_request(message);

	if let (Some(header), Some(body)) = (&header_version, &body_version)
		&& header != body
	{
		return Err(mcp::Error::VersionMismatch(request_id).into());
	}

	let declared_version = header_version.as_ref().or(body_version.as_ref());
	let declares_modern_version = declared_version
		.is_some_and(|version| version.as_str() >= ProtocolVersion::STANDARD_HEADERS.as_str());
	let missing_modern_version_source = header_version.is_none() || body_version.is_none();
	if declares_modern_version && missing_modern_version_source {
		return Err(mcp::Error::InvalidProtocolVersion.into());
	}

	let version = body_version.or(header_version);
	if initialize
		&& let Some(v) = version.as_ref()
		&& v.as_str() >= ProtocolVersion::STANDARD_HEADERS.as_str()
	{
		// `initialize` selects legacy session semantics. Modern versions use
		// `server/discover` plus per-request `_meta`, so accepting 2026+ here would
		// create a session that claims a protocol era that no longer defines it.
		return Err(mcp::Error::UnsupportedVersionForInitialize(request_id, v.to_string()).into());
	}

	Ok(RequestProtocol { version })
}

fn is_initialize_request(message: &ClientJsonRpcMessage) -> bool {
	matches!(
		message,
		ClientJsonRpcMessage::Request(req)
			if matches!(req.request, ClientRequest::InitializeRequest(_))
	)
}

fn message_protocol_version(message: &ClientJsonRpcMessage) -> Option<ProtocolVersion> {
	match message {
		ClientJsonRpcMessage::Request(req) => match &req.request {
			ClientRequest::InitializeRequest(init) => Some(init.params.protocol_version.clone()),
			_ => req.request.get_meta().protocol_version(),
		},
		ClientJsonRpcMessage::Notification(notification) => {
			notification.notification.get_meta().protocol_version()
		},
		_ => None,
	}
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}

fn reject_modern_session_request(headers: &::http::HeaderMap) -> Result<(), ProxyError> {
	if let Some(version) = protocol_version_header(headers, None)?
		&& version.as_str() >= ProtocolVersion::STANDARD_HEADERS.as_str()
	{
		return Err(mcp::Error::UnsupportedVersion(None, version.to_string()).into());
	}
	Ok(())
}
