use super::*;
use crate::http::Error as HttpError;
use crate::mcp::sse::McpTarget;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::{McpTargetSpec, SimpleBackend};
use crate::{ProxyInputs, json};
use agent_core::prelude::*;
use anyhow::anyhow;
use axum_core::BoxError;
use futures::StreamExt;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Uri};
use indexmap::IndexMap;
use reqwest::header::ACCEPT;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::service::AtomicU32Provider;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use sse_stream::SseStream;
use thiserror::Error;

#[derive(Debug)]
pub(crate) struct ConnectionPool {
	pi: Arc<ProxyInputs>,
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: IndexMap<Strng, Arc<upstream::Upstream>>,
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
			by_name: IndexMap::new(),
			stateful,
		};
		s.setup_connections()?;
		Ok(s)
	}

	pub(crate) fn setup_connections(&mut self) -> anyhow::Result<()> {
		for tgt in &self.backend.targets {
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
			McpTargetSpec::Sse(_) => {
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

				upstream::Upstream::McpHttp(client)
			},
			McpTargetSpec::Stdio { .. } => {
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

#[derive(Clone, Debug)]
pub struct ClientWrapper {
	backend: Arc<SimpleBackend>,
	uri: Uri,
	idp: Arc<AtomicU32Provider>,
	client: PolicyClient,
	policies: BackendPolicies,
	session_id: AtomicOption<String>,
}

#[derive(Error, Debug)]
pub enum ClientError {
	#[error("http request failed with code: {}", .0.status())]
	Status(Box<crate::http::Response>),
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
				Err(ClientError::new(anyhow!("unexpected 'json' response")))
			},
			StreamableHttpPostResponse::Sse(_, _) => {
				Err(ClientError::new(anyhow!("unexpected 'sse' response")))
			},
		}
	}

	pub async fn send_message(
		&self,
		req: JsonRpcRequest<ClientRequest>,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::Request(req);
		Box::pin(self.internal_send_message(message, user_headers)).await
	}

	pub async fn send_notification(
		&self,
		req: ClientNotification,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::notification(req);
		Box::pin(self.internal_send_message(message, user_headers)).await
	}
	pub async fn send_delete(
		&self,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		self.internal_delete(user_headers).await
	}
	pub async fn get_event_stream(
		&self,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		self.internal_get_event_stream(user_headers).await
	}
	pub async fn send_message2(
		&self,
		req: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		Box::pin(self.internal_send_message(req, user_headers)).await
	}
	fn internal_send_message<'a>(
		&'a self,
		req: ClientJsonRpcMessage,
		user_headers: &'a HeaderMap,
	) -> Pin<Box<dyn Future<Output = Result<StreamableHttpPostResponse, ClientError>> + Send + '_>> {
		Box::pin(self.internal_send_message2(req, user_headers))
	}
	async fn internal_send_message2(
		&self,
		message: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
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

		for (k, v) in user_headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING || k == http::header::CONTENT_LENGTH {
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
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
	fn internal_delete<'a>(
		&'a self,
		user_headers: &'a HeaderMap,
	) -> Pin<Box<dyn Future<Output = Result<StreamableHttpPostResponse, ClientError>> + Send + '_>> {
		Box::pin(self.internal_delete2(user_headers))
	}
	fn internal_get_event_stream<'a>(
		&'a self,
		user_headers: &'a HeaderMap,
	) -> Pin<Box<dyn Future<Output = Result<StreamableHttpPostResponse, ClientError>> + Send + '_>> {
		Box::pin(self.internal_get_event_stream2(user_headers))
	}
	async fn internal_delete2(
		&self,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let mut req = http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		if let Some(session_id) = self.session_id.load().clone() {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id.as_ref().parse().map_err(ClientError::new)?,
			);
		}

		for (k, v) in user_headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING || k == http::header::CONTENT_LENGTH {
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(StreamableHttpPostResponse::Accepted)
	}
	async fn internal_get_event_stream2(
		&self,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let mut req = http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		if let Some(session_id) = self.session_id.load().clone() {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id.as_ref().parse().map_err(ClientError::new)?,
			);
		}

		for (k, v) in user_headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING || k == http::header::CONTENT_LENGTH {
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(StreamableHttpPostResponse::Accepted)
	}
}
