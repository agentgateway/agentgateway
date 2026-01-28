use agent_core::prelude::Strng;
use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::llm::policy::webhook::{Message, ResponseChoice};
use crate::llm::types::{RequestType, ResponseType, SimpleChatCompletionMessage};
use crate::llm::{
	AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, conversion,
	num_tokens_from_anthropic_messages,
};

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	pub messages: Vec<RequestMessage>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<RequestContent>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens: Option<u64>,

	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct RequestMessage {
	pub role: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<RequestContent>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestMessage {
	pub fn message_text(&self) -> Option<&str> {
		self.content.as_ref().and_then(|c| match c {
			RequestContent::Text(t) => Some(t.as_str()),
			// TODO?
			RequestContent::Array(_) => None,
		})
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum RequestContent {
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

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub model: String,
	pub usage: Usage,
	pub content: Vec<Content>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Content {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Usage {
	pub input_tokens: u64,
	pub output_tokens: u64,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let (system_prompts, message_prompts): (Vec<_>, Vec<_>) = prompts
			.into_iter()
			.partition(|p| p.role.as_str() == "system");

		if !system_prompts.is_empty() {
			let mut items: Vec<ContentPart> = match std::mem::take(&mut self.system) {
				Some(RequestContent::Text(text)) => vec![ContentPart {
					r#type: "text".to_string(),
					text: Some(text),
					rest: Default::default(),
				}],
				Some(RequestContent::Array(existing)) => existing,
				None => Vec::new(),
			};

			items.splice(
				0..0,
				system_prompts.into_iter().map(|p| ContentPart {
					r#type: "text".to_string(),
					text: Some(p.content.to_string()),
					rest: Default::default(),
				}),
			);

			self.system = Some(RequestContent::Array(items));
		}

		if !message_prompts.is_empty() {
			self
				.messages
				.splice(..0, message_prompts.into_iter().map(Into::into));
		}
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let (system_prompts, message_prompts): (Vec<_>, Vec<_>) = prompts
			.into_iter()
			.partition(|p| p.role.as_str() == "system");

		if !system_prompts.is_empty() {
			let mut items: Vec<ContentPart> = match std::mem::take(&mut self.system) {
				Some(RequestContent::Text(text)) => vec![ContentPart {
					r#type: "text".to_string(),
					text: Some(text),
					rest: Default::default(),
				}],
				Some(RequestContent::Array(existing)) => existing,
				None => Vec::new(),
			};

			items.extend(system_prompts.into_iter().map(|p| ContentPart {
				r#type: "text".to_string(),
				text: Some(p.content.to_string()),
				rest: Default::default(),
			}));

			self.system = Some(RequestContent::Array(items));
		}

		if !message_prompts.is_empty() {
			self
				.messages
				.extend(message_prompts.into_iter().map(Into::into));
		}
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let tokens = num_tokens_from_anthropic_messages(&model, &self.messages)?;
			Some(tokens)
		} else {
			None
		};
		// Pass the original body through
		let llm = LLMRequest {
			input_tokens,
			input_format: InputFormat::Messages,
			request_model: model,
			provider,
			streaming: self.stream.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: self.temperature.map(Into::into),
				top_p: self.top_p.map(Into::into),
				frequency_penalty: None,
				presence_penalty: None,
				seed: None,
				max_tokens: self.max_tokens,
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
						RequestContent::Text(t) => Some(strng::new(t)),
						// TODO?
						RequestContent::Array(_) => None,
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
		self.messages = messages.into_iter().map(Into::into).collect();
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		conversion::completions::from_messages::translate(self)
	}

	fn to_anthropic(&self) -> Result<Vec<u8>, AIError> {
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_bedrock(
		&self,
		provider: &crate::llm::bedrock::Provider,
		headers: Option<&::http::HeaderMap>,
		_prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		conversion::bedrock::from_messages::translate(self, provider, headers)
	}
}

impl From<SimpleChatCompletionMessage> for RequestMessage {
	fn from(r: SimpleChatCompletionMessage) -> Self {
		RequestMessage {
			role: r.role.to_string(),
			content: Some(RequestContent::Text(r.content.to_string())),
			rest: Default::default(),
		}
	}
}

impl ResponseType for Response {
	fn to_llm_response(&self, include_completion_in_log: bool) -> LLMResponse {
		LLMResponse {
			input_tokens: Some(self.usage.input_tokens),
			output_tokens: Some(self.usage.output_tokens),
			total_tokens: Some(self.usage.output_tokens + self.usage.input_tokens),
			provider_model: Some(strng::new(&self.model)),
			count_tokens: None,
			completion: if include_completion_in_log {
				Some(
					self
						.content
						.iter()
						.flat_map(|c| c.text.clone())
						.collect_vec(),
				)
			} else {
				None
			},
			first_token: Default::default(),
		}
	}

	fn set_webhook_choices(&mut self, choices: Vec<ResponseChoice>) -> anyhow::Result<()> {
		if self.content.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}
		for (m, wh) in self.content.iter_mut().zip(choices.into_iter()) {
			m.text = Some(wh.message.content.to_string());
		}
		Ok(())
	}

	fn to_webhook_choices(&self) -> Vec<ResponseChoice> {
		self
			.content
			.iter()
			.map(|c| {
				let content = c.text.clone().unwrap_or_default();
				ResponseChoice {
					message: Message {
						role: "assistant".into(),
						content: content.into(),
					},
				}
			})
			.collect()
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}
}

// 'typed' provides a typed accessor
pub mod typed {
	use serde::{Deserialize, Deserializer, Serialize};
	use serde_json::Value;

	use crate::serdes::is_default;

	#[derive(Copy, Clone, Deserialize, Serialize, Debug, PartialEq, Eq, Default)]
	#[serde(rename_all = "snake_case")]
	pub enum Role {
		#[default]
		User,
		Assistant,
	}
	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentTextBlock {
		pub text: String,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub citations: Option<Value>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentImageBlock {
		pub source: Value,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentSearchResultBlock {
		pub content: Vec<Value>,
		pub source: String,
		pub title: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct ContentDocumentBlock {
		pub source: Value,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub citations: Option<Value>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub context: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub title: Option<String>,
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ContentBlock {
		Text(ContentTextBlock),
		Image(ContentImageBlock),
		Document(ContentDocumentBlock),
		SearchResult(ContentSearchResultBlock),
		Thinking {
			thinking: String,
			signature: String,
		},
		RedactedThinking {
			data: String,
		},
		/// Tool use content
		ToolUse {
			id: String,
			name: String,
			input: serde_json::Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		/// Tool result content
		ToolResult {
			tool_use_id: String,
			content: ToolResultContent,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
			#[serde(skip_serializing_if = "Option::is_none")]
			is_error: Option<bool>,
		},
		ServerToolUse {
			id: String,
			name: String,
			input: serde_json::Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		/// Web search tool result content
		WebSearchToolResult {
			tool_use_id: String,
			#[serde(skip_serializing_if = "Option::is_none")]
			content: Option<serde_json::Value>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		// There are LOTs of possible values; since we don't support them all, just allow them without failing
		#[serde(other)]
		Unknown,
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	#[serde(untagged)]
	pub enum ToolResultContent {
		/// The text contents of the tool message.
		Text(String),
		/// An array of content parts with a defined type. For tool messages, only type `text` is supported.
		Array(Vec<ToolResultContentPart>),
	}

	#[derive(Debug, Serialize, Deserialize, Clone)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub enum ToolResultContentPart {
		Text {
			text: String,
			#[serde(skip_serializing_if = "Option::is_none")]
			citations: Option<Value>,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		Image {
			source: Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
		Document {
			source: Value,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
			#[serde(skip_serializing_if = "Option::is_none")]
			citations: Option<Value>,
			#[serde(skip_serializing_if = "Option::is_none")]
			context: Option<String>,
			#[serde(skip_serializing_if = "Option::is_none")]
			title: Option<String>,
		},
		SearchResult {
			content: Vec<Value>,
			source: String,
			title: String,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug, PartialEq, Eq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum CacheControlEphemeral {
		Ephemeral {
			#[serde(default)]
			#[serde(skip_serializing_if = "Option::is_none")]
			ttl: Option<String>,
		},
	}

	#[derive(Clone, Deserialize, Serialize, Debug)]
	#[serde(rename_all = "snake_case")]
	pub struct Message {
		pub role: Role,
		#[serde(deserialize_with = "deserialize_content")]
		pub content: Vec<ContentBlock>,
	}

	// Custom deserializer that handles both string and array formats
	fn deserialize_content<'de, D>(deserializer: D) -> Result<Vec<ContentBlock>, D::Error>
	where
		D: Deserializer<'de>,
	{
		use serde::de::Error;
		use serde_json::Value;

		let value = Value::deserialize(deserializer)?;

		match value {
			// If it's a string, wrap it in a Text content block
			Value::String(text) => Ok(vec![ContentBlock::Text(ContentTextBlock {
				text,
				citations: None,
				cache_control: None,
			})]),
			// If it's an array, deserialize normally
			Value::Array(_) => Vec::<ContentBlock>::deserialize(value).map_err(D::Error::custom),
			// Reject other types
			_ => Err(D::Error::custom(
				"content must be either a string or an array",
			)),
		}
	}

	/// System prompt format - can be either a simple string or an array of content blocks
	#[derive(Clone, Debug, Serialize, Deserialize)]
	#[serde(untagged)]
	pub enum SystemPrompt {
		Text(String),
		Blocks(Vec<SystemContentBlock>),
	}

	/// System content block for structured system prompts
	#[derive(Clone, Debug, Serialize, Deserialize)]
	#[serde(tag = "type", rename_all = "snake_case")]
	pub enum SystemContentBlock {
		Text {
			text: String,
			#[serde(skip_serializing_if = "Option::is_none")]
			cache_control: Option<CacheControlEphemeral>,
		},
	}

	#[derive(Deserialize, Serialize, Default, Debug)]
	pub struct Request {
		/// The User/Assistent prompts.
		pub messages: Vec<Message>,
		/// The System prompt - can be a string or array of content blocks
		#[serde(skip_serializing_if = "Option::is_none")]
		pub system: Option<SystemPrompt>,
		/// The model to use.
		pub model: String,
		/// The maximum number of tokens to generate before stopping.
		pub max_tokens: usize,
		/// The stop sequences to use.
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		pub stop_sequences: Vec<String>,
		/// Whether to incrementally stream the response.
		#[serde(default, skip_serializing_if = "is_default")]
		pub stream: bool,
		/// Amount of randomness injected into the response.
		///
		/// Defaults to 1.0. Ranges from 0.0 to 1.0. Use temperature closer to 0.0 for analytical /
		/// multiple choice, and closer to 1.0 for creative and generative tasks. Note that even
		/// with temperature of 0.0, the results will not be fully deterministic.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub temperature: Option<f32>,
		/// Use nucleus sampling.
		///
		/// In nucleus sampling, we compute the cumulative distribution over all the options for each
		/// subsequent token in decreasing probability order and cut it off once it reaches a particular
		/// probability specified by top_p. You should either alter temperature or top_p, but not both.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_p: Option<f32>,
		/// Only sample from the top K options for each subsequent token.
		/// Used to remove "long tail" low probability responses. Learn more technical details here.
		/// Recommended for advanced use cases only. You usually only need to use temperature.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub top_k: Option<usize>,
		/// Tools that the model may use
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tools: Option<Vec<Tool>>,
		/// How the model should use tools
		#[serde(skip_serializing_if = "Option::is_none")]
		pub tool_choice: Option<ToolChoice>,
		/// Request metadata
		#[serde(skip_serializing_if = "Option::is_none")]
		pub metadata: Option<Metadata>,

		#[serde(skip_serializing_if = "Option::is_none")]
		pub thinking: Option<ThinkingInput>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ThinkingInput {
		Enabled { budget_tokens: u64 },
		Disabled {},
	}

	/// Response body for the Messages API.
	#[derive(Debug, Serialize, Deserialize, Clone)]
	pub struct MessagesResponse {
		/// Unique object identifier.
		/// The format and length of IDs may change over time.
		pub id: String,
		/// Object type.
		/// For Messages, this is always "message".
		pub r#type: String,
		/// Conversational role of the generated message.
		/// This will always be "assistant".
		pub role: Role,
		/// Content generated by the model.
		/// This is an array of content blocks, each of which has a type that determines its shape.
		/// Currently, the only type in responses is "text".
		///
		/// Example:
		/// `[{"type": "text", "text": "Hi, I'm Claude."}]`
		///
		/// If the request input messages ended with an assistant turn, then the response content
		/// will continue directly from that last turn. You can use this to constrain the model's
		/// output.
		///
		/// For example, if the input messages were:
		/// `[ {"role": "user", "content": "What's the Greek name for Sun? (A) Sol (B) Helios (C) Sun"},
		///    {"role": "assistant", "content": "The best answer is ("} ]`
		///
		/// Then the response content might be:
		/// `[{"type": "text", "text": "B)"}]`
		pub content: Vec<ContentBlock>,
		/// The model that handled the request.
		pub model: String,
		/// The reason that we stopped.
		/// This may be one the following values:
		/// - "end_turn": the model reached a natural stopping point
		/// - "max_tokens": we exceeded the requested max_tokens or the model's maximum
		/// - "stop_sequence": one of your provided custom stop_sequences was generated
		///
		/// Note that these values are different than those in /v1/complete, where end_turn and
		/// stop_sequence were not differentiated.
		///
		/// In non-streaming mode this value is always non-null. In streaming mode, it is null
		/// in the message_start event and non-null otherwise.
		pub stop_reason: Option<StopReason>,
		/// Which custom stop sequence was generated, if any.
		/// This value will be a non-null string if one of your custom stop sequences was generated.
		pub stop_sequence: Option<String>,
		/// Billing and rate-limit usage.
		/// Anthropic's API bills and rate-limits by token counts, as tokens represent the underlying
		/// cost to our systems.
		///
		/// Under the hood, the API transforms requests into a format suitable for the model. The
		/// model's output then goes through a parsing stage before becoming an API response. As a
		/// result, the token counts in usage will not match one-to-one with the exact visible
		/// content of an API request or response.
		///
		/// For example, output_tokens will be non-zero, even for an empty string response from Claude.
		pub usage: Usage,
	}

	#[derive(Clone, Serialize, Deserialize, Debug)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum MessagesStreamEvent {
		MessageStart {
			message: MessagesResponse,
		},
		ContentBlockStart {
			index: usize,
			content_block: ContentBlock,
		},
		ContentBlockDelta {
			index: usize,
			delta: ContentBlockDelta,
		},
		ContentBlockStop {
			index: usize,
		},
		MessageDelta {
			delta: MessageDelta,
			usage: MessageDeltaUsage,
		},
		MessageStop,
		Ping,
	}

	impl MessagesStreamEvent {
		/// Get the SSE event name for this event type
		#[allow(dead_code)] // Used by Bedrock streaming translation
		pub fn event_name(&self) -> &'static str {
			match self {
				Self::MessageStart { .. } => "message_start",
				Self::ContentBlockStart { .. } => "content_block_start",
				Self::ContentBlockDelta { .. } => "content_block_delta",
				Self::ContentBlockStop { .. } => "content_block_stop",
				Self::MessageDelta { .. } => "message_delta",
				Self::MessageStop => "message_stop",
				Self::Ping => "ping",
			}
		}

		/// Convert to (event_name, self) tuple for transform_multi
		#[allow(dead_code)] // Used by Bedrock streaming translation
		pub fn into_sse_tuple(self) -> (&'static str, Self) {
			let name = self.event_name();
			(name, self)
		}
	}

	// Note: event_name() and into_sse_tuple() are used by Bedrock streaming translation

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	#[allow(clippy::enum_variant_names)]
	pub enum ContentBlockDelta {
		TextDelta {
			text: String,
		},
		InputJsonDelta {
			partial_json: String,
		},
		ThinkingDelta {
			thinking: String,
		},
		SignatureDelta {
			signature: String,
		},
		CitationsDelta {
			#[serde(default)]
			citations: Vec<serde_json::Value>,
		},
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	pub struct MessageDeltaUsage {
		/// Cumulative input tokens
		pub input_tokens: usize,
		/// Cumulative output tokens
		pub output_tokens: usize,
		/// Cumulative cache creation tokens
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_creation_input_tokens: Option<usize>,
		/// Cumulative cache read tokens
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_read_input_tokens: Option<usize>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	pub struct MessageDelta {
		/// The reason that we stopped.
		/// This may be one the following values:
		/// - "end_turn": the model reached a natural stopping point
		/// - "max_tokens": we exceeded the requested max_tokens or the model's maximum
		/// - "stop_sequence": one of your provided custom stop_sequences was generated
		///
		/// Note that these values are different than those in /v1/complete, where end_turn and
		/// stop_sequence were not differentiated.
		///
		/// In non-streaming mode this value is always non-null. In streaming mode, it is null
		/// in the message_start event and non-null otherwise.
		pub stop_reason: Option<StopReason>,
		/// Which custom stop sequence was generated, if any.
		/// This value will be a non-null string if one of your custom stop sequences was generated.
		pub stop_sequence: Option<String>,
	}

	/// Response body for the Messages API.
	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct MessagesErrorResponse {
		pub r#type: String,
		pub error: MessagesError,
	}

	#[derive(Debug, Deserialize, Serialize, Clone)]
	pub struct MessagesError {
		pub r#type: String,
		pub message: String,
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
		/// The model invoked one or more tools.
		ToolUse,
		/// The model's response was refused.
		Refusal,
		/// The model paused generation (for long-running responses).
		PauseTurn,
		/// The model exceeded the context window.
		ModelContextWindowExceeded,
	}

	/// Billing and rate-limit usage.
	#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
	pub struct Usage {
		/// The number of input tokens which were used.
		pub input_tokens: usize,

		/// The number of output tokens which were used.
		pub output_tokens: usize,

		/// The number of input tokens used to create the cache entry.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_creation_input_tokens: Option<usize>,

		/// The number of input tokens read from the cache.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_read_input_tokens: Option<usize>,
	}

	/// Tool definition
	#[derive(Debug, Serialize, Deserialize)]
	pub struct Tool {
		/// Name of the tool
		pub name: String,
		/// Description of the tool
		#[serde(skip_serializing_if = "Option::is_none")]
		pub description: Option<String>,
		/// JSON schema for tool input
		pub input_schema: serde_json::Value,
		/// Create a cache control breakpoint at this content block
		#[serde(skip_serializing_if = "Option::is_none")]
		pub cache_control: Option<CacheControlEphemeral>,
	}

	/// Tool choice configuration
	#[derive(Debug, Serialize, Deserialize)]
	#[serde(tag = "type")]
	pub enum ToolChoice {
		/// Let model choose whether to use tools
		#[serde(rename = "auto")]
		Auto,
		/// Model must use one of the provided tools
		#[serde(rename = "any")]
		Any,
		/// Model must use a specific tool
		#[serde(rename = "tool")]
		Tool { name: String },
		/// Model must not use any tools
		#[serde(rename = "none")]
		None,
	}

	/// Message metadata
	#[derive(Debug, Serialize, Deserialize, Default)]
	pub struct Metadata {
		/// Custom metadata fields
		#[serde(flatten)]
		pub fields: std::collections::HashMap<String, String>,
	}
}
