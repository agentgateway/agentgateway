use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, HeaderValue, header};
use serde::{Deserialize, Serialize};

use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::SpanWriter;
use crate::types::agent::SimpleBackendReference;
use crate::*;

const REQUEST_PATH: &str = "request";
const RESPONSE_PATH: &str = "response";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsPromptRequest {
	/// body contains the object which is a list of the Message JSON objects from the prompts in the request
	pub body: PromptMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsPromptResponse {
	/// action is the action to be taken based on the request.
	/// The following actions are available on the response:
	/// - PassAction: No action is required.
	/// - MaskAction: Mask the response body.
	/// - RejectAction: Reject the request.
	pub action: RequestAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsResponseRequest {
	/// body contains the object with a list of Choice that contains the response content from the LLM.
	pub body: ResponseChoices,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsResponseResponse {
	/// action is the action to be taken based on the request.
	/// The following actions are available on the response:
	/// - PassAction: No action is required.
	/// - MaskAction: Mask the response body.
	/// - RejectAction: Reject the response.
	pub action: ResponseAction,
}

// For convenience, re-use SimpleChatCompletionMessage
pub type Message = crate::llm::SimpleChatCompletionMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptMessages {
	/// List of prompt messages including role and content.
	pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResponseChoice {
	/// message contains the role and text content of the response from the LLM model.
	pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResponseChoices {
	/// list of possible independent responses from the LLM
	pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PassAction {
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MaskAction {
	/// body contains the modified messages that masked out some of the original contents.
	/// When used in a GuardrailPromptResponse, this should be PromptMessages.
	/// When used in GuardrailResponseResponse, this should be ResponseChoices
	pub body: MaskActionBody,
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RejectAction {
	/// body is the rejection message that will be used for HTTP error response body.
	pub body: String,
	/// status_code is the HTTP status code to be returned in the HTTP error response.
	pub status_code: u16,
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

/// Enum for actions available in prompt responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum RequestAction {
	Mask(MaskAction),
	Reject(RejectAction),
	Pass(PassAction),
}

/// Enum for actions available in response responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum ResponseAction {
	Mask(MaskAction),
	Reject(RejectAction),
	Pass(PassAction),
}

/// Enum for MaskAction body that can be either PromptMessages or ResponseChoices
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaskActionBody {
	PromptMessages(PromptMessages),
	ResponseChoices(ResponseChoices),
}

fn build_request_for_request(
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsPromptRequest {
		body: PromptMessages { messages },
	};
	build_request(&body, REQUEST_PATH, http_headers)
}

fn build_request_for_response(
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsResponseRequest {
		body: ResponseChoices { choices },
	};
	build_request(&body, RESPONSE_PATH, http_headers)
}

fn build_request<T: serde::Serialize>(
	body: &T,
	path: &str,
	http_headers: &HeaderMap,
) -> anyhow::Result<crate::http::Request> {
	let body_bytes = serde_json::to_vec(body)?;
	let mut rb = ::http::Request::builder()
		.uri(format!("/{path}"))
		.method(http::Method::POST);
	for (k, v) in http_headers {
		// TODO: this is configurable by users
		if k == header::CONTENT_LENGTH {
			// TODO: probably others
			continue;
		}
		rb = rb.header(k.clone(), v.clone());
	}
	let req = rb
		.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
		.body(crate::http::Body::from(body_bytes))?;
	Ok(req)
}

pub async fn send_request(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	messages: Vec<Message>,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<GuardrailsPromptResponse> {
	let whr = build_request_for_request(http_headers, messages)?;
	let res = Box::pin(client.call_reference_and_span(whr, target, span_writer)).await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

pub async fn send_response(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<GuardrailsResponseResponse> {
	let whr = build_request_for_response(http_headers, choices)?;
	let res = client
		.call_reference_and_span(whr, target, span_writer)
		.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}
