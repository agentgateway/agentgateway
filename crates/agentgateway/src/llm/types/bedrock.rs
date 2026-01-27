use std::collections::HashMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Deserialize, Serialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Role {
	#[default]
	User,
	Assistant,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ContentBlock {
	Text(String),
	Image(ImageBlock),
	ToolResult(ToolResultBlock),
	ToolUse(ToolUseBlock),
	ReasoningContent(ReasoningContentBlock),
	CachePoint(CachePointBlock),
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImageBlock {
	pub format: String,
	pub source: ImageSource,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImageSource {
	pub bytes: String,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(untagged)]
pub enum ReasoningContentBlock {
	// New format from Bedrock: { "reasoningText": { "text": "...", "signature": "..." } }
	Structured {
		#[serde(rename = "reasoningText")]
		reasoning_text: ReasoningText,
	},
	// Legacy/simple format: { "text": "..." }
	Simple {
		text: String,
	},
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningText {
	pub text: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub signature: Option<String>,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultBlock {
	/// The ID of the tool request that this is the result for.
	pub tool_use_id: String,
	/// The content for tool result content block.
	pub content: Vec<ToolResultContentBlock>,
	/// The status for the tool result content block.
	/// This field is only supported Anthropic Claude 3 models.
	pub status: Option<ToolResultStatus>,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolResultStatus {
	Error,
	Success,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlock {
	/// The ID for the tool request.
	pub tool_use_id: String,
	/// The name of the tool that the model wants to use.
	pub name: String,
	/// The input to pass to the tool.
	pub input: serde_json::Value,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ToolResultContentBlock {
	/// A tool result that is text.
	Text(String),
	/// A tool result that is an image.
	Image(ImageBlock),
	/// A tool result that is JSON format data.
	Json(serde_json::Value),
	/// A tool result that is video.
	Video(serde_json::Value),
}
#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase", untagged)]
pub enum SystemContentBlock {
	CachePoint {
		#[serde(rename = "cachePoint")]
		cache_point: CachePointBlock,
	},
	Text {
		text: String,
	},
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Message {
	pub role: Role,
	pub content: Vec<ContentBlock>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct InferenceConfiguration {
	/// The maximum number of tokens to generate before stopping.
	#[serde(rename = "maxTokens")]
	pub max_tokens: usize,
	/// Amount of randomness injected into the response.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,
	/// Use nucleus sampling.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
	/// Only sample from the top K options for each subsequent token (if supported by model).
	#[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
	pub top_k: Option<usize>,
	/// The stop sequences to use.
	#[serde(rename = "stopSequences", skip_serializing_if = "Vec::is_empty")]
	pub stop_sequences: Vec<String>,
}

#[derive(Clone, Serialize, Debug)]
pub struct ConverseRequest {
	/// Specifies the model or throughput with which to run inference.
	#[serde(rename = "modelId")]
	pub model_id: String,
	/// The messages that you want to send to the model.
	pub messages: Vec<Message>,
	/// A prompt that provides instructions or context to the model.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<Vec<SystemContentBlock>>,
	/// Inference parameters to pass to the model.
	#[serde(rename = "inferenceConfig", skip_serializing_if = "Option::is_none")]
	pub inference_config: Option<InferenceConfiguration>,
	/// Configuration information for the tools that the model can use.
	#[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
	pub tool_config: Option<ToolConfiguration>,
	/// Configuration information for a guardrail.
	#[serde(rename = "guardrailConfig", skip_serializing_if = "Option::is_none")]
	pub guardrail_config: Option<GuardrailConfiguration>,
	/// Additional model request fields.
	#[serde(
		rename = "additionalModelRequestFields",
		skip_serializing_if = "Option::is_none"
	)]
	pub additional_model_request_fields: Option<serde_json::Value>,
	/// Prompt variables.
	#[serde(rename = "promptVariables", skip_serializing_if = "Option::is_none")]
	pub prompt_variables: Option<HashMap<String, PromptVariableValues>>,
	/// Additional model response field paths.
	#[serde(
		rename = "additionalModelResponseFieldPaths",
		skip_serializing_if = "Option::is_none"
	)]
	pub additional_model_response_field_paths: Option<Vec<String>>,
	/// Request metadata.
	#[serde(rename = "requestMetadata", skip_serializing_if = "Option::is_none")]
	pub request_metadata: Option<HashMap<String, String>>,
	/// Performance configuration.
	#[serde(rename = "performanceConfig", skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CountTokensRequest {
	pub input: CountTokensInputInvokeModel,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CountTokensInputInvokeModel {
	pub invoke_model: InvokeModelBody,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct InvokeModelBody {
	pub body: String,
}

#[derive(Clone, Serialize, Debug)]
pub struct ToolConfiguration {
	/// An array of tools that you want to pass to a model.
	pub tools: Vec<Tool>,
	/// If supported by model, forces the model to request a tool.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_choice: Option<ToolChoice>,
}

#[derive(Clone, std::fmt::Debug, ::serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
	/// CachePoint to include in the tool configuration.
	CachePoint(CachePointBlock),
	/// The specification for the tool.
	ToolSpec(ToolSpecification),
}

#[derive(Clone, std::fmt::Debug, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachePointBlock {
	/// Specifies the type of cache point within the CachePointBlock.
	pub r#type: CachePointType,
}

#[derive(
	Clone,
	Eq,
	Ord,
	PartialEq,
	PartialOrd,
	std::fmt::Debug,
	std::hash::Hash,
	::serde::Serialize,
	::serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum CachePointType {
	Default,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct GuardrailConfiguration {
	/// The unique identifier of the guardrail
	#[serde(rename = "guardrailIdentifier")]
	pub guardrail_identifier: String,
	/// The version of the guardrail
	#[serde(rename = "guardrailVersion")]
	pub guardrail_version: String,
	/// Whether to enable trace output from the guardrail
	#[serde(rename = "trace", skip_serializing_if = "Option::is_none")]
	pub trace: Option<String>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct PromptVariableValues {
	// TODO: Implement prompt variable values
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PerformanceConfiguration {
	// TODO: Implement performance configuration
}

/// The actual response from the Bedrock Converse API (matches AWS SDK ConverseOutput)
#[derive(Debug, Deserialize, Clone)]
pub struct ConverseResponse {
	/// The result from the call to Converse
	pub output: Option<ConverseOutput>,
	/// The reason why the model stopped generating output
	#[serde(rename = "stopReason")]
	pub stop_reason: StopReason,
	/// The total number of tokens used in the call to Converse
	pub usage: Option<TokenUsage>,
	/// Metrics for the call to Converse
	#[allow(dead_code)]
	pub metrics: Option<ConverseMetrics>,
	/// Additional fields in the response that are unique to the model
	#[allow(dead_code)]
	#[serde(rename = "additionalModelResponseFields")]
	pub additional_model_response_fields: Option<serde_json::Value>,
	/// A trace object that contains information about the Guardrail behavior
	pub trace: Option<ConverseTrace>,
	/// Model performance settings for the request
	#[serde(rename = "performanceConfig")]
	#[allow(dead_code)]
	pub performance_config: Option<PerformanceConfiguration>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConverseErrorResponse {
	// Sometimes its capitalized, sometimes it is not... yikes.
	#[serde(alias = "Message")]
	pub message: String,
}

/// The actual content output from the model
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ConverseOutput {
	Message(Message),
	#[serde(other)]
	Unknown,
}

/// Token usage information
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct TokenUsage {
	/// The number of input tokens which were used
	#[serde(rename = "inputTokens")]
	pub input_tokens: usize,
	/// The number of output tokens which were used
	#[serde(rename = "outputTokens")]
	pub output_tokens: usize,
	/// The total number of tokens used
	#[serde(rename = "totalTokens")]
	pub total_tokens: usize,
	/// The number of input tokens read from cache (optional)
	#[serde(
		rename = "cacheReadInputTokens",
		skip_serializing_if = "Option::is_none"
	)]
	pub cache_read_input_tokens: Option<usize>,
	/// The number of input tokens written to cache (optional)
	#[serde(
		rename = "cacheWriteInputTokens",
		skip_serializing_if = "Option::is_none"
	)]
	pub cache_write_input_tokens: Option<usize>,
}

/// Metrics for the Converse call
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct ConverseMetrics {
	/// Latency in milliseconds
	#[serde(rename = "latencyMs")]
	pub latency_ms: u64,
}

/// Trace information for Guardrail behavior
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConverseTrace {
	/// Guardrail trace information
	#[serde(rename = "guardrail", skip_serializing_if = "Option::is_none")]
	pub guardrail: Option<serde_json::Value>,
}

/// Reason for stopping the response generation.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
	ContentFiltered,
	EndTurn,
	GuardrailIntervened,
	MaxTokens,
	ModelContextWindowExceeded,
	StopSequence,
	ToolUse,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolChoice {
	/// The model must request at least one tool (no text is generated).
	Any,
	/// (Default). The Model automatically decides if a tool should be called or whether to generate text instead.
	Auto,
	/// The Model must request the specified tool. Only supported by Anthropic Claude 3 models.
	Tool { name: String },
	/// The `Unknown` variant represents cases where new union variant was received. Consider upgrading the SDK to the latest available version.
	/// An unknown enum variant
	///
	/// _Note: If you encounter this error, consider upgrading your SDK to the latest version._
	/// The `Unknown` variant represents cases where the server sent a value that wasn't recognized
	/// by the client. This can happen when the server adds new functionality, but the client has not been updated.
	/// To investigate this, consider turning on debug logging to print the raw HTTP response.
	#[non_exhaustive]
	#[allow(dead_code)]
	Unknown,
}

#[derive(Clone, std::fmt::Debug, ::serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpecification {
	/// The name for the tool.
	pub name: String,
	/// The description for the tool.
	pub description: Option<String>,
	/// The input schema for the tool in JSON format.
	pub input_schema: Option<ToolInputSchema>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolInputSchema {
	Json(serde_json::Value),
}

// This is NOT deserialized directly, see the associated method
#[derive(Clone, Debug)]
pub enum ConverseStreamOutput {
	/// The messages output content block delta.
	ContentBlockDelta(ContentBlockDeltaEvent),
	/// Start information for a content block.
	#[allow(unused)]
	ContentBlockStart(ContentBlockStartEvent),
	/// Stop information for a content block.
	#[allow(unused)]
	ContentBlockStop(ContentBlockStopEvent),
	/// Message start information.
	MessageStart(MessageStartEvent),
	/// Message stop information.
	MessageStop(MessageStopEvent),
	/// Metadata for the converse output stream.
	Metadata(ConverseStreamMetadataEvent),
}

impl ConverseStreamOutput {
	pub fn deserialize(m: aws_event_stream_parser::Message) -> anyhow::Result<Self> {
		let Some(v) = m
			.headers
			.headers
			.iter()
			.find(|h| h.key.as_str() == ":event-type")
			.and_then(|v| match &v.value {
				aws_event_stream_parser::HeaderValue::String(s) => Some(s.to_string()),
				_ => None,
			})
		else {
			anyhow::bail!("no event type header")
		};
		Ok(match v.as_str() {
			"contentBlockDelta" => ConverseStreamOutput::ContentBlockDelta(serde_json::from_slice::<
				ContentBlockDeltaEvent,
			>(&m.body)?),
			"contentBlockStart" => ConverseStreamOutput::ContentBlockStart(serde_json::from_slice::<
				ContentBlockStartEvent,
			>(&m.body)?),
			"contentBlockStop" => ConverseStreamOutput::ContentBlockStop(serde_json::from_slice::<
				ContentBlockStopEvent,
			>(&m.body)?),
			"messageStart" => {
				ConverseStreamOutput::MessageStart(serde_json::from_slice::<MessageStartEvent>(&m.body)?)
			},
			"messageStop" => {
				ConverseStreamOutput::MessageStop(serde_json::from_slice::<MessageStopEvent>(&m.body)?)
			},
			"metadata" => ConverseStreamOutput::Metadata(serde_json::from_slice::<
				ConverseStreamMetadataEvent,
			>(&m.body)?),
			m => anyhow::bail!("unexpected event type: {m}"),
		})
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockDeltaEvent {
	/// The delta for a content block delta event.
	pub delta: Option<ContentBlockDelta>,
	/// The block index for a content block delta event.
	#[allow(dead_code)]
	pub content_block_index: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct ContentBlockStartEvent {
	/// Start information about a content block start event.
	pub start: Option<ContentBlockStart>,
	/// The index for a content block start event.
	pub content_block_index: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct ContentBlockStopEvent {
	/// The index for a content block.
	pub content_block_index: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStartEvent {
	/// The role for the message.
	pub role: Role,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStopEvent {
	/// The reason why the model stopped generating output.
	pub stop_reason: StopReason,
	/// The additional model response fields.
	#[allow(dead_code)]
	pub additional_model_response_fields: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseStreamMetadataEvent {
	/// Usage information for the conversation stream event.
	pub usage: Option<TokenUsage>,
	/// The metrics for the conversation stream metadata event.
	#[allow(dead_code)]
	pub metrics: Option<ConverseMetrics>,
	/// Model performance configuration metadata for the conversation stream event.
	#[allow(dead_code)]
	pub performance_config: Option<PerformanceConfiguration>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentBlockDelta {
	ReasoningContent(ReasoningContentBlockDelta),
	Text(String),
	ToolUse(#[allow(unused)] ToolUseBlockDelta),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlockDelta {
	#[allow(unused)]
	pub input: String,
}

#[derive(Clone, Debug, Deserialize)]
pub enum ReasoningContentBlockDelta {
	#[serde(rename = "redactedContent")]
	RedactedContent(#[allow(unused)] Bytes),
	#[serde(rename = "signature")]
	Signature(#[allow(unused)] String),
	#[serde(rename = "text")]
	Text(String),
	#[non_exhaustive]
	Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentBlockStart {
	/// Information about a tool that the model is requesting to use.
	#[allow(dead_code)]
	ToolUse(ToolUseBlockStart),
	/// Reasoning/thinking content block start
	#[allow(dead_code)]
	ReasoningContent,
	/// Text content block start
	#[allow(dead_code)]
	Text,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlockStart {
	/// The ID for the tool request.
	#[allow(dead_code)]
	pub tool_use_id: String,
	/// The name of the tool that the model is requesting to use.
	#[allow(dead_code)]
	pub name: String,
}
