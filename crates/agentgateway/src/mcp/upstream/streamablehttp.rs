use crate::http::{Error as HttpError, Request};
use crate::json;
use crate::mcp::ClientError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::SimpleBackend;
use crate::*;
use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, Uri};
use agent_core::prelude::*;
use anyhow::anyhow;
use axum_core::BoxError;
use futures::StreamExt;
use reqwest::header::ACCEPT;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::service::AtomicU32Provider;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use sse_stream::SseStream;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct Client {
	backend: Arc<SimpleBackend>,
	uri: Uri,
	idp: Arc<AtomicU32Provider>,
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

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
			.body(body.into())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		Self::insert_user_headers(user_headers, &mut req);

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

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		Self::insert_user_headers(user_headers, &mut req);

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

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		Self::insert_user_headers(user_headers, &mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(StreamableHttpPostResponse::Accepted)
	}

	fn insert_user_headers(user_headers: &HeaderMap, req: &mut Request) {
		for (k, v) in user_headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING || k == http::header::CONTENT_LENGTH {
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
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
