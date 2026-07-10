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
use crate::mcp::{REMOVED_METHODS_2026_07_28, is_modern_version};
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
		self.version.as_ref().is_some_and(is_modern_version)
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
		let bytes = match http::read_body_with_limit(body, limit).await {
			Ok(b) => b,
			Err(e) => return mcp::Error::Deserialize(e).into(),
		};
		let message = match serde_json::from_slice::<ClientJsonRpcMessage>(&bytes) {
			Ok(m) => m,
			Err(e) => {
				return match unknown_method_error(&part.headers, &bytes) {
					Some(err) => err.into(),
					None => mcp::Error::Deserialize(http::Error::new(e)).into(),
				};
			},
		};
		// Raw body is only needed for the `unknown_method_error` recovery above; release it now
		// so the buffer is not pinned across the upstream round-trip below.
		drop(bytes);
		let request_id = request_id(&message);
		let protocol = validate_request_protocol(&part.headers, &message, request_id.clone())?;
		validate_standard_headers(&part.headers, &message, &protocol)?;
		part.extensions.insert(protocol.clone());

		if !self.config.stateful_mode {
			return self.serve_stateless(inputs, part, message, protocol).await;
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
			return self.serve_stateless(inputs, part, message, protocol).await;
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

	async fn serve_stateless(
		&self,
		inputs: RelayInputs,
		part: ::http::request::Parts,
		message: ClientJsonRpcMessage,
		protocol: RequestProtocol,
	) -> Result<Response, ProxyError> {
		let relay = inputs.build_new_connections()?;
		// Use stateless session - not registered in session manager
		let mut session = self.session_manager.create_stateless_session(relay);
		let initialize_upstream = protocol.uses_sessions();
		let needs_cleanup = initialize_upstream || session.has_connection_teardown();
		// Teardown is needed when the synthetic upstream initialize may open upstream sessions,
		// or when stdio/SSE targets hold per-connection state. Modern requests (no synthetic
		// initialize) against plain streamable/OpenAPI targets have nothing to clean up.
		if !needs_cleanup {
			return Box::pin(session.stateless_send_and_initialize(part, message, initialize_upstream))
				.await;
		}
		let cleanup_part = part.clone();
		let response =
			Box::pin(session.stateless_send_and_initialize(part, message, initialize_upstream)).await;

		let (tx, rx) = tokio::sync::oneshot::channel::<()>();
		tokio::task::spawn(async move {
			// Wait until the response is actually completed.
			let _ = rx.await;
			trace!("cleaning up stateless session");
			// Clean up upstream resources (e.g., stdio processes)
			let _ = session.delete_session(cleanup_part).await;
		});
		response.map(|r| r.map(|b| DropBody::new(b, tx)))
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
	include_supported_versions: bool,
) -> Result<Option<ProtocolVersion>, ProxyError> {
	let Some(value) = headers.get(HEADER_MCP_PROTOCOL_VERSION) else {
		return Ok(None);
	};
	let value = value
		.to_str()
		.map_err(|_| ProxyError::MCP(mcp::Error::InvalidProtocolVersion))?;
	// This is the gateway-owned version set used by the transport gate and version errors.
	let version = ProtocolVersion::KNOWN_VERSIONS
		.iter()
		.find(|version| version.as_str() == value)
		.cloned()
		.ok_or_else(|| {
			ProxyError::MCP(mcp::Error::UnsupportedVersion {
				request_id,
				version: value.to_string(),
				include_supported_versions,
			})
		})?;
	Ok(Some(version))
}

/// Validates a POST request's protocol version and gateway-owned modern method routing.
/// The protocol header is parsed once here.
///
/// Keep the statement order. Removed-method rejection must run before header/body version
/// reconciliation because removed methods must return 404 even when their params do not parse.
fn validate_request_protocol(
	headers: &::http::HeaderMap,
	message: &ClientJsonRpcMessage,
	request_id: Option<RequestId>,
) -> Result<RequestProtocol, ProxyError> {
	let is_initialize = matches!(
		message,
		ClientJsonRpcMessage::Request(request)
			if matches!(request.request, ClientRequest::InitializeRequest(_))
	);
	let header_version = protocol_version_header(headers, request_id.clone(), !is_initialize)?;
	let body_version = message_protocol_version(message);

	// This check uses only the header version because modern clients must send it and the
	// body version is reconciled later.
	if header_version.as_ref().is_some_and(is_modern_version)
		&& let ClientJsonRpcMessage::Request(req) = message
	{
		let method = req.request.method();
		// rmcp's untagged parse puts unknown methods and known methods with invalid params in
		// `CustomRequest`. Typed variants are known by construction, so this check reserves 404 for
		// unknown methods and lets dispatch return -32602 for invalid params.
		if REMOVED_METHODS_2026_07_28.contains(&method)
			|| (matches!(req.request, ClientRequest::CustomRequest(_))
				&& !mcp::is_known_client_request_method(method))
		{
			return Err(mcp::Error::MethodNotFound(request_id, method.to_string()).into());
		}
		if body_version.is_none() {
			return Err(
				mcp::Error::InvalidParams(
					request_id,
					"_meta.protocolVersion is required for modern requests".to_string(),
				)
				.into(),
			);
		}
	}

	if let (Some(header), Some(body)) = (&header_version, &body_version)
		&& header != body
	{
		return Err(mcp::Error::VersionMismatch(request_id).into());
	}

	// Completeness checks header or body. A body-only modern version is still a
	// modern request, but it is missing the required protocol header.
	let declares_modern_version = header_version
		.as_ref()
		.or(body_version.as_ref())
		.is_some_and(is_modern_version);
	if declares_modern_version && (header_version.is_none() || body_version.is_none()) {
		return Err(mcp::Error::InvalidProtocolVersion.into());
	}

	Ok(RequestProtocol {
		version: body_version.or(header_version),
	})
}

/// Recovers a `MethodNotFound` for modern request bodies that fail the typed
/// `ClientJsonRpcMessage` parse (e.g. non-object `params`) but name an unknown method.
/// Parseable unknown methods get the same 404 from `validate_request_protocol`; this
/// fallback only classifies bodies the typed parse cannot represent.
/// Header-only modern detection: body `_meta` is unreadable once the typed parse has failed.
fn unknown_method_error(headers: &::http::HeaderMap, bytes: &[u8]) -> Option<mcp::Error> {
	if !protocol_version_header(headers, None, true)
		.ok()?
		.is_some_and(|v| is_modern_version(&v))
	{
		return None;
	}
	let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
	if value.get("jsonrpc")?.as_str()? != "2.0" {
		return None;
	}
	let method = value.get("method")?.as_str()?.to_string();
	if mcp::is_known_client_request_method(&method) {
		return None;
	}
	let request_id: RequestId = serde_json::from_value(value.get("id")?.clone()).ok()?;
	Some(mcp::Error::MethodNotFound(Some(request_id), method))
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
	if let Some(version) = protocol_version_header(headers, None, true)?
		&& is_modern_version(&version)
	{
		return Err(
			mcp::Error::UnsupportedVersion {
				request_id: None,
				version: version.to_string(),
				include_supported_versions: true,
			}
			.into(),
		);
	}
	Ok(())
}
