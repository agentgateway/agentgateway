use super::*;
use crate::mcp::relay::pool::ClientWrapper;
#[allow(unused_imports)]
use crate::*;
use ::http::HeaderMap;
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use serde::Serialize;
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

impl UpstreamError {
	pub(crate) fn error_code(&self) -> String {
		match self {
			Self::ServiceError(e) => match e {
				rmcp::ServiceError::McpError(_) => "mcp_error".to_string(),
				rmcp::ServiceError::Timeout { timeout: _ } => "timeout".to_string(),
				rmcp::ServiceError::Cancelled { reason } => {
					reason.clone().unwrap_or("cancelled".to_string())
				},
				rmcp::ServiceError::UnexpectedResponse => "unexpected_response".to_string(),
				rmcp::ServiceError::TransportSend(_) => "transport_error".to_string(),
				_ => "unknown".to_string(),
			},
			Self::OpenAPIError(_) => "openapi_error".to_string(),
			Self::Http(_) => "http_error".to_string(),
			Self::Authorization => "unauthorized".to_string(),
			Self::InvalidRequest(_) => "invalid_request".to_string(),
		}
	}
}

// impl From<UpstreamError> for ErrorData {
// 	fn from(value: UpstreamError) -> Self {
// 		match value {
// 			UpstreamError::OpenAPIError(e) => ErrorData::internal_error(e.to_string(), None),
// 			UpstreamError::OpenAPIError(e) => ErrorData::internal_error(e.to_string(), None),
// 			UpstreamError::ServiceError(e) => match e {
// 				rmcp::ServiceError::McpError(e) => e,
// 				rmcp::ServiceError::Timeout { timeout } => {
// 					ErrorData::internal_error(format!("request timed out after {timeout:?}"), None)
// 				},
// 				rmcp::ServiceError::Cancelled { reason } => match reason {
// 					Some(reason) => ErrorData::internal_error(reason.clone(), None),
// 					None => ErrorData::internal_error("unknown reason", None),
// 				},
// 				rmcp::ServiceError::UnexpectedResponse => {
// 					ErrorData::internal_error("unexpected response", None)
// 				},
// 				rmcp::ServiceError::TransportSend(e) => ErrorData::internal_error(e.to_string(), None),
// 				_ => ErrorData::internal_error("unknown error", None),
// 			},
// 		}
// 	}
// }

#[derive(Clone, Serialize, Debug, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum FilterMatcher {
	Equals(String),
	Prefix(String),
	Suffix(String),
	Contains(String),
	#[serde(skip_serializing)]
	Regex(
		#[serde(with = "serde_regex")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		regex::Regex,
	),
}

impl FilterMatcher {
	pub fn matches(&self, value: &str) -> bool {
		match self {
			FilterMatcher::Equals(m) => value == m,
			FilterMatcher::Prefix(m) => value.starts_with(m),
			FilterMatcher::Suffix(m) => value.ends_with(m),
			FilterMatcher::Contains(m) => value.contains(m),
			FilterMatcher::Regex(m) => m.is_match(value),
		}
	}
}

#[derive(Clone, Serialize, Debug, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Filter {
	matcher: FilterMatcher,
	resource_type: String,
}

fn assert_send<T: Send>() {}
fn assert_send2<T: Send>(t: T) -> T {
	t
}
impl Filter {
	pub fn matches(&self, value: &str) -> bool {
		self.matcher.matches(value)
	}

	pub fn new(matcher: FilterMatcher, resource_type: String) -> Self {
		Self {
			matcher,
			resource_type,
		}
	}
}

// UpstreamTarget defines a source for MCP information.
#[derive(Debug)]
pub(crate) enum Upstream {
	McpHttp(ClientWrapper),
	McpStdio(RunningService<RoleClient, crate::mcp::relay::pool::PeerClientHandler>),
	OpenAPI(Box<crate::mcp::openapi::Handler>),
}

impl Upstream {
	// pub(crate) async fn initialize(
	// 	&self,
	// 	param: InitializeRequestParam,
	// ) -> Result<InitializeResult, UpstreamError> {
	// 	match &self {
	// 		Upstream::McpStdio(m) => todo!(),
	// 		Upstream::McpHttp(c) => {
	// 			let res = c
	// 				.send_message(ClientRequest::InitializeRequest(InitializeRequest::new(
	// 										param,
	// 									)),
	// 				)
	// 				.await?;
	// 			let (resp, session_id) = ClientWrapper::expect_single_response(res).await?;
	// 			if let Some(session_id) = session_id {
	// 				c.set_session_id(session_id);
	// 			}
	// 			match resp {
	// 				ServerResult::InitializeResult(ir) => Ok(ir),
	// 				_ => Err(UpstreamError::Http(ClientError::new(anyhow!(
	// 					"expected InitializeResult"
	// 				)))),
	// 			}
	// 		},
	// 		Upstream::OpenAPI(m) => todo!(),
	// 	}
	// }

	// pub(crate) async fn list_tools(
	// 	&self,
	// 	request: Option<PaginatedRequestParam>,
	// ) -> Result<ListToolsResult, UpstreamError> {
	// 	match &self {
	// 		Upstream::McpStdio(m) => Ok(m.list_tools(request).await?),
	// 		Upstream::McpHttp(c) => {
	// 			let res = c
	// 				.send_message(ClientRequest::ListToolsRequest(ListToolsRequest {
	// 										method: ListToolsRequestMethod,
	// 										params: request,
	// 										extensions: Default::default(),
	// 									}),
	// 				)
	// 				.await?;
	// 			let (resp, session_id) = ClientWrapper::expect_single_response(res).await?;
	// 			match resp {
	// 				ServerResult::ListToolsResult(ltr) => Ok(ltr),
	// 				_ => Err(UpstreamError::Http(ClientError::new(anyhow!(
	// 					"expected ListToolsResult"
	// 				)))),
	// 			}
	// 		},
	// 		Upstream::OpenAPI(m) => Ok(ListToolsResult {
	// 			next_cursor: None,
	// 			tools: m.tools(),
	// 		}),
	// 	}
	// }

	// pub(crate) async fn list_tools2(
	// 	&self,
	// 	request: Option<PaginatedRequestParam>,
	// ) -> Result<mergestream::Messages, UpstreamError> {
	// 	match &self {
	// 		Upstream::McpStdio(m) => todo!(),
	// 		Upstream::McpHttp(c) => {
	// 			let res = c
	// 				.send_message(ClientRequest::ListToolsRequest(ListToolsRequest {
	// 										method: ListToolsRequestMethod,
	// 										params: request,
	// 										extensions: Default::default(),
	// 									}),
	// 				)
	// 				.await?;
	// 			res.try_into().map_err(Into::into)
	// 		},
	// 		Upstream::OpenAPI(m) => todo!(),
	// 	}
	// }

	pub(crate) async fn generic_stream(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		user_headers: &http::HeaderMap,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(m) => todo!(),
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
			Upstream::OpenAPI(m) => todo!(),
		}
	}

	pub(crate) async fn generic_notification(
		&self,
		request: ClientNotification,
		user_headers: &http::HeaderMap,
	) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(m) => todo!(),
			Upstream::McpHttp(c) => {
				c.send_notification(request, user_headers).await?;
				Ok(())
			},
			Upstream::OpenAPI(m) => todo!(),
		}
	}

	pub(crate) async fn get_prompt(
		&self,
		request: GetPromptRequestParam,
	) -> Result<GetPromptResult, UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				todo!()
			},
			Upstream::McpStdio(m) => Ok(m.get_prompt(request).await?),
			Upstream::OpenAPI(_) => Ok(GetPromptResult {
				description: None,
				messages: vec![],
			}),
		}
	}

	pub(crate) async fn list_prompts(
		&self,
		request: Option<PaginatedRequestParam>,
	) -> Result<ListPromptsResult, UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				todo!()
			},
			Upstream::McpStdio(m) => Ok(m.list_prompts(request).await?),
			Upstream::OpenAPI(_) => Ok(ListPromptsResult {
				next_cursor: None,
				prompts: vec![],
			}),
		}
	}

	pub(crate) async fn list_resources(
		&self,
		request: Option<PaginatedRequestParam>,
	) -> Result<ListResourcesResult, UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				todo!()
			},
			Upstream::McpStdio(m) => Ok(m.list_resources(request).await?),
			Upstream::OpenAPI(_) => Ok(ListResourcesResult {
				next_cursor: None,
				resources: vec![],
			}),
		}
	}

	pub(crate) async fn list_resource_templates(
		&self,
		request: Option<PaginatedRequestParam>,
	) -> Result<ListResourceTemplatesResult, UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				todo!()
			},
			Upstream::McpStdio(m) => Ok(m.list_resource_templates(request).await?),
			Upstream::OpenAPI(_) => Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: vec![],
			}),
		}
	}

	pub(crate) async fn read_resource(
		&self,
		request: ReadResourceRequestParam,
	) -> Result<ReadResourceResult, UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				todo!()
			},
			Upstream::McpStdio(m) => Ok(m.read_resource(request).await?),
			Upstream::OpenAPI(_) => Ok(ReadResourceResult { contents: vec![] }),
		}
	}
	pub(crate) async fn notify(&self, request: ClientNotification) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpHttp(c) => {
				let res = c
					// TODO
					.send_message2(
						ClientJsonRpcMessage::notification(request),
						&HeaderMap::new(),
					)
					.await?;
				ClientWrapper::expect_accepted(res).await?;
				Ok(())
			},
			Upstream::McpStdio(m) => Ok(m.send_notification(request).await?),
			Upstream::OpenAPI(m) => Ok(()),
		}
	}
	// pub(crate) async fn call_tool(
	// 	&self,
	// 	request: CallToolRequestParam,
	// ) -> Result<CallToolResult, UpstreamError> {
	// 	match &self {
	// 		Upstream::McpHttp(c) => {
	// 			let res = c
	// 				.send_message(
	// 					CallToolRequest {
	// 									method: CallToolRequestMethod,
	// 									params: request,
	// 									extensions: Default::default(),
	// 								}
	// 								.into(),
	// 					,
	// 				)
	// 				.await?;
	// 			todo!()
	// 			// let (resp, session_id) = ClientWrapper::expect_stream(res).await?;
	// 		},
	// 		Upstream::McpStdio(m) => Ok(m.call_tool(request).await?),
	// 		Upstream::OpenAPI(m) => {
	// 			todo!()
	// 			// let res =
	// 			// 	Box::pin(async move { m.call_tool(request.name.as_ref(), request.arguments).await })
	// 			// 		.await?;
	// 			// Ok(CallToolResult {
	// 			// 	content: vec![Content::text(res)],
	// 			// 	// TODO: for JSON responses, return structured_content
	// 			// 	structured_content: None,
	// 			// 	is_error: None,
	// 			// })
	// 		},
	// 	}
	// }
}
