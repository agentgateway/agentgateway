use crate::http::Request;
use crate::json;
use crate::mcp::ClientError;
use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::UpstreamError;
use crate::mcp::upstream::stdio::Process;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::SimpleBackend;
use crate::*;
use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, Uri};
use anyhow::anyhow;
use futures_core::stream::BoxStream;
use futures_util::{StreamExt, TryFutureExt};
use reqwest::header::ACCEPT;
use rmcp::RoleClient;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::service::AtomicU32Provider;
use rmcp::transport::Transport;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::{SseError, StreamableHttpPostResponse};
use sse_stream::{Sse, SseStream};

type BoxedSseStream = BoxStream<'static, Result<Sse, SseError>>;

#[derive(Debug, Clone)]
struct ClientCore {
	backend: Arc<SimpleBackend>,
	uri: Uri,
	client: PolicyClient,
	policies: BackendPolicies,
}

#[derive(Debug)]
pub struct Client {
	client: ClientCore,

	active_stream: Arc<tokio::sync::Mutex<Option<Arc<super::stdio::Process>>>>,
}

struct SseClient {
	client: ClientCore,

	events: BoxedSseStream,
}

impl crate::mcp::upstream::stdio::MCPTransport for SseClient {
	async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
		// TODO: parse
		todo!()
		// self.events.next().await?.ok()
	}
	fn send(
		&mut self,
		item: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
	) -> impl Future<Output = Result<(), UpstreamError>> + Send + 'static {
		// let client = self.client.clone();
		// let uri = self.uri.clone();
		// self.send_message(item, user_headers).map_err(Into::into)
		async {
			todo!();
			Ok(())
		}
		// async move { client(uri, item, None).await }
	}
	async fn close(&mut self) -> Result<(), UpstreamError> {
		todo!()
	}
}

impl ClientCore {
	pub async fn send_message(
		&self,
		message: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
	) -> Result<(), ClientError> {
		Box::pin(self.internal_send_message(message, user_headers)).await
	}
	pub async fn send_message2(
		&self,
		req: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
	) -> Result<(), ClientError> {
		Box::pin(self.internal_send_message(req, user_headers)).await
	}
	fn internal_send_message<'a>(
		&'a self,
		req: ClientJsonRpcMessage,
		user_headers: &'a HeaderMap,
	) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>> {
		Box::pin(self.internal_send_message2(req, user_headers))
	}
	async fn internal_send_message2(
		&self,
		message: ClientJsonRpcMessage,
		user_headers: &HeaderMap,
	) -> Result<(), ClientError> {
		let client = self.client.clone();

		let body = serde_json::to_vec(&message).map_err(ClientError::new)?;

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.body(body.into())
			.map_err(ClientError::new)?;

		insert_user_headers(user_headers, &mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(())
	}
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
			client: ClientCore {
				backend: Arc::new(backend),
				uri: ("http://".to_string() + &hp + path.as_str())
					.parse()
					.expect("TODO"),
				policies,
				client,
			},
			active_stream: Default::default(),
		}
	}

	async fn get_stream(&self, user_headers: &HeaderMap) -> Result<Arc<Process>, UpstreamError> {
		let mut stream = self.active_stream.lock().await;
		if let Some(s) = stream.clone() {
			Ok(s)
		} else {
			let (post_uri, sse) = self.establish_sse(user_headers).await?;
			let transport = SseClient {
				client: ClientCore {
					uri: post_uri,
					..self.client.clone()
				},
				events: sse,
			};

			let proc = Arc::new(Process::new(transport));
			*stream = Some(proc.clone());
			Ok(proc)
		}
	}
	async fn establish_sse(
		&self,
		user_headers: &HeaderMap,
	) -> Result<(Uri, BoxedSseStream), ClientError> {
		let res = Box::pin(self.client.internal_establish_sse(user_headers)).await?;
		let mut s = match res {
			StreamableHttpPostResponse::Sse(s, _) => s,
			_ => return Err(ClientError::new(anyhow!("unexpected return typ"))),
		};
		let parsed = loop {
			let sse = futures_util::StreamExt::next(&mut s)
				.await
				.ok_or_else(|| ClientError::new(anyhow!("unexpected empty stream")))?
				.map_err(ClientError::new)?;
			let Some("endpoint") = sse.event.as_deref() else {
				continue;
			};
			let ep = sse.data.unwrap_or_default();
			let parsed = message_endpoint(self.client.uri.clone(), ep).map_err(ClientError::new)?;
			break parsed;
		};
		Ok((parsed, s))
	}
}
impl ClientCore {
	fn internal_establish_sse<'a>(
		&'a self,
		user_headers: &'a HeaderMap,
	) -> Pin<Box<dyn Future<Output = Result<StreamableHttpPostResponse, ClientError>> + Send + '_>> {
		Box::pin(self.internal_establish_sse2(user_headers))
	}
	async fn internal_establish_sse2(
		&self,
		user_headers: &HeaderMap,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let client = self.client.clone();

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.body(http::Body::empty())
			.map_err(ClientError::new)?;

		insert_user_headers(user_headers, &mut req);

		let resp = client
			.call_with_default_policies(req, &self.backend, self.policies.clone())
			.await
			.map_err(ClientError::new)?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Err(ClientError::new(anyhow!("expected an SSE stream")));
		}

		if resp.status().is_client_error() || resp.status().is_server_error() {
			return Err(ClientError::Status(Box::new(resp)));
		}

		let content_type = resp.headers().get(CONTENT_TYPE);

		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, None))
			},
			_ => Err(ClientError::new(anyhow!(
				"unexpected content type: {:?}",
				content_type
			))),
		}
	}
}
impl Client {
	pub async fn connect_to_event_stream(
		&self,
		user_headers: &HeaderMap,
	) -> Result<Messages, UpstreamError> {
		let stream = self.get_stream(user_headers).await?;
		Ok(stream.get_event_stream().await)
	}
	pub async fn send_message(
		&self,
		req: JsonRpcRequest<ClientRequest>,
		user_headers: &HeaderMap,
	) -> Result<ServerJsonRpcMessage, UpstreamError> {
		let stream = self.get_stream(user_headers).await?;
		stream.send_message(req, user_headers).await
	}

	pub async fn send_notification(
		&self,
		req: ClientNotification,
		user_headers: &HeaderMap,
	) -> Result<(), UpstreamError> {
		let stream = self.get_stream(user_headers).await?;
		stream.send_notification(req, user_headers).await
	}
}
impl ClientCore {
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

		insert_user_headers(user_headers, &mut req);

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

		insert_user_headers(user_headers, &mut req);

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

fn message_endpoint(base: Uri, endpoint: String) -> Result<Uri, http::uri::InvalidUri> {
	// If endpoint is a full URL, parse and return it directly
	if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
		return endpoint.parse::<Uri>();
	}

	let mut base_parts = base.into_parts();
	let endpoint_clone = endpoint.clone();

	if endpoint.starts_with("?") {
		// Query only - keep base path and append query
		if let Some(base_path_and_query) = &base_parts.path_and_query {
			let base_path = base_path_and_query.path();
			base_parts.path_and_query = Some(format!("{}{}", base_path, endpoint).parse()?);
		} else {
			base_parts.path_and_query = Some(format!("/{}", endpoint).parse()?);
		}
	} else {
		// Path (with optional query) - replace entire path_and_query
		let path_to_use = if endpoint.starts_with("/") {
			endpoint // Use absolute path as-is
		} else {
			format!("/{}", endpoint) // Make relative path absolute
		};
		base_parts.path_and_query = Some(path_to_use.parse()?);
	}

	Uri::from_parts(base_parts).map_err(|_| endpoint_clone.parse::<Uri>().unwrap_err())
}
