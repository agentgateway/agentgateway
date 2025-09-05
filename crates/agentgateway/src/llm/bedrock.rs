use agent_core::prelude::Strng;
use agent_core::strng;
use async_openai::types::FinishReason;
use bytes::Bytes;
use chrono;
use itertools::Itertools;
use rand::Rng;
use tracing::trace;

use crate::http::{Body, Response};
use crate::llm::bedrock::types::{
	ContentBlock, ContentBlockDelta, ConverseErrorResponse, ConverseRequest, ConverseResponse,
	ConverseStreamOutput, StopReason,
};
use crate::llm::{AIError, LLMResponse, universal};
use crate::telemetry::log::AsyncLog;
use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("bedrock");
}

impl Provider {
	pub async fn process_request(
		&self,
		mut req: universal::Request,
	) -> Result<ConverseRequest, AIError> {
		// Use provider's model if configured, otherwise keep the request model
		if let Some(provider_model) = &self.model {
			req.model = Some(provider_model.to_string());
		} else if req.model.is_none() {
			return Err(AIError::MissingField("model not specified".into()));
		}
		let bedrock_request = translate_request(req, self);

		Ok(bedrock_request)
	}

	pub async fn process_response(
		&self,
		model: &str,
		bytes: &Bytes,
	) -> Result<universal::Response, AIError> {
		let model = self.model.as_deref().unwrap_or(model);
		let resp =
			serde_json::from_slice::<ConverseResponse>(bytes).map_err(AIError::ResponseParsing)?;

		// Bedrock response doesn't contain the model, so we pass through the model from the request into the response
		translate_response(resp, model)
	}

	pub async fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp =
			serde_json::from_slice::<ConverseErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}

	pub(super) async fn process_streaming(
		&self,
		log: AsyncLog<LLMResponse>,
		resp: Response,
		model: &str,
	) -> Response {
		let model = self.model.as_deref().unwrap_or(model).to_string();
		// Bedrock doesn't return an ID, so get one from the request... if we can
		let message_id = resp
			.headers()
			.get(http::x_headers::X_AMZN_REQUESTID)
			.and_then(|s| s.to_str().ok().map(|s| s.to_owned()))
			.unwrap_or_else(|| format!("{:016x}", rand::rng().random::<u64>()));
		resp.map(|b| translate_stream(b, log, model, message_id))
	}

	pub fn get_path_for_model(&self, streaming: bool, model: &str) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		if streaming {
			strng::format!("/model/{model}/converse-stream")
		} else {
			strng::format!("/model/{model}/converse")
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
}

pub(super) fn translate_error(
	resp: ConverseErrorResponse,
) -> Result<universal::ChatCompletionErrorResponse, AIError> {
	Ok(universal::ChatCompletionErrorResponse {
		event_id: None,
		error: universal::ChatCompletionError {
			r#type: "invalid_request_error".to_string(),
			message: resp.message,
			param: None,
			code: None,
			event_id: None,
		},
	})
}

pub(super) fn translate_response(
	resp: ConverseResponse,
	model: &str,
) -> Result<universal::Response, AIError> {
	// Get the output content from the response
	let output = resp.output.ok_or(AIError::IncompleteResponse)?;

	// Extract the message from the output
	let message = match output {
		types::ConverseOutput::Message(msg) => msg,
		types::ConverseOutput::Unknown => return Err(AIError::IncompleteResponse),
	};
	// Bedrock has a vec of possible content types, while openai allows 1 text content and many tool calls
	// Assume the bedrock response has only one text
	// Convert Bedrock content blocks to OpenAI message content
	let mut tool_calls: Vec<universal::MessageToolCall> = Vec::new();
	let mut content = None;
	for block in &message.content {
		match block {
			ContentBlock::Text(text) => {
				content = Some(text.clone());
			},
			ContentBlock::Image { .. } => continue, // Skip images in response for now
			ContentBlock::ToolResult(_) => {
				// There should not be a ToolResult in the response, only in the request
				continue;
			},
			ContentBlock::ToolUse(tu) => {
				let Some(args) = serde_json::to_string(&tu.input).ok() else {
					continue;
				};
				tool_calls.push(universal::MessageToolCall {
					id: tu.tool_use_id.clone(),
					r#type: universal::ToolType::Function,
					function: universal::FunctionCall {
						name: tu.name.clone(),
						arguments: args,
					},
				});
			}, // TODO: guard content, reasoning
		};
	}

	let message = universal::ResponseMessage {
		role: universal::Role::Assistant,
		content,
		tool_calls: if tool_calls.is_empty() {
			None
		} else {
			Some(tool_calls)
		},
		#[allow(deprecated)]
		function_call: None,
		refusal: None,
		audio: None,
		extra: None,
		reasoning_content: None,
	};
	let finish_reason = Some(translate_stop_reason(&resp.stop_reason));
	// Only one choice for Bedrock
	let choice = universal::ChatChoice {
		index: 0,
		message,
		finish_reason,
		logprobs: None,
	};
	let choices = vec![choice];

	// Convert usage from Bedrock format to OpenAI format
	let usage = if let Some(token_usage) = resp.usage {
		universal::Usage {
			prompt_tokens: token_usage.input_tokens as u32,
			completion_tokens: token_usage.output_tokens as u32,
			total_tokens: token_usage.total_tokens as u32,

			prompt_tokens_details: None,
			completion_tokens_details: None,
		}
	} else {
		// Fallback if usage is not provided
		universal::Usage::default()
	};

	// Generate a unique ID since it's not provided in the response
	let id = format!("bedrock-{}", chrono::Utc::now().timestamp_millis());

	// Log guardrail trace information if present
	if let Some(trace) = &resp.trace
		&& let Some(guardrail_trace) = &trace.guardrail
	{
		trace!("Bedrock guardrail trace: {:?}", guardrail_trace);
	}

	Ok(universal::Response {
		id,
		object: "chat.completion".to_string(),
		created: chrono::Utc::now().timestamp() as u32,
		model: model.to_string(),
		choices,
		usage: Some(usage),
		service_tier: None,
		system_fingerprint: None,
	})
}

fn translate_stop_reason(resp: &StopReason) -> FinishReason {
	match resp {
		StopReason::EndTurn => universal::FinishReason::Stop,
		StopReason::MaxTokens => universal::FinishReason::Length,
		StopReason::StopSequence => universal::FinishReason::Stop,
		StopReason::ContentFiltered => universal::FinishReason::ContentFilter,
		StopReason::GuardrailIntervened => universal::FinishReason::ContentFilter,
		StopReason::ToolUse => universal::FinishReason::ToolCalls,
	}
}

pub(super) fn translate_request(req: universal::Request, provider: &Provider) -> ConverseRequest {
	// Bedrock has system prompts in a separate field. Join them
	let system = req
		.messages
		.iter()
		.filter_map(|msg| {
			if universal::message_role(msg) == universal::SYSTEM_ROLE {
				universal::message_text(msg).map(|s| s.to_string())
			} else {
				None
			}
		})
		.collect::<Vec<String>>()
		.join("\n");

	// Convert messages to Bedrock format
	let messages = req
		.messages
		.iter()
		.filter(|msg| universal::message_role(msg) != universal::SYSTEM_ROLE)
		.filter_map(|msg| {
			let role = match universal::message_role(msg) {
				universal::ASSISTANT_ROLE => types::Role::Assistant,
				// Default to user for other roles
				_ => types::Role::User,
			};

			universal::message_text(msg)
				.map(|s| vec![ContentBlock::Text(s.to_string())])
				.map(|content| types::Message { role, content })
		})
		.collect();

	// Build inference configuration
	let inference_config = types::InferenceConfiguration {
		max_tokens: universal::max_tokens(&req),
		temperature: req.temperature,
		top_p: req.top_p,
		stop_sequences: universal::stop_sequence(&req),
		anthropic_version: None, // Not used for Bedrock
	};

	// Build guardrail configuration if specified
	let guardrail_config = if let (Some(identifier), Some(version)) =
		(&provider.guardrail_identifier, &provider.guardrail_version)
	{
		Some(types::GuardrailConfiguration {
			guardrail_identifier: identifier.to_string(),
			guardrail_version: version.to_string(),
			trace: Some("enabled".to_string()),
		})
	} else {
		None
	};

	let metadata = req
		.user
		.map(|user| HashMap::from([("user_id".to_string(), user)]));

	let tool_choice = match req.tool_choice {
		Some(universal::ToolChoiceOption::Named(universal::NamedToolChoice {
			r#type: _,
			function,
		})) => Some(types::ToolChoice::Tool {
			name: function.name,
		}),
		Some(universal::ToolChoiceOption::Auto) => Some(types::ToolChoice::Auto),
		Some(universal::ToolChoiceOption::Required) => Some(types::ToolChoice::Any),
		Some(universal::ToolChoiceOption::None) => None,
		None => None,
	};
	let tools = req.tools.map(|tools| {
		tools
			.into_iter()
			.map(|tool| {
				let tool_spec = types::ToolSpecification {
					name: tool.function.name,
					description: tool.function.description,
					input_schema: tool.function.parameters.map(types::ToolInputSchema::Json),
				};

				types::Tool::ToolSpec(tool_spec)
			})
			.collect_vec()
	});
	let tool_config = tools.map(|tools| types::ToolConfiguration { tools, tool_choice });
	ConverseRequest {
		model_id: req.model.unwrap_or_default(),
		messages,
		system: if system.is_empty() {
			None
		} else {
			Some(vec![types::SystemContentBlock::Text { text: system }])
		},
		inference_config: Some(inference_config),
		tool_config,
		guardrail_config,
		additional_model_request_fields: None,
		prompt_variables: None,
		additional_model_response_field_paths: None,
		request_metadata: metadata,
		performance_config: None,
	}
}

pub(super) fn translate_stream(
	b: Body,
	log: AsyncLog<LLMResponse>,
	model: String,
	message_id: String,
) -> Body {
	// This is static for all chunks!
	let created = chrono::Utc::now().timestamp() as u32;
	let mut saw_token = false;
	parse::aws_sse::transform::<universal::StreamResponse>(b, move |f| {
		let res = types::ConverseStreamOutput::deserialize(f).ok()?;
		let mk = |choices: Vec<universal::ChatChoiceStream>, usage: Option<universal::Usage>| {
			Some(universal::StreamResponse {
				id: message_id.clone(),
				model: model.clone(),
				object: "chat.completion.chunk".to_string(),
				system_fingerprint: None,
				service_tier: None,
				created,
				choices,
				usage,
			})
		};

		match res {
			ConverseStreamOutput::ContentBlockDelta(d) => {
				if !saw_token {
					saw_token = true;
					log.non_atomic_mutate(|r| {
						r.first_token = Some(Instant::now());
					});
				}
				let delta = d.delta.map(|delta| {
					let mut dr = universal::StreamResponseDelta::default();
					match delta {
						ContentBlockDelta::ReasoningContent(types::ReasoningContentBlockDelta::Text(t)) => {
							dr.reasoning_content = Some(t);
						},
						// TODO
						ContentBlockDelta::ReasoningContent(_) => {},
						ContentBlockDelta::Text(t) => {
							dr.content = Some(t);
						},
						// TODO
						ContentBlockDelta::ToolUse(_) => {},
					};
					dr
				});
				if let Some(delta) = delta {
					let choice = universal::ChatChoiceStream {
						index: 0,
						logprobs: None,
						delta,
						finish_reason: None,
					};
					mk(vec![choice], None)
				} else {
					None
				}
			},
			ConverseStreamOutput::ContentBlockStart(_) => {
				// TODO support tool calls
				None
			},
			ConverseStreamOutput::ContentBlockStop(_) => {
				// No need to send anything here
				None
			},
			ConverseStreamOutput::MessageStart(start) => {
				// Just send a blob with the role
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta {
						role: Some(match start.role {
							types::Role::Assistant => universal::Role::Assistant,
							types::Role::User => universal::Role::User,
						}),
						..Default::default()
					},
					finish_reason: None,
				};
				mk(vec![choice], None)
			},
			ConverseStreamOutput::MessageStop(stop) => {
				let finish_reason = Some(translate_stop_reason(&stop.stop_reason));

				// Just send a blob with the finish reason
				let choice = universal::ChatChoiceStream {
					index: 0,
					logprobs: None,
					delta: universal::StreamResponseDelta::default(),
					finish_reason,
				};
				mk(vec![choice], None)
			},
			ConverseStreamOutput::Metadata(metadata) => {
				if let Some(usage) = metadata.usage {
					log.non_atomic_mutate(|r| {
						r.output_tokens = Some(usage.output_tokens as u64);
						r.input_tokens_from_response = Some(usage.input_tokens as u64);
						r.total_tokens = Some(usage.total_tokens as u64);
					});

					mk(
						vec![],
						Some(universal::Usage {
							prompt_tokens: usage.input_tokens as u32,
							completion_tokens: usage.output_tokens as u32,
							total_tokens: usage.total_tokens as u32,
							prompt_tokens_details: None,
							completion_tokens_details: None,
						}),
					)
				} else {
					None
				}
			},
		}
	})
}

pub(super) mod types {
	use std::collections::HashMap;

	use bytes::Bytes;
	use serde::{Deserialize, Serialize};

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, Default)]
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
		Image {
			source: String,
			media_type: String,
			data: String,
		},
		ToolResult(ToolResultBlock),
		ToolUse(ToolUseBlock),
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

	#[derive(Clone, Deserialize, Serialize, Debug)]
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
	}
	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "camelCase")]
	#[serde(untagged)]
	pub enum SystemContentBlock {
		Text { text: String },
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
		/// The stop sequences to use.
		#[serde(rename = "stopSequences", skip_serializing_if = "Vec::is_empty")]
		pub stop_sequences: Vec<String>,
		/// Anthropic version (not used for Bedrock)
		#[serde(rename = "anthropicVersion", skip_serializing_if = "Option::is_none")]
		pub anthropic_version: Option<String>,
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

	#[derive(Clone, Serialize, Debug)]
	pub struct ToolConfiguration {
		/// An array of tools that you want to pass to a model.
		pub tools: Vec<Tool>,
		/// If supported by model, forces the model to request a tool.
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
	#[serde(rename_all = "camelCase")]
	pub enum ReasoningContentBlockDelta {
		RedactedContent(#[allow(unused)] Bytes),
		Signature(#[allow(unused)] String),
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
}
