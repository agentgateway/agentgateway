use std::sync::Arc;

use crate::http::{Request, Response};
use crate::mcp::handler::RelayInputs;
use crate::mcp::session::{Session, SessionManager, dropper};
use crate::telemetry::log::AsyncLog;
use crate::*;
use ::http::StatusCode;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ConstString, ServerJsonRpcMessage};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};

use crate::proxy::ProxyError;

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
			(http::Method::POST, _) => self.handle_post(request, inputs).await,
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

		if !self.config.stateful_mode {
			let relay = inputs.build_new_connections()?;
			// Use stateless session - not registered in session manager
			let mut session = self
				.session_manager
				.create_stateless_session(relay)
				.map_err(|error| mcp::Error::StartSession(error.to_string()))?;
			let cleanup = dropper(self.session_manager.clone(), session.clone(), part.clone());
			let response = session
				.stateless_send_and_initialize(part.clone(), message)
				.await;
			cleanup.cleanup().await;
			return response;
		}

		let session_id = part
			.headers
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok());

		if let Some(session_id) = session_id {
			let method_name = message_method_name(&message);
			let mut session = self.resolve_request_session(
				session_id,
				inputs,
				&part.extensions,
				method_name.as_deref(),
			)?;

			return session.send(part, message).await;
		}

		// No session header... we need to create one, if it is an initialize
		if let ClientJsonRpcMessage::Request(req) = &message
			&& !matches!(req.request, ClientRequest::InitializeRequest(_))
		{
			return mcp::Error::MissingSessionHeader.into();
		}
		let relay = inputs.build_new_connections()?;
		let mut session = self
			.session_manager
			.create_session(relay)
			.map_err(|error| mcp::Error::StartSession(error.to_string()))?;
		let mut resp = session.send(part, message).await?;

		let Ok(sid) = session.id.parse() else {
			return mcp::Error::InvalidSessionIdHeader.into();
		};
		resp.headers_mut().insert(HEADER_SESSION_ID, sid);
		self.session_manager.insert_session(session);
		Ok(resp)
	}

	pub async fn handle_get(
		&self,
		request: Request,
		inputs: RelayInputs,
	) -> Result<Response, ProxyError> {
		// check accept header
		if !request
			.headers()
			.get(http::header::ACCEPT)
			.and_then(|header| header.to_str().ok())
			.is_some_and(|header| header.contains(EVENT_STREAM_MIME_TYPE))
		{
			return mcp::Error::InvalidAccept.into();
		}

		let Some(session_id) = request
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
		else {
			return mcp::Error::SessionIdRequired.into();
		};

		let session = self.resolve_request_session(session_id, inputs, request.extensions(), None)?;

		let (parts, _) = request.into_parts();
		session.get_stream(parts).await
	}

	pub async fn handle_delete(&self, request: Request) -> Result<Response, ProxyError> {
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

	fn resolve_request_session(
		&self,
		session_id: &str,
		inputs: RelayInputs,
		extensions: &::http::Extensions,
		method_name: Option<&str>,
	) -> Result<Session, ProxyError> {
		match self.session_manager.resolve_session(session_id, inputs) {
			Ok(session) => Ok(session),
			Err(reason) => {
				record_resume_failure(extensions, session_id, method_name, reason);
				match reason {
					mcp::ResumeFailureReason::MalformedHandle => mcp::Error::InvalidSessionIdHeader,
					_ => mcp::Error::UnknownSession,
				}
				.into()
			},
		}
	}
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}

fn message_method_name(message: &ClientJsonRpcMessage) -> Option<String> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.request.method().to_string()),
		ClientJsonRpcMessage::Notification(notification) => Some(match &notification.notification {
			rmcp::model::ClientNotification::CancelledNotification(r) => r.method.as_str().to_string(),
			rmcp::model::ClientNotification::ProgressNotification(r) => r.method.as_str().to_string(),
			rmcp::model::ClientNotification::InitializedNotification(r) => r.method.as_str().to_string(),
			rmcp::model::ClientNotification::RootsListChangedNotification(r) => {
				r.method.as_str().to_string()
			},
			rmcp::model::ClientNotification::CustomNotification(r) => r.method.to_string(),
		}),
		_ => None,
	}
}

fn record_resume_failure(
	extensions: &::http::Extensions,
	session_id: &str,
	method_name: Option<&str>,
	reason: mcp::ResumeFailureReason,
) {
	let Some(log) = extensions.get::<AsyncLog<mcp::MCPInfo>>() else {
		return;
	};
	log.non_atomic_mutate(|mcp: &mut mcp::MCPInfo| {
		mcp.session_id = Some(session_id.to_string());
		if mcp.method_name.is_none() {
			mcp.method_name = method_name.map(ToOwned::to_owned);
		}
		mcp.resume_failure_reason = Some(reason);
	});
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;
	use std::path::Path;
	use std::sync::Arc;

	use super::*;
	use crate::http::sessionpersistence::{
		Encoder, MCPSession, MCPSnapshotMember, MCPSnapshotRouting, MCPSnapshotState, SessionState,
	};
	use crate::mcp::router::{McpBackendGroup, McpTarget};
	use crate::mcp::{MCPInfo, McpAuthorizationSet, ResumeFailureReason};
	use crate::proxy::httpproxy::PolicyClient;
	use crate::telemetry::log::AsyncLog;
	use crate::types::agent::McpTargetSpec;
	use rmcp::model::{
		ClientCapabilities, ClientInfo, Implementation, InitializeRequest, PromptsCapability,
		ProtocolVersion, RequestId, ServerCapabilities, ServerInfo, ToolsCapability,
	};
	use tempfile::NamedTempFile;

	fn capture_target(name: &str, capture_file: &Path) -> Arc<McpTarget> {
		Arc::new(McpTarget {
			name: name.into(),
			spec: McpTargetSpec::Stdio {
				cmd: "sh".into(),
				args: vec![
					"-c".into(),
					"while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$CAPTURE_FILE\"; done".into(),
				],
				env: HashMap::from([(
					"CAPTURE_FILE".to_string(),
					capture_file.display().to_string(),
				)]),
			},
			backend: None,
			always_use_prefix: false,
			backend_policies: Default::default(),
		})
	}

	fn relay_inputs(
		targets: Vec<Arc<McpTarget>>,
		allow_degraded: bool,
	) -> crate::mcp::handler::RelayInputs {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		crate::mcp::handler::RelayInputs {
			backend: McpBackendGroup {
				targets,
				stateful: true,
				allow_degraded,
				allow_insecure_multiplex: false,
			},
			policies: McpAuthorizationSet::new(vec![].into()),
			client: PolicyClient {
				inputs: test.inputs(),
			},
		}
	}

	fn encode_snapshot(
		session_encoder: &Encoder,
		members: Vec<MCPSnapshotMember>,
		routing: MCPSnapshotRouting,
	) -> String {
		SessionState::MCPSnapshot(MCPSnapshotState::new(members, routing))
			.encode(session_encoder)
			.expect("snapshot should encode")
	}

	fn server_info(has_tools: bool, has_prompts: bool) -> ServerInfo {
		let mut capabilities = ServerCapabilities::default();
		capabilities.tools = has_tools.then_some(ToolsCapability::default());
		capabilities.prompts = has_prompts.then_some(PromptsCapability::default());
		ServerInfo::new(capabilities).with_protocol_version(ProtocolVersion::V_2025_06_18)
	}

	fn snapshot_member(
		target: &Arc<McpTarget>,
		session: MCPSession,
		info: ServerInfo,
	) -> MCPSnapshotMember {
		MCPSnapshotMember::new(
			target.name.to_string(),
			session,
			info,
			target
				.snapshot_fingerprint()
				.expect("target fingerprint should be serializable"),
		)
	}

	fn empty_parts() -> ::http::request::Parts {
		::http::Request::builder()
			.uri("/mcp")
			.body(crate::http::Body::empty())
			.expect("request should build")
			.into_parts()
			.0
	}

	#[tokio::test]
	async fn handle_get_recovers_snapshot_session() {
		let capture = NamedTempFile::new().expect("temp file");
		let target = capture_target("serverA", capture.path());
		let encoder = Encoder::base64();
		let session_manager = Arc::new(SessionManager::new(encoder.clone()));
		let service = StreamableHttpService::new(
			session_manager.clone(),
			StreamableHttpServerConfig {
				stateful_mode: true,
			},
		);
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&target,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(false, false),
			)],
			MCPSnapshotRouting {
				default_target_name: Some("serverA".to_string()),
				is_multiplexing: false,
			},
		);
		let request = ::http::Request::builder()
			.uri("/mcp")
			.header(http::header::ACCEPT, EVENT_STREAM_MIME_TYPE)
			.header(HEADER_SESSION_ID, &session_id)
			.body(crate::http::Body::empty())
			.expect("request should build");

		let response = service
			.handle_get(request, relay_inputs(vec![target], true))
			.await
			.expect("GET should recover a snapshot-backed session");

		assert_eq!(response.status(), http::StatusCode::OK);
		assert!(
			response
				.headers()
				.get(http::header::CONTENT_TYPE)
				.and_then(|value| value.to_str().ok())
				.is_some_and(|value| value.starts_with(EVENT_STREAM_MIME_TYPE))
		);

		drop(response);
		let _ = session_manager
			.delete_session(&session_id, empty_parts())
			.await;
	}

	#[tokio::test]
	async fn handle_get_treats_stale_instance_ref_as_unknown_session() {
		let capture = NamedTempFile::new().expect("temp file");
		let target = capture_target("serverA", capture.path());
		let encoder = Encoder::base64();
		let origin_manager = Arc::new(SessionManager::new(encoder.clone()));
		let (session, _rx) = origin_manager
			.create_legacy_session(
				relay_inputs(vec![target.clone()], true)
					.build_new_connections()
					.expect("relay should build"),
			)
			.expect("legacy session handle should mint");
		let service = StreamableHttpService::new(
			Arc::new(SessionManager::new(encoder)),
			StreamableHttpServerConfig {
				stateful_mode: true,
			},
		);
		let log = AsyncLog::<MCPInfo>::default();
		log.store(Some(MCPInfo::default()));
		let mut request = ::http::Request::builder()
			.uri("/mcp")
			.header(http::header::ACCEPT, EVENT_STREAM_MIME_TYPE)
			.header(HEADER_SESSION_ID, session.id.as_ref())
			.body(crate::http::Body::empty())
			.expect("request should build");
		request.extensions_mut().insert(log.clone());

		let err = service
			.handle_get(request, relay_inputs(vec![target], true))
			.await
			.expect_err("stale instance-bound handle should fail");

		assert!(matches!(err, ProxyError::MCP(mcp::Error::UnknownSession)));

		let info = log.take().expect("resume failure should be logged");
		assert_eq!(info.session_id.as_deref(), Some(session.id.as_ref()));
		assert_eq!(
			info.resume_failure_reason,
			Some(ResumeFailureReason::LiveSessionMissing)
		);
		assert_eq!(info.method_name, None);
	}

	#[tokio::test]
	async fn handle_post_records_malformed_resume_failure_context() {
		let encoder = Encoder::base64();
		let session_manager = Arc::new(SessionManager::new(encoder));
		let service = StreamableHttpService::new(
			session_manager,
			StreamableHttpServerConfig {
				stateful_mode: true,
			},
		);
		let log = AsyncLog::<MCPInfo>::default();
		log.store(Some(MCPInfo::default()));
		let initialize = ClientJsonRpcMessage::request(
			InitializeRequest::new(ClientInfo::new(
				ClientCapabilities::default(),
				Implementation::new("test-client", "1.0.0"),
			))
			.into(),
			RequestId::Number(1),
		);
		let mut request = ::http::Request::builder()
			.uri("/mcp")
			.header(
				http::header::ACCEPT,
				format!("{JSON_MIME_TYPE}, {EVENT_STREAM_MIME_TYPE}"),
			)
			.header(http::header::CONTENT_TYPE, JSON_MIME_TYPE)
			.header(HEADER_SESSION_ID, "definitely-not-a-session-handle")
			.body(crate::http::Body::from(
				serde_json::to_vec(&initialize).expect("initialize should serialize"),
			))
			.expect("request should build");
		request.extensions_mut().insert(log.clone());

		let err = service
			.handle_post(request, relay_inputs(Vec::new(), true))
			.await
			.expect_err("malformed handle should fail before routing");

		assert!(matches!(
			err,
			ProxyError::MCP(mcp::Error::InvalidSessionIdHeader)
		));

		let info = log.take().expect("resume failure should be logged");
		assert_eq!(
			info.session_id.as_deref(),
			Some("definitely-not-a-session-handle")
		);
		assert_eq!(
			info.resume_failure_reason,
			Some(ResumeFailureReason::MalformedHandle)
		);
		assert_eq!(info.method_name.as_deref(), Some("initialize"));
	}
}
