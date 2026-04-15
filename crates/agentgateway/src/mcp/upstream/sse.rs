use ::http::Uri;
use ::http::header::{ACCEPT, CONTENT_TYPE};
use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::{StreamExt, TryFutureExt};
use headers::HeaderMapExt;
use parking_lot::Mutex;
use pin_project_lite::pin_project;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::EVENT_STREAM_MIME_TYPE;
use sse_stream::{Sse, SseStream};

use crate::client::ResolvedDestination;
use crate::mcp::ClientError;
use crate::mcp::mergestream::Messages;
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
use crate::mcp::upstream::stdio::Process;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::*;

type BoxedSseStream = BoxStream<'static, Result<Sse, sse_stream::Error>>;

#[derive(Debug, Clone)]
struct ClientCore {
	http_client: super::McpHttpClient,
	uri: Uri,
}

#[derive(Debug)]
pub struct Client {
	client: ClientCore,

	active_stream: Arc<Mutex<Option<std::sync::Weak<super::stdio::Process>>>>,
}

struct SseClient {
	client: ClientCore,

	events: BoxedSseStream,
}

pin_project! {
	struct GuardedMessages {
		#[pin]
		inner: Messages,
		_guard: Arc<Process>,
	}
}

impl Stream for GuardedMessages {
	type Item = Result<ServerJsonRpcMessage, ClientError>;

	fn poll_next(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Option<Self::Item>> {
		self.project().inner.poll_next(cx)
	}
}

impl crate::mcp::upstream::stdio::MCPTransport for SseClient {
	async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
		loop {
			let raw = self.events.next().await?.ok()?;
			let Some(data) = raw.data else {
				continue;
			};
			if data.is_empty() {
				continue;
			}
			match serde_json::from_str::<ServerJsonRpcMessage>(&data) {
				Err(e) => {
					// Not a hard error, for now?
					tracing::warn!("failed to deserialize server message: {e}");
					continue;
				},
				Ok(message) => {
					return Some(message);
				},
			};
		}
	}
	fn send(
		&mut self,
		item: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> impl Future<Output = Result<(), UpstreamError>> + Send + 'static {
		let ctx = ctx.clone();
		let client = self.client.clone();
		Box::pin(async move { client.send_message(item, &ctx).map_err(Into::into).await })
	}
	async fn close(&mut self) -> Result<(), UpstreamError> {
		Ok(())
	}
}

impl ClientCore {
	async fn send_message(
		&self,
		message: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), ClientError> {
		let body = serde_json::to_vec(&message).map_err(ClientError::new)?;

		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::POST)
			.header(CONTENT_TYPE, "application/json")
			.body(body.into())
			.map_err(ClientError::new)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await.map_err(ClientError::new)?;

		if !resp.status().is_success() {
			return Err(ClientError::Status(Box::new(resp)));
		}
		Ok(())
	}
}

impl ClientCore {
	async fn establish_sse(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.body(http::Body::empty())
			.map_err(ClientError::new)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await?;

		if resp.status() == http::StatusCode::ACCEPTED {
			return Err(ClientError::new(anyhow!("expected an SSE stream")));
		}

		if !resp.status().is_success() {
			return Err(ClientError::Status(Box::new(resp)));
		}

		let content_type = resp.headers().get(CONTENT_TYPE);

		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let content_encoding = resp.headers().typed_get::<headers::ContentEncoding>();
				let (body, _encoding) =
					crate::http::compression::decompress_body(resp.into_body(), content_encoding.as_ref())
						.map_err(ClientError::new)?;
				let event_stream = SseStream::from_byte_stream(body.into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, None))
			},
			_ => Err(ClientError::new(anyhow!(
				"establish sse: unexpected content type: {:?}",
				content_type
			))),
		}
	}
}
impl Client {
	pub fn new(http_client: super::McpHttpClient, path: Strng) -> Self {
		let hp = http_client.backend().hostport();
		let uri = format!("http://{}{}", hp, path);
		let uri = uri.parse().expect("invalid URI");
		Self {
			client: ClientCore { http_client, uri },
			active_stream: Default::default(),
		}
	}

	pub fn get_session_state(&self) -> http::sessionpersistence::MCPSession {
		http::sessionpersistence::MCPSession {
			target_name: Some(self.client.http_client.target_name().to_string()),
			session: None,
			backend: self.client.http_client.pinned_backend(),
		}
	}

	pub fn set_session_id(&self, _: Option<&str>, pinned: Option<SocketAddr>) {
		if let Some(pinned) = pinned {
			self
				.client
				.http_client
				.pin_backend(ResolvedDestination(pinned));
		}
	}

	pub async fn stop(&self) -> Result<(), UpstreamError> {
		let stream = self
			.active_stream
			.lock()
			.take()
			.and_then(|stream| stream.upgrade());
		if let Some(s) = stream {
			s.stop().await?;
		}
		Ok(())
	}
	async fn get_stream(&self, ctx: &IncomingRequestContext) -> Result<Arc<Process>, UpstreamError> {
		if let Some(stream) = self
			.active_stream
			.lock()
			.as_ref()
			.and_then(std::sync::Weak::upgrade)
		{
			return Ok(stream);
		}

		let (post_uri, sse) = self.establish_sse(ctx).await?;
		let transport = SseClient {
			client: ClientCore {
				uri: post_uri,
				..self.client.clone()
			},
			events: sse,
		};

		let proc = Arc::new(Process::new(transport));
		let mut stream = self.active_stream.lock();
		if let Some(existing) = stream.as_ref().and_then(std::sync::Weak::upgrade) {
			return Ok(existing);
		}
		*stream = Some(Arc::downgrade(&proc));
		Ok(proc)
	}
	async fn establish_sse(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<(Uri, BoxedSseStream), ClientError> {
		let res = Box::pin(self.client.establish_sse(ctx)).await?;
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
	pub async fn connect_to_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<Messages, UpstreamError> {
		let stream = self.get_stream(ctx).await?;
		let messages = stream.get_event_stream().await?;
		// We want to drop the Process handle when the stream is done, so that we can close it if we are
		// the last reference.
		Ok(Messages::boxed(
			GuardedMessages {
				inner: messages,
				_guard: stream,
			}
			.boxed(),
		))
	}
	pub async fn send_message(
		&self,
		req: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<ServerJsonRpcMessage, UpstreamError> {
		let stream = self.get_stream(ctx).await?;
		stream.send_message(req, ctx).await
	}

	pub async fn send_notification(
		&self,
		req: ClientNotification,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		let stream = self.get_stream(ctx).await?;
		stream.send_notification(req, ctx).await
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
