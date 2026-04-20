use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;

use ::http::header::{ACCEPT, CONTENT_TYPE};
use ::http::HeaderValue;
use headers::HeaderMapExt;
use rmcp::model::{ClientRequest, JsonObject, JsonRpcRequest, Tool};
use serde_json::Value;

use crate::http::sessionpersistence;
use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, McpHttpClient, UpstreamError};

/// Handler for HTTP-native MCP tools — each tool is backed by a Kubernetes Service
/// and receives tool call arguments as a JSON POST body to `/`.
#[derive(Debug)]
pub struct Handler {
	target_name: String,
	tools: Vec<(Tool, McpHttpClient)>,
}

impl Handler {
	pub fn new(target_name: String, tools: Vec<(Tool, McpHttpClient)>) -> Self {
		Self { target_name, tools }
	}

	pub fn get_session_state(&self) -> sessionpersistence::MCPSession {
		sessionpersistence::MCPSession {
			target_name: Some(self.target_name.clone()),
			session: None,
			backend: None,
		}
	}

	/// HttpTool targets are stateless; each call routes independently to the tool's own backend.
	pub fn set_session_id(&self, _id: Option<&str>, _pinned: Option<SocketAddr>) {}

	pub async fn send_message(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<Messages, UpstreamError> {
		use rmcp::model::*;
		let method = request.request.method();
		let id = request.id;
		let res = match request.request {
			ClientRequest::InitializeRequest(_) => Messages::from_result(
				id,
				ServerInfo::new(ServerCapabilities::builder().enable_tools().build()),
			),
			ClientRequest::ListToolsRequest(_) => Messages::from_result(
				id,
				ListToolsResult {
					meta: None,
					next_cursor: None,
					tools: self.tools(),
				},
			),
			ClientRequest::CallToolRequest(ctr) => {
				let res = self
					.call_tool(ctr.params.name.as_ref(), ctr.params.arguments, ctx)
					.await?;
				let serialized = serde_json::to_string(&res)
					.map_err(|e| UpstreamError::OpenAPIError(e.into()))?;
				let mut result = CallToolResult::success(vec![Content::text(serialized)]);
				result.structured_content = Some(res);
				Messages::from_result(id, result)
			},
			ClientRequest::GetPromptRequest(_) => {
				Messages::from_result(id, GetPromptResult::new(vec![]))
			},
			ClientRequest::ListPromptsRequest(_) => Messages::from_result(
				id,
				ListPromptsResult {
					meta: None,
					next_cursor: None,
					prompts: vec![],
				},
			),
			ClientRequest::ListResourcesRequest(_) => Messages::from_result(
				id,
				ListResourcesResult {
					meta: None,
					next_cursor: None,
					resources: vec![],
				},
			),
			ClientRequest::ListResourceTemplatesRequest(_) => Messages::from_result(
				id,
				ListResourceTemplatesResult {
					meta: None,
					next_cursor: None,
					resource_templates: vec![],
				},
			),
			ClientRequest::ListTasksRequest(_) => {
				Messages::from_result(id, ListTasksResult::new(vec![]))
			},
			ClientRequest::GetTaskInfoRequest(_) => Messages::from_result(
				id,
				GetTaskResult {
					task: Task::default(),
					meta: None,
				},
			),
			ClientRequest::GetTaskResultRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
			ClientRequest::CancelTaskRequest(_) => Messages::empty(),
			ClientRequest::ReadResourceRequest(_) => {
				Messages::from_result(id, ReadResourceResult::new(vec![]))
			},
			ClientRequest::PingRequest(_)
			| ClientRequest::CustomRequest(_)
			| ClientRequest::SetLevelRequest(_)
			| ClientRequest::SubscribeRequest(_)
			| ClientRequest::UnsubscribeRequest(_) => Messages::empty(),
			ClientRequest::CompleteRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
		};
		Ok(res)
	}

	/// POST tool arguments as a JSON body to the tool's backend at `/`.
	async fn call_tool(
		&self,
		name: &str,
		args: Option<JsonObject>,
		ctx: &IncomingRequestContext,
	) -> Result<Value, UpstreamError> {
		let (_tool, client) = self
			.tools
			.iter()
			.find(|(t, _)| t.name == name)
			.ok_or_else(|| UpstreamError::InvalidRequest(format!("tool {name} not found")))?;

		let body = serde_json::to_vec(&args.unwrap_or_default())
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?;

		let uri = format!("http://{}/", client.backend().hostport());
		let mut request = http::Request::builder()
			.method(http::Method::POST)
			.uri(uri)
			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
			.header(ACCEPT, HeaderValue::from_static("application/json"))
			.body(body.into())
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?;

		ctx.apply(&mut request);

		let response = client.call(request).await?;
		let status = response.status();

		if status.is_success() {
			let lim = crate::http::response_buffer_limit(&response);
			let content_encoding = response.headers().typed_get::<headers::ContentEncoding>();
			let body_bytes = crate::http::compression::to_bytes_with_decompression(
				response.into_body(),
				content_encoding.as_ref(),
				lim,
			)
			.await
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?
			.1;

			Ok(
				match serde_json::from_slice::<Value>(&body_bytes)
					.map_err(|e| UpstreamError::OpenAPIError(e.into()))?
				{
					Value::Object(obj) => Value::Object(obj),
					Value::Null => Value::Null,
					data => serde_json::json!({ "data": data }),
				},
			)
		} else {
			let lim = crate::http::response_buffer_limit(&response);
			let body = String::from_utf8(
				crate::http::read_body_with_limit(response.into_body(), lim)
					.await
					.map_err(|e| UpstreamError::OpenAPIError(e.into()))?
					.to_vec(),
			)
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?;
			Err(UpstreamError::OpenAPIError(anyhow::anyhow!(
				"HTTP tool '{name}' call failed with status {status}: {body}"
			)))
		}
	}

	fn tools(&self) -> Vec<Tool> {
		self.tools.iter().map(|(t, _)| t.clone()).collect()
	}
}

/// Convert an `HttpToolEntry` into an rmcp `Tool` for advertising in `tools/list`.
pub(super) fn entry_to_tool(entry: &crate::types::agent::HttpToolEntry) -> Tool {
	let schema_obj = match &entry.schema {
		Value::Object(obj) => obj.clone(),
		_ => serde_json::Map::new(),
	};
	Tool::new_with_raw(
		Cow::Owned(entry.name.clone()),
		entry.description.as_deref().map(|d| Cow::Owned(d.to_string())),
		Arc::new(schema_obj),
	)
}
