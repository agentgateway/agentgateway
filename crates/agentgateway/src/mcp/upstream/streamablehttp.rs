use ::http::Uri;
use ::http::header::CONTENT_TYPE;
use anyhow::anyhow;
use futures::StreamExt;
use reqwest::header::ACCEPT;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use sse_stream::SseStream;

use crate::http::Request;
use crate::mcp::ClientError;
use crate::mcp::upstream::IncomingRequestContext;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::SimpleBackend;
use crate::{json, *};

#[derive(Clone, Debug)]
pub struct Client {
	backend: Arc<SimpleBackend>,
	uri: Uri,
	client: PolicyClient,
	policies: BackendPolicies,
	session_id: AtomicOption<String>,
}

impl Client {
	pub fn new(
		backend: SimpleBackend,
		path: Strng,
		client: PolicyClient,
		policies: BackendPolicies,
	) -> anyhow::Result<Self> {
		let hp = backend.hostport();
		Ok(Self {
			backend: Arc::new(backend),
			uri: ("http://".to_string() + &hp + path.as_str()).parse()?,
			client,
			policies,
			session_id: Default::default(),
		})
	}
	pub fn set_session_id(&self, s: String) {
		self.session_id.store(Some(Arc::new(s)));
	}

	pub async fn send_request(
		&self,
		req: JsonRpcRequest<ClientRequest>,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::Request(req);
		self.send_message(message, ctx).await
	}
	pub async fn send_notification(
		&self,
		req: ClientNotification,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let message = ClientJsonRpcMessage::notification(req);
		self.send_message(message, ctx).await
	}
	async fn send_message(
		&self,
		message: ClientJsonRpcMessage,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let body = serde_json::to_vec(&message).map_err(ClientError::new)?;

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
			.body(body.into())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		if !resp.status().is_success() {
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
				let message = json::from_response_body::<ServerJsonRpcMessage>(resp)
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
	pub async fn send_delete(
		&self,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if !resp.status().is_success() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(StreamableHttpPostResponse::Accepted)
	}
	pub async fn get_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if !resp.status().is_success() {
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
			_ => Err(ClientError::new(anyhow!(
				"unexpected content type for GET streams: {:?}",
				content_type
			))),
		}
	}

	fn maybe_insert_session_id(&self, req: &mut Request) -> Result<(), ClientError> {
		if let Some(session_id) = self.session_id.load().clone() {
			req.headers_mut().insert(
				HEADER_SESSION_ID,
				session_id.as_ref().parse().map_err(ClientError::new)?,
			);
		}
		Ok(())
	}
}
