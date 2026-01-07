use agent_core::strng;
use agent_core::strng::Strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::llm::bedrock::Provider;
use crate::llm::policy::webhook::{Message, ResponseChoice};
use crate::llm::types::{ResponseType, SimpleChatCompletionMessage};
use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, conversion};
use crate::{json, llm};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
	pub messages: Vec<RequestMessage>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub frequency_penalty: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub presence_penalty: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub seed: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream_options: Option<StreamOptions>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_completion_tokens: Option<u32>,

	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

/// Options for streaming response. Only set this when you set `stream: true`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StreamOptions {
	/// If set, an additional chunk will be streamed before the `data: [DONE]` message. The `usage` field on this chunk shows the token usage statistics for the entire request, and the `choices` field will always be an empty array. All other chunks will also include a `usage` field, but with a null value.
	pub include_usage: bool,

	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub model: String,
	pub usage: Option<Usage>,
	/// A list of chat completion choices. Can be more than one if `n` is greater than 1.
	pub choices: Vec<Choice>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Choice {
	pub message: ResponseMessage,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ResponseMessage {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub role: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Usage {
	/// Number of tokens in the prompt.
	pub prompt_tokens: u32,
	/// Number of tokens in the generated completion.
	pub completion_tokens: u32,
	/// Total number of tokens used in the request (prompt + completion).
	pub total_tokens: u32,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl ResponseType for Response {
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
		LLMResponse {
			input_tokens: self.usage.as_ref().map(|u| u.prompt_tokens as u64),
			output_tokens: self.usage.as_ref().map(|u| u.completion_tokens as u64),
			total_tokens: self.usage.as_ref().map(|u| u.total_tokens as u64),
			count_tokens: None,
			provider_model: Some(strng::new(&self.model)),
			completion: if include_completion_in_log {
				Some(
					self
						.choices
						.iter()
						.flat_map(|c| c.message.content.clone())
						.collect_vec(),
				)
			} else {
				None
			},
			first_token: Default::default(),
		}
	}

	fn set_webhook_choices(&mut self, choices: Vec<ResponseChoice>) -> anyhow::Result<()> {
		if self.choices.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}
		for (m, wh) in self.choices.iter_mut().zip(choices.into_iter()) {
			m.message.content = Some(wh.message.content.to_string());
		}
		Ok(())
	}

	fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
		self
			.choices
			.iter()
			.map(|c| {
				let role = c.message.role.clone().unwrap_or_default().into();
				let content = c.message.content.clone().unwrap_or_default().into();
				ResponseChoice {
					message: Message { role, content },
				}
			})
			.collect()
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
}

impl super::RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}
	fn prepend_prompts(&mut self, prompts: Vec<llm::types::SimpleChatCompletionMessage>) {
		self
			.messages
			.splice(..0, prompts.into_iter().map(convert_message));
	}

	fn append_prompts(&mut self, prompts: Vec<llm::types::SimpleChatCompletionMessage>) {
		self
			.messages
			.extend(prompts.into_iter().map(convert_message));
	}

	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		conversion::messages::from_completions::translate(self)
	}

	fn to_bedrock(
		&self,
		provider: &Provider,
		headers: Option<&::http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_completions::translate(self, provider, headers, prompt_caching)
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let messages = self.get_messages();
			let tokens = crate::llm::num_tokens_from_messages(&model, &messages)?;
			Some(tokens)
		} else {
			None
		};
		// Pass the original body through
		let llm = LLMRequest {
			input_tokens,
			input_format: InputFormat::Completions,
			request_model: model,
			provider,
			streaming: self.stream.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: self.temperature.map(Into::into),
				top_p: self.top_p.map(Into::into),
				frequency_penalty: self.frequency_penalty.map(Into::into),
				presence_penalty: self.presence_penalty.map(Into::into),
				seed: self.seed,
				max_tokens: self
					.max_completion_tokens
					.or(self.max_tokens)
					.map(Into::into),
				encoding_format: None,
				dimensions: None,
			},
		};
		Ok(llm)
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		self
			.messages
			.iter()
			.map(|m| {
				let content = m
					.content
					.as_ref()
					.and_then(|c| match c {
						Content::Text(t) => Some(strng::new(t)),
						// TODO?
						Content::Array(_) => None,
					})
					.unwrap_or_default();
				SimpleChatCompletionMessage {
					role: strng::new(&m.role),
					content,
				}
			})
			.collect()
	}

	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>) {
		self.messages = messages.into_iter().map(convert_message).collect();
	}
}

fn convert_message(r: SimpleChatCompletionMessage) -> RequestMessage {
	RequestMessage {
		role: r.role.to_string(),
		name: None,
		content: Some(Content::Text(r.content.to_string())),
		rest: Default::default(),
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestMessage {
	pub role: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<Content>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestMessage {
	pub fn message_text(&self) -> Option<&str> {
		self.content.as_ref().and_then(|c| match c {
			Content::Text(t) => Some(t.as_str()),
			// TODO?
			Content::Array(_) => None,
		})
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
	Text(String),
	Array(Vec<ContentPart>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentPart {
	pub r#type: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl TryInto<typed::Request> for &Request {
	type Error = AIError;

	fn try_into(self) -> Result<typed::Request, Self::Error> {
		json::convert::<_, typed::Request>(self).map_err(AIError::RequestMarshal)
	}
}

// 'typed' provides a typed accessor
pub mod typed {
	#![allow(deprecated)]
	#![allow(deprecated_in_future)]

	use std::collections::HashMap;

	use async_openai::types::chat::{
		ChatChoiceLogprobs, ChatCompletionResponseMessageAudio, CompletionUsage,
	};
	#[allow(deprecated)]
	#[allow(deprecated_in_future)]
	pub use async_openai::types::chat::{
		ChatCompletionAudio, ChatCompletionFunctionCall, ChatCompletionFunctions,
		ChatCompletionMessageToolCall as MessageToolCall, ChatCompletionMessageToolCallChunk,
		ChatCompletionMessageToolCalls as MessageToolCalls,
		ChatCompletionNamedToolChoice as NamedToolChoice,
		ChatCompletionRequestAssistantMessage as RequestAssistantMessage,
		ChatCompletionRequestAssistantMessageContent as RequestAssistantMessageContent,
		ChatCompletionRequestDeveloperMessage as RequestDeveloperMessage,
		ChatCompletionRequestDeveloperMessageContent as RequestDeveloperMessageContent,
		ChatCompletionRequestMessage as RequestMessage,
		ChatCompletionRequestSystemMessage as RequestSystemMessage,
		ChatCompletionRequestSystemMessageContent as RequestSystemMessageContent,
		ChatCompletionRequestToolMessage as RequestToolMessage,
		ChatCompletionRequestToolMessageContent as RequestToolMessageContent,
		ChatCompletionRequestToolMessageContentPart as RequestToolMessageContentPart,
		ChatCompletionRequestUserMessage as RequestUserMessage,
		ChatCompletionRequestUserMessageContent as RequestUserMessageContent,
		ChatCompletionStreamOptions as StreamOptions, ChatCompletionTool as FunctionTool,
		ChatCompletionToolChoiceOption as ToolChoiceOption, ChatCompletionToolChoiceOption,
		ChatCompletionTools as Tool, CompletionUsage as Usage, FinishReason, FunctionCall,
		FunctionCallStream, FunctionName, FunctionObject, FunctionType, PredictionContent,
		ReasoningEffort, ResponseFormat, ResponseModalities as ChatCompletionModalities, Role,
		ServiceTier, StopConfiguration as Stop, ToolChoiceOptions, WebSearchOptions,
	};
	use serde::{Deserialize, Serialize};

	/// Represents a chat completion response returned by model, based on the provided input.
	#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
	pub struct Response {
		/// A unique identifier for the chat completion.
		pub id: String,
		/// A list of chat completion choices. Can be more than one if `n` is greater than 1.
		pub choices: Vec<ChatChoice>,
		/// The Unix timestamp (in seconds) of when the chat completion was created.
		pub created: u32,
		/// The model used for the chat completion.
		pub model: String,
		/// The service tier used for processing the request. This field is only included if the `service_tier` parameter is specified in the request.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub service_tier: Option<ServiceTier>,
		/// This fingerprint represents the backend configuration that the model runs with.
		///
		/// Can be used in conjunction with the `seed` request parameter to understand when backend changes have been made that might impact determinism.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub system_fingerprint: Option<String>,

		/// The object type, which is always `chat.completion`.
		pub object: String,
		pub usage: Option<CompletionUsage>,
	}

	#[derive(Debug, Deserialize, Clone, PartialEq, Serialize)]
	/// Represents a streamed chunk of a chat completion response returned by model, based on the provided input.
	pub struct StreamResponse {
		/// A unique identifier for the chat completion. Each chunk has the same ID.
		pub id: String,
		/// A list of chat completion choices. Can contain more than one elements if `n` is greater than 1. Can also be empty for the last chunk if you set `stream_options: {"include_usage": true}`.
		pub choices: Vec<ChatChoiceStream>,

		/// The Unix timestamp (in seconds) of when the chat completion was created. Each chunk has the same timestamp.
		pub created: u32,
		/// The model to generate the completion.
		pub model: String,
		/// The service tier used for processing the request. This field is only included if the `service_tier` parameter is specified in the request.
		pub service_tier: Option<ServiceTier>,
		/// This fingerprint represents the backend configuration that the model runs with.
		/// Can be used in conjunction with the `seed` request parameter to understand when backend changes have been made that might impact determinism.
		pub system_fingerprint: Option<String>,
		/// The object type, which is always `chat.completion.chunk`.
		pub object: String,

		/// An optional field that will only be present when you set `stream_options: {"include_usage": true}` in your request.
		/// When present, it contains a null value except for the last chunk which contains the token usage statistics for the entire request.
		pub usage: Option<CompletionUsage>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
	pub struct ChatChoiceStream {
		/// The index of the choice in the list of choices.
		pub index: u32,
		pub delta: StreamResponseDelta,
		/// The reason the model stopped generating tokens. This will be
		/// `stop` if the model hit a natural stop point or a provided
		/// stop sequence,
		///
		/// `length` if the maximum number of tokens specified in the
		/// request was reached,
		/// `content_filter` if content was omitted due to a flag from our
		/// content filters,
		/// `tool_calls` if the model called a tool, or `function_call`
		/// (deprecated) if the model called a function.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub finish_reason: Option<FinishReason>,
		/// Log probability information for the choice.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub logprobs: Option<ChatChoiceLogprobs>,
	}

	/// A chat completion delta generated by streamed model responses.
	#[derive(Default, Debug, Deserialize, Serialize, Clone, PartialEq)]
	pub struct StreamResponseDelta {
		/// The contents of the chunk message.
		pub content: Option<String>,
		/// Deprecated and replaced by `tool_calls`. The name and arguments of a function that should be called, as generated by the model.
		#[deprecated]
		#[serde(skip_serializing_if = "Option::is_none")]
		pub function_call: Option<FunctionCallStream>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>,
		/// The role of the author of this message.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub role: Option<Role>,
		/// The refusal message generated by the model.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub refusal: Option<String>,

		/// Agentgateway: added reasoning_content
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reasoning_content: Option<String>,

		/// Agentgateway: add opaque passthrough for fields like reasoning, etc that we do not support
		#[serde(flatten)]
		pub extra: Option<serde_json::Value>,
	}

	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
	pub struct ChatChoice {
		/// The index of the choice in the list of choices.
		pub index: u32,
		pub message: ResponseMessage,
		/// The reason the model stopped generating tokens. This will be `stop` if the model hit a natural stop point or a provided stop sequence,
		/// `length` if the maximum number of tokens specified in the request was reached,
		/// `content_filter` if content was omitted due to a flag from our content filters,
		/// `tool_calls` if the model called a tool, or `function_call` (deprecated) if the model called a function.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub finish_reason: Option<FinishReason>,
		/// Log probability information for the choice.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub logprobs: Option<ChatChoiceLogprobs>,
	}

	/// A chat completion message generated by the model.
	#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
	pub struct ResponseMessage {
		/// The contents of the message.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub content: Option<String>,
		/// The refusal message generated by the model.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub refusal: Option<String>,
		/// The tool calls generated by the model, such as function calls.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_calls: Option<Vec<MessageToolCalls>>,

		/// The role of the author of this message.
		pub role: Role,

		/// Deprecated and replaced by `tool_calls`.
		/// The name and arguments of a function that should be called, as generated by the model.
		#[serde(skip_serializing_if = "Option::is_none")]
		#[deprecated]
		pub function_call: Option<FunctionCall>,

		/// If the audio output modality is requested, this object contains data about the audio response from the model. [Learn more](https://platform.openai.com/docs/guides/audio).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub audio: Option<ChatCompletionResponseMessageAudio>,

		/// Agentgateway: add reasoning, which is non-standard.
		///
		/// There is no consistent standard for OpenAI compatible endpoints in how to express 'reasoning'
		/// Deepseek: reasoning_content (https://api-docs.deepseek.com/guides/reasoning_model)
		/// z.ai: reasoning_content (https://docs.z.ai/api-reference/llm/chat-completion#response-message-reasoning-content
		/// OpenRouter: `reasoning` and `reasoning_details` (https://openrouter.ai/docs/use-cases/reasoning-tokens#reasoning_details-array-structure)
		/// LiteLLM: `reasoning_content` and `thinking_blocks` (https://docs.litellm.ai/docs/reasoning_content)
		///
		/// Since 3/4 of these use `reasoning_content`, it seems like a reasonable default.
		/// Note: due to 'extra' below we still get other fields passed through, too; we just won't do anything
		/// specific with them.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reasoning_content: Option<String>,

		/// Agentgateway: add opaque passthrough for fields like reasoning, etc that we do not support
		#[serde(flatten)]
		pub extra: Option<serde_json::Value>,
	}

	#[derive(Clone, Debug, Serialize, Deserialize)]
	pub struct Request {
		/// A list of messages comprising the conversation so far. Depending on the [model](https://platform.openai.com/docs/models) you use, different message types (modalities) are supported, like [text](https://platform.openai.com/docs/guides/text-generation), [images](https://platform.openai.com/docs/guides/vision), and [audio](https://platform.openai.com/docs/guides/audio).
		pub messages: Vec<RequestMessage>, // min: 1

		/// ID of the model to use.
		/// See the [model endpoint compatibility](https://platform.openai.com/docs/models#model-endpoint-compatibility) table for details on which models work with the Chat API.
		/// Agentgateway: translated this to Option<> since the users can override the model.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub model: Option<String>,

		/// Whether or not to store the output of this chat completion request
		///
		/// for use in our [model distillation](https://platform.openai.com/docs/guides/distillation) or [evals](https://platform.openai.com/docs/guides/evals) products.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub store: Option<bool>, // nullable: true, default: false

		/// **o1 models only**
		///
		/// Constrains effort on reasoning for
		/// [reasoning models](https://platform.openai.com/docs/guides/reasoning).
		///
		/// Currently supported values are `low`, `medium`, and `high`. Reducing
		///
		/// reasoning effort can result in faster responses and fewer tokens
		/// used on reasoning in a response.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reasoning_effort: Option<ReasoningEffort>,

		///  Developer-defined tags and values used for filtering completions in the [dashboard](https://platform.openai.com/chat-completions).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub metadata: Option<serde_json::Value>, // nullable: true

		/// Number between -2.0 and 2.0. Positive values penalize new tokens based on their existing frequency in the text so far, decreasing the model's likelihood to repeat the same line verbatim.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub frequency_penalty: Option<f32>, // min: -2.0, max: 2.0, default: 0

		/// Modify the likelihood of specified tokens appearing in the completion.
		///
		/// Accepts a json object that maps tokens (specified by their token ID in the tokenizer) to an associated bias value from -100 to 100.
		/// Mathematically, the bias is added to the logits generated by the model prior to sampling.
		/// The exact effect will vary per model, but values between -1 and 1 should decrease or increase likelihood of selection;
		/// values like -100 or 100 should result in a ban or exclusive selection of the relevant token.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub logit_bias: Option<HashMap<String, serde_json::Value>>, // default: null

		/// Whether to return log probabilities of the output tokens or not. If true, returns the log probabilities of each output token returned in the `content` of `message`.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub logprobs: Option<bool>,

		/// An integer between 0 and 20 specifying the number of most likely tokens to return at each token position, each with an associated log probability. `logprobs` must be set to `true` if this parameter is used.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_logprobs: Option<u8>,

		/// The maximum number of [tokens](https://platform.openai.com/tokenizer) that can be generated in the chat completion.
		///
		/// This value can be used to control [costs](https://openai.com/api/pricing/) for text generated via API.
		/// This value is now deprecated in favor of `max_completion_tokens`, and is
		/// not compatible with [o1 series models](https://platform.openai.com/docs/guides/reasoning).
		#[deprecated]
		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_tokens: Option<u32>,

		/// An upper bound for the number of tokens that can be generated for a completion, including visible output tokens and [reasoning tokens](https://platform.openai.com/docs/guides/reasoning).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub max_completion_tokens: Option<u32>,

		/// How many chat completion choices to generate for each input message. Note that you will be charged based on the number of generated tokens across all of the choices. Keep `n` as `1` to minimize costs.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub n: Option<u8>, // min:1, max: 128, default: 1

		#[serde(skip_serializing_if = "Option::is_none")]
		pub modalities: Option<Vec<ChatCompletionModalities>>,

		/// Configuration for a [Predicted Output](https://platform.openai.com/docs/guides/predicted-outputs),which can greatly improve response times when large parts of the model response are known ahead of time. This is most common when you are regenerating a file with only minor changes to most of the content.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub prediction: Option<PredictionContent>,

		/// Parameters for audio output. Required when audio output is requested with `modalities: ["audio"]`. [Learn more](https://platform.openai.com/docs/guides/audio).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub audio: Option<ChatCompletionAudio>,

		/// Number between -2.0 and 2.0. Positive values penalize new tokens based on whether they appear in the text so far, increasing the model's likelihood to talk about new topics.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub presence_penalty: Option<f32>, // min: -2.0, max: 2.0, default 0

		/// An object specifying the format that the model must output. Compatible with [GPT-4o](https://platform.openai.com/docs/models/gpt-4o), [GPT-4o mini](https://platform.openai.com/docs/models/gpt-4o-mini), [GPT-4 Turbo](https://platform.openai.com/docs/models/gpt-4-and-gpt-4-turbo) and all GPT-3.5 Turbo models newer than `gpt-3.5-turbo-1106`.
		///
		/// Setting to `{ "type": "json_schema", "json_schema": {...} }` enables Structured Outputs which guarantees the model will match your supplied JSON schema. Learn more in the [Structured Outputs guide](https://platform.openai.com/docs/guides/structured-outputs).
		///
		/// Setting to `{ "type": "json_object" }` enables JSON mode, which guarantees the message the model generates is valid JSON.
		///
		/// **Important:** when using JSON mode, you **must** also instruct the model to produce JSON yourself via a system or user message. Without this, the model may generate an unending stream of whitespace until the generation reaches the token limit, resulting in a long-running and seemingly "stuck" request. Also note that the message content may be partially cut off if `finish_reason="length"`, which indicates the generation exceeded `max_tokens` or the conversation exceeded the max context length.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub response_format: Option<ResponseFormat>,

		///  This feature is in Beta.
		/// If specified, our system will make a best effort to sample deterministically, such that repeated requests
		/// with the same `seed` and parameters should return the same result.
		/// Determinism is not guaranteed, and you should refer to the `system_fingerprint` response parameter to monitor changes in the backend.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub seed: Option<i64>,

		/// Specifies the latency tier to use for processing the request. This parameter is relevant for customers subscribed to the scale tier service:
		/// - If set to 'auto', the system will utilize scale tier credits until they are exhausted.
		/// - If set to 'default', the request will be processed using the default service tier with a lower uptime SLA and no latency guarentee.
		/// - When not set, the default behavior is 'auto'.
		///
		/// When this parameter is set, the response body will include the `service_tier` utilized.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub service_tier: Option<ServiceTier>,

		/// Up to 4 sequences where the API will stop generating further tokens.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stop: Option<Stop>,

		/// If set, partial message deltas will be sent, like in ChatGPT.
		/// Tokens will be sent as data-only [server-sent events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events#Event_stream_format)
		/// as they become available, with the stream terminated by a `data: [DONE]` message. [Example Python code](https://cookbook.openai.com/examples/how_to_stream_completions).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream: Option<bool>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub stream_options: Option<StreamOptions>,

		/// What sampling temperature to use, between 0 and 2. Higher values like 0.8 will make the output more random,
		/// while lower values like 0.2 will make it more focused and deterministic.
		///
		/// We generally recommend altering this or `top_p` but not both.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f32>, // min: 0, max: 2, default: 1,

		/// An alternative to sampling with temperature, called nucleus sampling,
		/// where the model considers the results of the tokens with top_p probability mass.
		/// So 0.1 means only the tokens comprising the top 10% probability mass are considered.
		///
		///  We generally recommend altering this or `temperature` but not both.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>, // min: 0, max: 1, default: 1

		/// A list of tools the model may call. Currently, only functions are supported as a tool.
		/// Use this to provide a list of functions the model may generate JSON inputs for. A max of 128 functions are supported.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tools: Option<Vec<Tool>>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_choice: Option<ChatCompletionToolChoiceOption>,

		/// Whether to enable [parallel function calling](https://platform.openai.com/docs/guides/function-calling/parallel-function-calling) during tool use.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub parallel_tool_calls: Option<bool>,

		/// A unique identifier representing your end-user, which can help OpenAI to monitor and detect abuse. [Learn more](https://platform.openai.com/docs/guides/safety-best-practices#end-user-ids).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub user: Option<String>,

		/// This tool searches the web for relevant results to use in a response.
		/// Learn more about the [web search tool](https://platform.openai.com/docs/guides/tools-web-search?api-mode=chat).
		#[serde(skip_serializing_if = "Option::is_none")]
		pub web_search_options: Option<WebSearchOptions>,

		/// Deprecated in favor of `tool_choice`.
		///
		/// Controls which (if any) function is called by the model.
		/// `none` means the model will not call a function and instead generates a message.
		/// `auto` means the model can pick between generating a message or calling a function.
		/// Specifying a particular function via `{"name": "my_function"}` forces the model to call that function.
		///
		/// `none` is the default when no functions are present. `auto` is the default if functions are present.
		#[deprecated]
		#[serde(skip_serializing_if = "Option::is_none")]
		pub function_call: Option<ChatCompletionFunctionCall>,

		/// Deprecated in favor of `tools`.
		///
		/// A list of functions the model may generate JSON inputs for.
		#[deprecated]
		#[allow(deprecated)]
		#[allow(deprecated_in_future)]
		#[serde(skip_serializing_if = "Option::is_none")]
		pub functions: Option<Vec<ChatCompletionFunctions>>,

		/// Agentgateway: vendor specific fields we allow only for internal creation
		#[serde(flatten, skip_deserializing)]
		pub vendor_extensions: RequestVendorExtensions,
	}

	#[derive(Clone, Debug, Serialize, Default)]
	pub struct RequestVendorExtensions {
		/// Anthropic
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_k: Option<usize>,
		/// Anthropic
		#[serde(skip_serializing_if = "Option::is_none")]
		pub thinking_budget_tokens: Option<u64>,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionErrorResponse {
		pub event_id: Option<String>,
		pub error: ChatCompletionError,
	}

	#[derive(Debug, Deserialize, Serialize)]
	pub struct ChatCompletionError {
		pub r#type: String,
		pub message: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub param: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub code: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub event_id: Option<String>,
	}

	pub const SYSTEM_ROLE: &str = "system";
	pub const ASSISTANT_ROLE: &str = "assistant";

	pub fn message_role(msg: &RequestMessage) -> &'static str {
		match msg {
			RequestMessage::Developer(_) => "developer",
			RequestMessage::System(_) => "system",
			RequestMessage::Assistant(_) => "assistant",
			RequestMessage::Tool(_) => "tool",
			RequestMessage::Function(_) => "function",
			RequestMessage::User(_) => "user",
		}
	}

	pub fn message_text(msg: &RequestMessage) -> Option<&str> {
		// All of these types support Vec<Text>... show we support those?
		// Right now, we don't support
		match msg {
			RequestMessage::Developer(RequestDeveloperMessage {
				content: RequestDeveloperMessageContent::Text(t),
				..
			}) => Some(t.as_str()),
			RequestMessage::System(RequestSystemMessage {
				content: RequestSystemMessageContent::Text(t),
				..
			}) => Some(t.as_str()),
			RequestMessage::Assistant(RequestAssistantMessage {
				content: Some(RequestAssistantMessageContent::Text(t)),
				..
			}) => Some(t.as_str()),
			RequestMessage::Tool(RequestToolMessage {
				content: RequestToolMessageContent::Text(t),
				..
			}) => Some(t.as_str()),
			RequestMessage::User(RequestUserMessage {
				content: RequestUserMessageContent::Text(t),
				..
			}) => Some(t.as_str()),
			_ => None,
		}
	}

	impl Request {
		pub fn max_tokens(&self) -> usize {
			self
				.max_completion_tokens
				.or(self.max_tokens)
				.unwrap_or(4096) as usize
		}

		pub fn max_tokens_option(&self) -> Option<u64> {
			self
				.max_completion_tokens
				.or(self.max_tokens)
				.map(Into::into)
		}

		pub fn stop_sequence(&self) -> Vec<String> {
			match &self.stop {
				Some(Stop::String(s)) => vec![s.clone()],
				Some(Stop::StringArray(s)) => s.clone(),
				_ => vec![],
			}
		}
	}
}
