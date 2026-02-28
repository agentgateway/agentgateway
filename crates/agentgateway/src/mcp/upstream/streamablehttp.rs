use ::http::Uri;
use ::http::header::{ACCEPT, CONTENT_TYPE};
use anyhow::anyhow;
use futures::StreamExt;
use headers::HeaderMapExt;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcRequest, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use sse_stream::SseStream;

use crate::client::ResolvedDestination;
use crate::http::Request;
use crate::mcp::ClientError;
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
use crate::mcp::upstream::IncomingRequestContext;
use crate::*;

#[derive(Clone, Debug)]
pub struct Client {
	http_client: super::McpHttpClient,
	uri: Uri,
	session_id: AtomicOption<String>,
}

impl Client {
	pub fn new(http_client: super::McpHttpClient, path: Strng) -> anyhow::Result<Self> {
		let hp = http_client.backend().hostport();
		Ok(Self {
			http_client,
			uri: ("http://".to_string() + &hp + path.as_str()).parse()?,
			session_id: Default::default(),
		})
	}

	pub fn get_session_state(&self) -> http::sessionpersistence::MCPSession {
		let session_id = self.session_id.load().clone();
		let backend = self.http_client.pinned_backend();
		http::sessionpersistence::MCPSession {
			session: session_id.map(|s| s.to_string()),
			backend,
		}
	}

	pub fn set_session_id(&self, s: Option<&str>, pinned: Option<SocketAddr>) {
		self.session_id.store(s.map(|s| Arc::new(s.to_string())));
		if let Some(pinned) = pinned {
			self.http_client.pin_backend(ResolvedDestination(pinned));
		}
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
		let response = self.send_message(message, ctx).await?;
		Self::reject_jsonrpc_error("notification", response)
	}
	pub async fn send_client_message(
		&self,
		message: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), ClientError> {
		let response = self.send_message(message, ctx).await?;
		let _ = Self::reject_jsonrpc_error("client message", response)?;
		Ok(())
	}
	fn reject_jsonrpc_error(
		operation: &str,
		response: StreamableHttpPostResponse,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		match response {
			StreamableHttpPostResponse::Json(ServerJsonRpcMessage::Error(err), _) => {
				Err(ClientError::new(anyhow!(
					"{operation}: upstream returned JSON-RPC error {:?}: {}",
					err.error.code,
					err.error.message
				)))
			},
			other => Ok(other),
		}
	}
	async fn send_message(
		&self,
		message: ClientJsonRpcMessage,

		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
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

		let resp = self.http_client.call(req).await?;

		// MCP spec has 202 only but some servers in the wild return 204. This is close enough for us to massage it.
		if resp.status() == http::StatusCode::ACCEPTED || resp.status() == http::StatusCode::NO_CONTENT
		{
			return Ok(StreamableHttpPostResponse::Accepted);
		}

		let session_id = resp
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
			.map(|s| s.to_string());
		let content_type = resp.headers().get(CONTENT_TYPE);

		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let content_encoding = resp.headers().typed_get::<headers::ContentEncoding>();
				let (body, _encoding) =
					crate::http::compression::decompress_body(resp.into_body(), content_encoding.as_ref())
						.map_err(ClientError::new)?;
				let event_stream = SseStream::from_byte_stream(body.into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, session_id))
			},
			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
				let lim = crate::http::response_buffer_limit(&resp);
				let content_encoding = resp.headers().typed_get::<headers::ContentEncoding>();
				let body_bytes = crate::http::compression::to_bytes_with_decompression(
					resp.into_body(),
					content_encoding.as_ref(),
					lim,
				)
				.await
				.map_err(ClientError::new)?
				.1;
				let message =
					serde_json::from_slice::<ServerJsonRpcMessage>(&body_bytes).map_err(ClientError::new)?;
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
		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::DELETE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		self.http_client.call(req).await?;

		Ok(StreamableHttpPostResponse::Accepted)
	}
	pub async fn get_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<StreamableHttpPostResponse, ClientError> {
		let mut req = ::http::Request::builder()
			.uri(&self.uri)
			.method(http::Method::GET)
			.header(ACCEPT, EVENT_STREAM_MIME_TYPE)
			.body(crate::http::Body::empty())
			.map_err(ClientError::new)?;

		self.maybe_insert_session_id(&mut req)?;

		ctx.apply(&mut req);

		let resp = self.http_client.call(req).await?;

		let content_type = resp.headers().get(CONTENT_TYPE);
		let session_id = resp
			.headers()
			.get(HEADER_SESSION_ID)
			.and_then(|v| v.to_str().ok())
			.map(|s| s.to_string());
		match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				let content_encoding = resp.headers().typed_get::<headers::ContentEncoding>();
				let (body, _encoding) =
					crate::http::compression::decompress_body(resp.into_body(), content_encoding.as_ref())
						.map_err(ClientError::new)?;
				let event_stream = SseStream::from_byte_stream(body.into_data_stream()).boxed();
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reject_jsonrpc_error_returns_err_for_error_message() {
		let message = serde_json::from_str::<ServerJsonRpcMessage>(
			r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"boom"}}"#,
		)
		.expect("valid json-rpc error");
		let response = StreamableHttpPostResponse::Json(message, None);

		let result = Client::reject_jsonrpc_error("client message", response);

		assert!(
			result.is_err(),
			"json-rpc error should be propagated as error"
		);
	}
}
