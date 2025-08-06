use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;
use chrono;
use serde::Serialize;
use tracing::trace;

use crate::llm::bedrock::types::{ConverseErrorResponse, ConverseRequest, ConverseResponse};
use crate::llm::universal::ChatCompletionRequest;
use crate::llm::{AIError, universal};
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
		mut req: universal::ChatCompletionRequest,
	) -> Result<ConverseRequest, AIError> {
		// Use provider's model if configured, otherwise keep the request model
		if let Some(provider_model) = &self.model {
			req.model = provider_model.to_string();
		}
		let bedrock_request = translate_request(req, self);

		Ok(bedrock_request)
	}

	pub async fn process_response(
		&self,
		model: Strng,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionResponse, AIError> {
		let resp =
			serde_json::from_slice::<ConverseResponse>(bytes).map_err(AIError::ResponseParsing)?;

		// Bedrock response doesn't contain the model, so we pass through the model from the request into the response
		translate_response(resp, model.as_str())
	}

	pub async fn process_error(
		&self,
		bytes: &Bytes,
	) -> Result<universal::ChatCompletionErrorResponse, AIError> {
		let resp =
			serde_json::from_slice::<ConverseErrorResponse>(bytes).map_err(AIError::ResponseParsing)?;
		translate_error(resp)
	}

	pub fn get_path_for_model(&self, model: &str) -> Strng {
		strng::format!("/model/{}/converse", model)
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
) -> Result<universal::ChatCompletionResponse, AIError> {
	// Get the output content from the response
	let output = resp.output.ok_or(AIError::IncompleteResponse)?;

	// Extract the message from the output
	let message = match output {
		types::ConverseOutput::Message(msg) => msg,
		types::ConverseOutput::Unknown => return Err(AIError::IncompleteResponse),
	};

	// Convert Bedrock content blocks to OpenAI message content
	let choices = message
		.content
		.iter()
		.filter_map(|block| {
			let text = match block {
				types::ContentBlock::Text(text) => Some(text.clone()),
				types::ContentBlock::Image { .. } => return None, // Skip images in response for now
			};
			let message = universal::ChatCompletionMessageForResponse {
				role: universal::MessageRole::assistant,
				content: text,
				reasoning_content: None,
				name: None,
				tool_calls: None,
			};
			let finish_reason = Some(match resp.stop_reason {
				types::StopReason::EndTurn => universal::FinishReason::stop,
				types::StopReason::MaxTokens => universal::FinishReason::length,
				types::StopReason::StopSequence => universal::FinishReason::stop,
			});
			// Only one choice for Bedrock
			let choice = universal::ChatCompletionChoice {
				index: 0,
				message,
				finish_reason,
				finish_details: None,
			};
			Some(choice)
		})
		.collect::<Vec<_>>();

	// Convert usage from Bedrock format to OpenAI format
	let usage = if let Some(token_usage) = resp.usage {
		universal::Usage {
			prompt_tokens: token_usage.input_tokens as i32,
			completion_tokens: token_usage.output_tokens as i32,
			total_tokens: token_usage.total_tokens as i32,
		}
	} else {
		// Fallback if usage is not provided
		universal::Usage {
			prompt_tokens: 0,
			completion_tokens: 0,
			total_tokens: 0,
		}
	};

	// Generate a unique ID since it's not provided in the response
	let id = format!("bedrock-{}", chrono::Utc::now().timestamp_millis());

	// Log guardrail trace information if present
	if let Some(trace) = &resp.trace {
		if let Some(guardrail_trace) = &trace.guardrail {
			trace!("Bedrock guardrail trace: {:?}", guardrail_trace);
		}
	}

	Ok(universal::ChatCompletionResponse {
		id: Some(id),
		object: "chat.completion".to_string(),
		created: chrono::Utc::now().timestamp(),
		model: model.to_string(),
		choices,
		usage,
		system_fingerprint: None,
	})
}

pub(super) fn translate_request(
	req: ChatCompletionRequest,
	provider: &Provider,
) -> types::ConverseRequest {
	// Bedrock has system prompts in a separate field. Join them
	let system = req
		.messages
		.iter()
		.filter_map(|msg| {
			if msg.role == universal::MessageRole::system {
				match &msg.content {
					universal::Content::Text(text) => Some(text.clone()),
					_ => None, // Skip non-text system messages
				}
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
		.filter(|msg| msg.role != universal::MessageRole::system)
		.map(|msg| {
			let role = match msg.role {
				universal::MessageRole::user => types::Role::User,
				universal::MessageRole::assistant => types::Role::Assistant,
				_ => types::Role::User, // Default to user for other roles
			};

			let content = match &msg.content {
				universal::Content::Text(text) => {
					vec![types::ContentBlock::Text(text.clone())]
				},
				universal::Content::ImageUrl(urls) => {
					urls
						.iter()
						.map(|img_url| {
							if let Some(url) = &img_url.image_url {
								types::ContentBlock::Image {
									source: url.url.clone(),
									media_type: "image/jpeg".to_string(), // Default to JPEG
									data: "".to_string(),                 // Base64 data would go here if using base64
								}
							} else {
								types::ContentBlock::Text(img_url.text.clone().unwrap_or_default())
							}
						})
						.collect()
				},
			};

			types::Message { role, content }
		})
		.collect();

	// Build inference configuration
	let inference_config = types::InferenceConfiguration {
		max_tokens: req.max_tokens.unwrap_or(4096) as usize,
		temperature: req.temperature,
		top_p: req.top_p,
		stop_sequences: req.stop.unwrap_or_default(),
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

	types::ConverseRequest {
		model_id: req.model,
		messages,
		system: if system.is_empty() {
			None
		} else {
			Some(vec![types::SystemContentBlock::Text { text: system }])
		},
		inference_config: Some(inference_config),
		tool_config: None, // TODO: Add tool support
		guardrail_config,
		additional_model_request_fields: None,
		prompt_variables: None,
		additional_model_response_field_paths: None,
		request_metadata: None,
		performance_config: None,
	}
}

pub(super) mod types {
	use std::collections::HashMap;

	use serde::{Deserialize, Serialize};

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Default)]
	#[serde(rename_all = "snake_case")]
	pub enum Role {
		#[default]
		User,
		Assistant,
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum ContentBlock {
		Text(String),
		Image {
			source: String,
			media_type: String,
			data: String,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	#[serde(untagged)]
	pub enum SystemContentBlock {
		Text { text: String },
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
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
		pub temperature: Option<f64>,
		/// Use nucleus sampling.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f64>,
		/// The stop sequences to use.
		#[serde(rename = "stopSequences", skip_serializing_if = "Vec::is_empty")]
		pub stop_sequences: Vec<String>,
		/// Anthropic version (not used for Bedrock)
		#[serde(rename = "anthropicVersion", skip_serializing_if = "Option::is_none")]
		pub anthropic_version: Option<String>,
	}

	#[derive(Clone, Serialize, Debug, PartialEq)]
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

	#[derive(Clone, Serialize, Debug, PartialEq)]
	pub struct ToolConfiguration {
		// TODO: Implement tool configuration
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

	#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
	pub struct PerformanceConfiguration {
		// TODO: Implement performance configuration
	}

	/// The actual response from the Bedrock Converse API (matches AWS SDK ConverseOutput)
	#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
	pub struct ConverseResponse {
		/// The result from the call to Converse
		pub output: Option<ConverseOutput>,
		/// The reason why the model stopped generating output
		#[serde(rename = "stopReason")]
		pub stop_reason: StopReason,
		/// The total number of tokens used in the call to Converse
		pub usage: Option<TokenUsage>,
		/// Metrics for the call to Converse
		pub metrics: Option<ConverseMetrics>,
		/// Additional fields in the response that are unique to the model
		#[serde(rename = "additionalModelResponseFields")]
		pub additional_model_response_fields: Option<serde_json::Value>,
		/// A trace object that contains information about the Guardrail behavior
		pub trace: Option<ConverseTrace>,
		/// Model performance settings for the request
		#[serde(rename = "performanceConfig")]
		pub performance_config: Option<PerformanceConfiguration>,
	}

	#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
	pub struct ConverseErrorResponse {
		pub message: String,
	}

	/// The actual content output from the model
	#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum ConverseOutput {
		Message(Message),
		#[serde(other)]
		Unknown,
	}

	/// Token usage information
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	pub struct ConverseMetrics {
		/// Latency in milliseconds
		#[serde(rename = "latencyMs")]
		pub latency_ms: u64,
	}

	/// Trace information for Guardrail behavior
	#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	pub struct ConverseTrace {
		/// Guardrail trace information
		#[serde(rename = "guardrail", skip_serializing_if = "Option::is_none")]
		pub guardrail: Option<serde_json::Value>,
	}

	/// Reason for stopping the response generation.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	#[serde(rename_all = "snake_case")]
	pub enum StopReason {
		/// The model reached a natural stopping point.
		EndTurn,
		/// The requested max_tokens or the model's maximum was exceeded.
		MaxTokens,
		/// One of the provided custom stop_sequences was generated.
		StopSequence,
	}
}
