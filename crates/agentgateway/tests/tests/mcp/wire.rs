use agentgateway::http;
use anyhow::Context;
use futures_util::StreamExt;
use rmcp::model::*;
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use serde_json::json;
use sse_stream::SseStream;
use tokio::time::Duration;

#[derive(Debug)]
pub(super) struct StreamEventMessage {
	pub(super) event_id: String,
	pub(super) message: ServerJsonRpcMessage,
}

pub(super) fn mcp_post_headers(session_id: Option<&str>) -> http::HeaderMap {
	let mut headers = http::HeaderMap::new();
	headers.insert(
		http::header::ACCEPT,
		http::HeaderValue::from_static("application/json, text/event-stream"),
	);
	if let Some(session_id) = session_id {
		headers.insert(
			HEADER_SESSION_ID,
			http::HeaderValue::from_str(session_id).expect("session id header should be valid"),
		);
	}
	headers
}

pub(super) fn initialize_request_message(id: i64) -> serde_json::Value {
	serde_json::to_value(ClientJsonRpcMessage::request(
		InitializeRequest::new(ClientInfo::new(
			ClientCapabilities::default(),
			Implementation::new("restart-handoff-test", "1.0.0"),
		))
		.into(),
		RequestId::Number(id),
	))
	.expect("initialize request should serialize")
}

pub(super) fn initialized_notification_message() -> serde_json::Value {
	serde_json::to_value(ClientJsonRpcMessage::notification(
		ClientNotification::InitializedNotification(InitializedNotification::default()),
	))
	.expect("initialized notification should serialize")
}

pub(super) fn list_prompts_request_message(id: i64) -> serde_json::Value {
	serde_json::to_value(ClientJsonRpcMessage::request(
		ClientRequest::ListPromptsRequest(ListPromptsRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams::default()),
			extensions: Default::default(),
		}),
		RequestId::Number(id),
	))
	.expect("list prompts request should serialize")
}

pub(super) fn list_resources_request_message(id: i64) -> serde_json::Value {
	serde_json::to_value(ClientJsonRpcMessage::request(
		ClientRequest::ListResourcesRequest(ListResourcesRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams::default()),
			extensions: Default::default(),
		}),
		RequestId::Number(id),
	))
	.expect("list resources request should serialize")
}

pub(super) fn subscribe_request_message(id: i64, uri: impl Into<String>) -> serde_json::Value {
	json!({
		"jsonrpc": "2.0",
		"id": id,
		"method": "resources/subscribe",
		"params": {
			"uri": uri.into(),
		}
	})
}

pub(super) fn call_tool_request_message(
	id: i64,
	name: impl Into<std::borrow::Cow<'static, str>>,
	arguments: Option<JsonObject>,
) -> serde_json::Value {
	serde_json::to_value(ClientJsonRpcMessage::request(
		ClientRequest::CallToolRequest(CallToolRequest::new(call_tool_params(
			name, arguments, None,
		))),
		RequestId::Number(id),
	))
	.expect("call tool request should serialize")
}

pub(super) async fn open_event_stream(
	port: u16,
	session_id: &str,
	last_event_id: Option<&str>,
) -> anyhow::Result<reqwest::Response> {
	let client = reqwest::Client::new();
	let mut request = client
		.get(format!("http://localhost:{port}/mcp"))
		.header(reqwest::header::ACCEPT, EVENT_STREAM_MIME_TYPE)
		.header(HEADER_SESSION_ID, session_id);
	if let Some(last_event_id) = last_event_id {
		request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
	}
	let response = request.send().await?.error_for_status()?;
	let content_type = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.context("event stream response missing content-type")?;
	anyhow::ensure!(
		content_type.starts_with(EVENT_STREAM_MIME_TYPE),
		"unexpected event stream content-type: {content_type}"
	);
	Ok(response)
}

pub(super) async fn read_first_sse_message(
	response: reqwest::Response,
) -> anyhow::Result<StreamEventMessage> {
	let mut stream = SseStream::from_byte_stream(response.bytes_stream());
	let sse = tokio::time::timeout(Duration::from_secs(5), async {
		loop {
			match stream.next().await {
				Some(Ok(sse)) if sse.data.as_deref().is_some_and(|data| !data.is_empty()) => {
					return Ok(sse);
				},
				Some(Ok(_)) => continue,
				Some(Err(error)) => return Err(anyhow::anyhow!("failed to read SSE event: {error}")),
				None => return Err(anyhow::anyhow!("event stream ended before first message")),
			}
		}
	})
	.await
	.context("timed out waiting for SSE event")??;
	let event_id = sse.id.context("SSE event missing id")?;
	let data = sse.data.context("SSE event missing data")?;
	let message = serde_json::from_str(&data)?;
	Ok(StreamEventMessage { event_id, message })
}

pub(super) fn resource_updated_uri(message: &ServerJsonRpcMessage) -> anyhow::Result<&str> {
	let ServerJsonRpcMessage::Notification(notification) = message else {
		anyhow::bail!("expected notification message, got {message:?}");
	};
	let ServerNotification::ResourceUpdatedNotification(update) = &notification.notification else {
		anyhow::bail!(
			"expected resources/updated notification, got {:?}",
			notification.notification
		);
	};
	Ok(update.params.uri.as_str())
}

pub(super) async fn read_server_message(
	resp: http::Response,
) -> anyhow::Result<ServerJsonRpcMessage> {
	let status = resp.status();
	let content_type = resp
		.headers()
		.get(http::header::CONTENT_TYPE)
		.and_then(|v| v.to_str().ok())
		.map(ToOwned::to_owned);
	let body = http::read_body_with_limit(resp.into_body(), 64 * 1024).await?;
	anyhow::ensure!(
		status.is_success(),
		"unexpected MCP response status {status}: {}",
		String::from_utf8_lossy(&body)
	);
	match content_type.as_deref() {
		Some(value) if value.starts_with(JSON_MIME_TYPE) => Ok(serde_json::from_slice(&body)?),
		Some(value) if value.starts_with(EVENT_STREAM_MIME_TYPE) => {
			let payload = String::from_utf8(body.to_vec())?;
			let event_data = payload
				.lines()
				.filter_map(|line| line.strip_prefix("data:"))
				.map(str::trim_start)
				.collect::<Vec<_>>()
				.join("\n");
			anyhow::ensure!(
				!event_data.is_empty(),
				"SSE MCP response did not contain a data event: {payload:?}"
			);
			Ok(serde_json::from_str(&event_data)?)
		},
		_ => anyhow::bail!("unexpected MCP response content-type: {:?}", content_type),
	}
}

pub(super) fn call_tool_params(
	name: impl Into<std::borrow::Cow<'static, str>>,
	arguments: Option<JsonObject>,
	task: Option<JsonObject>,
) -> CallToolRequestParams {
	let mut params = CallToolRequestParams::new(name);
	if let Some(arguments) = arguments {
		params = params.with_arguments(arguments);
	}
	if let Some(task) = task {
		params = params.with_task(task);
	}
	params
}

pub(super) fn unsubscribe_request_params(uri: impl Into<String>) -> UnsubscribeRequestParams {
	serde_json::from_value(json!({ "uri": uri.into() })).expect("unsubscribe params should be valid")
}
