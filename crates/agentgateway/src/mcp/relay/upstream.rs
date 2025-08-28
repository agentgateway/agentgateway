use super::*;
use crate::mcp::relay::pool::ClientWrapper;
#[allow(unused_imports)]
use crate::*;
use ::http::HeaderMap;
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpstreamError {
	#[error("unauthorized tool call")]
	Authorization,
	#[error("invalid request: {0}")]
	InvalidRequest(String),
	#[error("stdio upstream error: {0}")]
	ServiceError(#[from] rmcp::ServiceError),
	#[error("http upstream error: {0}")]
	Http(#[from] pool::ClientError),
	#[error("openapi upstream error: {0}")]
	OpenAPIError(#[from] anyhow::Error),
}

// UpstreamTarget defines a source for MCP information.
#[derive(Debug)]
pub(crate) enum Upstream {
	McpHttp(ClientWrapper),
	McpStdio(()),
	OpenAPI(Box<crate::mcp::openapi::Handler>),
}

impl Upstream {
	pub(crate) async fn delete(&self, user_headers: &http::HeaderMap) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(_m) => todo!(),
			Upstream::McpHttp(c) => {
				c.send_delete(user_headers).await?;
				Ok(())
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
	}
	pub(crate) async fn get_event_stream(
		&self,
		user_headers: &http::HeaderMap,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(_m) => todo!(),
			Upstream::McpHttp(c) => c
				.get_event_stream(user_headers)
				.await?
				.try_into()
				.map_err(Into::into),
			Upstream::OpenAPI(_m) => todo!(),
		}
	}
	pub(crate) async fn generic_stream(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		user_headers: &http::HeaderMap,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(_m) => todo!(),
			Upstream::McpHttp(c) => {
				let is_init = matches!(&request.request, &ClientRequest::InitializeRequest(_));
				let res = c.send_message(request, user_headers).await?;
				if is_init {
					let sid = match &res {
						StreamableHttpPostResponse::Accepted => None,
						StreamableHttpPostResponse::Json(_, sid) => sid.as_ref(),
						StreamableHttpPostResponse::Sse(_, sid) => sid.as_ref(),
					};
					if let Some(sid) = sid {
						c.set_session_id(sid.clone())
					}
				}
				res.try_into().map_err(Into::into)
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
	}

	pub(crate) async fn generic_notification(
		&self,
		request: ClientNotification,
		user_headers: &http::HeaderMap,
	) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(_m) => todo!(),
			Upstream::McpHttp(c) => {
				c.send_notification(request, user_headers).await?;
				Ok(())
			},
			Upstream::OpenAPI(_m) => todo!(),
		}
	}

	pub(crate) async fn notify(&self, request: ClientNotification) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				let res = c
					.send_message2(
						ClientJsonRpcMessage::notification(request),
						&HeaderMap::new(),
					)
					.await?;
				ClientWrapper::expect_accepted(res).await?;
				Ok(())
			},
			Upstream::McpStdio(_m) => todo!(),
			Upstream::OpenAPI(_m) => Ok(()),
		}
	}
}
