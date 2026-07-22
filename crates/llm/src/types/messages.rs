use agent_core::prelude::Strng;
use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::types::{
	OutputMessage, OutputMessagePart, RequestType, ResponseType, SimpleChatCompletionMessage,
};
use crate::webhook::{Message, ResponseChoice};
use crate::{AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse};

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct Request {
	pub messages: Vec<RequestMessage>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<TextBlock>,
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

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct RequestMessage {
	pub role: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<ContentBlock>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum ContentBlock {
	Text(String),
	Array(Vec<ContentPart>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentPart {
	Text {
		r#type: String,
		text: String,
		#[serde(flatten, default)]
		rest: serde_json::Value,
	},
	Unknown(serde_json::Value),
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum TextBlock {
	Text(String),
	Array(Vec<TextPart>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextPart {
	Text {
		r#type: String,
		text: String,
		#[serde(flatten, default)]
		rest: serde_json::Value,
	},
	Unknown(serde_json::Value),
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub id: String,
	pub r#type: String,
	pub role: String,
	pub model: String,
	pub stop_reason: Option<String>,
	pub stop_sequence: Option<String>,
	pub usage: Usage,
	pub content: Vec<Content>,
	#[serde(skip)]
	pub input_audio_tokens: Option<u64>,
	#[serde(skip)]
	pub output_audio_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
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
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_creation_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_read_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub service_tier: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

pub fn get_messages_helper(
	messages: &[RequestMessage],
	system: &Option<TextBlock>,
) -> Vec<SimpleChatCompletionMessage> {
	let mut out = Vec::new();
	if let Some(system) = system {
		let content = match system {
			TextBlock::Text(t) => strng::new(t),
			TextBlock::Array(parts) => {
				let text = parts
					.iter()
					.filter_map(|part| match part {
						TextPart::Text { text, .. } => Some(text.as_str()),
						_ => None,
					})
					.fold(String::new(), |mut acc, s| {
						if !acc.is_empty() {
							acc.push('\n');
						}
						acc.push_str(s);
						acc
					});
				strng::new(&text)
			},
		};
		if !content.is_empty() {
			out.push(SimpleChatCompletionMessage {
				role: strng::literal!("system"),
				content,
			});
		}
	}

	out.extend(messages.iter().map(|m| {
		let content = m
			.content
			.as_ref()
			.and_then(|c| match c {
				ContentBlock::Text(t) => Some(strng::new(t)),
				ContentBlock::Array(parts) if !parts.is_empty() => {
					let text = parts
						.iter()
						.filter_map(|part| match part {
							ContentPart::Text { text, .. } => Some(text.as_str()),
							_ => None,
						})
						.fold(String::new(), |mut acc, s| {
							if !acc.is_empty() {
								acc.push(' ');
							}
							acc.push_str(s);
							acc
						});
					Some(strng::new(&text))
				},
				_ => None,
			})
			.unwrap_or_default();
		SimpleChatCompletionMessage {
			role: strng::new(&m.role),
			content,
		}
	}));
	out
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		prepend_prompts_helper(&mut self.messages, &mut self.system, prompts);
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		append_prompts_helper(&mut self.messages, &mut self.system, prompts);
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let messages = self.get_messages();
			let tokens = crate::tokenizer::num_tokens_from_messages(&model, &messages)?;
			Some(tokens)
		} else {
			None
		};
		// Pass the original body through
		let llm = LLMRequest {
			input_tokens,
			input_format: InputFormat::Messages,
			cache_convention: crate::CacheTokenConvention::pending(),
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
			prompt: Default::default(),
			provider_state: None,
		};
		Ok(llm)
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		get_messages_helper(&self.messages, &self.system)
	}

	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>) {
		let (system_prompts, message_prompts): (Vec<_>, Vec<_>) = messages
			.into_iter()
			.partition(|m| m.role.as_str() == "system");

		self.system = if system_prompts.is_empty() {
			None
		} else {
			Some(TextBlock::Array(
				system_prompts
					.into_iter()
					.map(|p| TextPart::Text {
						r#type: "text".to_string(),
						text: p.content.to_string(),
						rest: Default::default(),
					})
					.collect(),
			))
		};
		self.messages = message_prompts.into_iter().map(Into::into).collect();
	}
}

pub fn prepend_prompts_helper(
	messages: &mut Vec<RequestMessage>,
	system: &mut Option<TextBlock>,
	prompts: Vec<SimpleChatCompletionMessage>,
) {
	let (system_prompts, message_prompts): (Vec<_>, Vec<_>) = prompts
		.into_iter()
		.partition(|p| p.role.as_str() == "system");

	if !system_prompts.is_empty() {
		let mut items: Vec<TextPart> = match std::mem::take(system) {
			Some(TextBlock::Array(existing)) => existing,
			Some(TextBlock::Text(text)) => vec![TextPart::Text {
				r#type: "text".to_string(),
				text,
				rest: Default::default(),
			}],
			None => Vec::new(),
		};

		items.splice(
			0..0,
			system_prompts.into_iter().map(|p| TextPart::Text {
				r#type: "text".to_string(),
				text: p.content.to_string(),
				rest: Default::default(),
			}),
		);

		*system = Some(TextBlock::Array(items));
	}

	if !message_prompts.is_empty() {
		messages.splice(..0, message_prompts.into_iter().map(Into::into));
	}
}

pub fn append_prompts_helper(
	messages: &mut Vec<RequestMessage>,
	system: &mut Option<TextBlock>,
	prompts: Vec<SimpleChatCompletionMessage>,
) {
	let (system_prompts, message_prompts): (Vec<_>, Vec<_>) = prompts
		.into_iter()
		.partition(|p| p.role.as_str() == "system");

	if !system_prompts.is_empty() {
		let mut items: Vec<TextPart> = match std::mem::take(system) {
			Some(TextBlock::Text(text)) => vec![TextPart::Text {
				r#type: "text".to_string(),
				text,
				rest: Default::default(),
			}],
			Some(TextBlock::Array(existing)) => existing,
			None => Vec::new(),
		};

		items.extend(system_prompts.into_iter().map(|p| TextPart::Text {
			r#type: "text".to_string(),
			text: p.content.to_string(),
			rest: Default::default(),
		}));

		*system = Some(TextBlock::Array(items));
	}

	if !message_prompts.is_empty() {
		messages.extend(message_prompts.into_iter().map(Into::into));
	}
}

impl From<SimpleChatCompletionMessage> for RequestMessage {
	fn from(r: SimpleChatCompletionMessage) -> Self {
		RequestMessage {
			role: r.role.to_string(),
			content: Some(ContentBlock::Text(r.content.to_string())),
			rest: Default::default(),
		}
	}
}

fn extract_output_message(resp: &Response) -> Option<Vec<OutputMessage>> {
	let mut content = Vec::new();

	for c in &resp.content {
		let typ = c.rest.get("type").and_then(|v| v.as_str());
		if matches!(typ, Some("tool_use" | "server_tool_use")) {
			let id = c.rest.get("id").and_then(|v| v.as_str()).unwrap_or("");
			let name = c.rest.get("name").and_then(|v| v.as_str()).unwrap_or("");
			let arguments = c
				.rest
				.get("input")
				.cloned()
				.unwrap_or(serde_json::Value::Object(Default::default()));
			content.push(OutputMessagePart::ToolCall {
				id: strng::new(id),
				name: strng::new(name),
				arguments,
			});
		}
	}

	if content.is_empty() {
		return None;
	}

	Some(vec![OutputMessage {
		role: strng::new(&resp.role),
		content,
		finish_reason: resp.stop_reason.as_deref().map(strng::new),
	}])
}

impl ResponseType for Response {
	fn to_llm_response(&self, log_content: crate::LogContentFields) -> LLMResponse {
		let output_messages = if log_content.tool_calls {
			extract_output_message(self)
		} else {
			None
		};

		LLMResponse {
			input_tokens: Some(self.usage.input_tokens),
			input_image_tokens: None,
			input_text_tokens: None,
			input_audio_tokens: self.input_audio_tokens,
			output_tokens: Some(self.usage.output_tokens),
			output_image_tokens: None,
			output_text_tokens: None,
			output_audio_tokens: self.output_audio_tokens,
			total_tokens: Some(self.usage.output_tokens + self.usage.input_tokens),
			provider_model: Some(strng::new(&self.model)),
			count_tokens: None,
			reasoning_tokens: None,
			cache_creation_input_tokens: self.usage.cache_creation_input_tokens,
			cached_input_tokens: self.usage.cache_read_input_tokens,
			service_tier: self.usage.service_tier.as_deref().map(Into::into),
			completion: if log_content.completion {
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
			output_messages,
			first_token: Default::default(),
		}
	}

	fn set_webhook_choices(&mut self, choices: Vec<ResponseChoice>) -> anyhow::Result<()> {
		if self.content.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}
		for (m, wh) in self.content.iter_mut().zip(choices) {
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
		System,
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

		#[serde(skip_serializing_if = "Option::is_none")]
		pub output_config: Option<OutputConfig>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, Default)]
	pub struct OutputConfig {
		#[serde(skip_serializing_if = "Option::is_none")]
		pub effort: Option<ThinkingEffort>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub format: Option<OutputFormat>,
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum OutputFormat {
		JsonSchema { schema: serde_json::Value },
	}

	#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case", tag = "type")]
	pub enum ThinkingInput {
		Enabled { budget_tokens: u64 },
		Disabled {},
		Adaptive {},
	}

	#[derive(Clone, Copy, Serialize, Deserialize, Debug, Eq, PartialEq)]
	#[serde(rename_all = "snake_case")]
	pub enum ThinkingEffort {
		Low,
		Medium,
		High,
		Xhigh,
		Max,
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

		// Internal fields not shown to user but used for our internal accounting.
		#[serde(skip)]
		pub input_audio_tokens: Option<usize>,
		#[serde(skip)]
		pub output_audio_tokens: Option<usize>,
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
		pub input_tokens: Option<usize>,
		/// Cumulative output tokens
		pub output_tokens: Option<usize>,
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
	#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

		/// The service tier used to serve the request.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub service_tier: Option<String>,
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
	#[serde(tag = "type", rename_all = "snake_case")]
	pub enum ToolChoice {
		/// Let model choose whether to use tools
		Auto {
			#[serde(default, skip_serializing_if = "Option::is_none")]
			disable_parallel_tool_use: Option<bool>,
		},
		/// Model must use one of the provided tools
		Any {
			#[serde(default, skip_serializing_if = "Option::is_none")]
			disable_parallel_tool_use: Option<bool>,
		},
		/// Model must use a specific tool
		Tool {
			name: String,
			#[serde(default, skip_serializing_if = "Option::is_none")]
			disable_parallel_tool_use: Option<bool>,
		},
		/// Model must not use any tools
		None {},
	}

	/// Message metadata
	#[derive(Debug, Serialize, Deserialize, Default)]
	pub struct Metadata {
		/// Custom metadata fields
		#[serde(flatten)]
		pub fields: std::collections::HashMap<String, String>,
	}

	fn extract_output_message_from_typed(
		resp: &MessagesResponse,
	) -> Option<Vec<super::OutputMessage>> {
		let mut content = Vec::new();

		for block in &resp.content {
			match block {
				ContentBlock::ToolUse {
					id, name, input, ..
				}
				| ContentBlock::ServerToolUse {
					id, name, input, ..
				} => {
					content.push(super::OutputMessagePart::ToolCall {
						id: agent_core::strng::new(id),
						name: agent_core::strng::new(name),
						arguments: input.clone(),
					});
				},
				_ => {},
			}
		}

		if content.is_empty() {
			return None;
		}

		let finish_reason = resp.stop_reason.as_ref().map(|sr| {
			let s = serde_json::to_value(sr)
				.ok()
				.and_then(|v| v.as_str().map(String::from))
				.unwrap_or_default();
			agent_core::strng::new(&s)
		});

		let role_str = match resp.role {
			Role::User => "user",
			Role::Assistant => "assistant",
			Role::System => "system",
		};

		Some(vec![super::OutputMessage {
			role: agent_core::strng::new(role_str),
			content,
			finish_reason,
		}])
	}

	impl super::ResponseType for MessagesResponse {
		fn to_llm_response(&self, log_content: crate::LogContentFields) -> crate::LLMResponse {
			let output_messages = if log_content.tool_calls {
				extract_output_message_from_typed(self)
			} else {
				None
			};

			crate::LLMResponse {
				input_tokens: Some(self.usage.input_tokens as u64),
				input_image_tokens: None,
				input_text_tokens: None,
				input_audio_tokens: self.input_audio_tokens.map(|i| i as u64),
				output_tokens: Some(self.usage.output_tokens as u64),
				output_image_tokens: None,
				output_text_tokens: None,
				output_audio_tokens: self.output_audio_tokens.map(|i| i as u64),
				total_tokens: Some((self.usage.input_tokens + self.usage.output_tokens) as u64),
				reasoning_tokens: None,
				cache_creation_input_tokens: self.usage.cache_creation_input_tokens.map(|i| i as u64),
				cached_input_tokens: self.usage.cache_read_input_tokens.map(|i| i as u64),
				service_tier: self.usage.service_tier.as_deref().map(Into::into),
				provider_model: Some(agent_core::strng::new(&self.model)),
				count_tokens: None,
				completion: if log_content.completion {
					Some(
						self
							.content
							.iter()
							.filter_map(|c| match c {
								ContentBlock::Text(t) => Some(t.text.clone()),
								_ => None,
							})
							.collect(),
					)
				} else {
					None
				},
				output_messages,
				first_token: Default::default(),
			}
		}

		fn set_webhook_choices(
			&mut self,
			choices: Vec<crate::webhook::ResponseChoice>,
		) -> anyhow::Result<()> {
			if self.content.len() != choices.len() {
				anyhow::bail!("webhook response message count mismatch");
			}
			for (block, wh) in self.content.iter_mut().zip(choices) {
				if let ContentBlock::Text(t) = block {
					t.text = wh.message.content.to_string();
				}
			}
			Ok(())
		}

		fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
			self
				.content
				.iter()
				.map(|c| {
					let content = match c {
						ContentBlock::Text(t) => t.text.clone(),
						_ => String::new(),
					};
					crate::webhook::ResponseChoice {
						message: crate::webhook::Message {
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
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::ResponseType;

	fn make_typed_response_with_tool_use() -> typed::MessagesResponse {
		typed::MessagesResponse {
			id: "msg_test".to_string(),
			r#type: "message".to_string(),
			role: typed::Role::Assistant,
			content: vec![
				typed::ContentBlock::Text(typed::ContentTextBlock {
					text: "I'll call the tool.".to_string(),
					citations: None,
					cache_control: None,
				}),
				typed::ContentBlock::ToolUse {
					id: "toolu_01A".to_string(),
					name: "get_weather".to_string(),
					input: serde_json::json!({"location": "San Francisco"}),
					cache_control: None,
				},
				typed::ContentBlock::ServerToolUse {
					id: "srvtoolu_01B".to_string(),
					name: "web_search".to_string(),
					input: serde_json::json!({"query": "rust programming"}),
					cache_control: None,
				},
			],
			model: "claude-sonnet-4-20250514".to_string(),
			stop_reason: Some(typed::StopReason::ToolUse),
			stop_sequence: None,
			usage: typed::Usage {
				input_tokens: 100,
				output_tokens: 50,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				service_tier: None,
			},
			input_audio_tokens: None,
			output_audio_tokens: None,
		}
	}

	#[test]
	fn test_typed_response_output_messages_populated_when_flag_true() {
		let response = make_typed_response_with_tool_use();
		let llm_response = response.to_llm_response(crate::LogContentFields {
			completion: true,
			tool_calls: true,
		});

		let messages = llm_response
			.output_messages
			.expect("output_messages should be Some");
		assert_eq!(messages.len(), 1);
		assert_eq!(messages[0].role.as_str(), "assistant");
		assert_eq!(messages[0].finish_reason.as_deref(), Some("tool_use"));

		let tool_calls = messages[0].tool_calls();
		assert_eq!(tool_calls.len(), 2);

		assert_eq!(tool_calls[0].id.as_str(), "toolu_01A");
		assert_eq!(tool_calls[0].name.as_str(), "get_weather");
		assert_eq!(
			tool_calls[0].arguments,
			serde_json::json!({"location":"San Francisco"})
		);

		assert_eq!(tool_calls[1].id.as_str(), "srvtoolu_01B");
		assert_eq!(tool_calls[1].name.as_str(), "web_search");
		assert_eq!(
			tool_calls[1].arguments,
			serde_json::json!({"query":"rust programming"})
		);
	}

	#[test]
	fn test_typed_response_text_only_omits_output_messages() {
		let response = typed::MessagesResponse {
			id: "msg_test2".to_string(),
			r#type: "message".to_string(),
			role: typed::Role::Assistant,
			content: vec![typed::ContentBlock::Text(typed::ContentTextBlock {
				text: "Hello!".to_string(),
				citations: None,
				cache_control: None,
			})],
			model: "claude-sonnet-4-20250514".to_string(),
			stop_reason: Some(typed::StopReason::EndTurn),
			stop_sequence: None,
			usage: typed::Usage {
				input_tokens: 50,
				output_tokens: 20,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				service_tier: None,
			},
			input_audio_tokens: None,
			output_audio_tokens: None,
		};

		let llm_response = response.to_llm_response(crate::LogContentFields {
			completion: true,
			tool_calls: true,
		});
		assert!(llm_response.output_messages.is_none());
	}

	#[test]
	fn test_weakly_typed_response_output_messages_from_rest() {
		let response = Response {
			id: "msg_test3".to_string(),
			r#type: "message".to_string(),
			role: "assistant".to_string(),
			model: "claude-sonnet-4-20250514".to_string(),
			stop_reason: Some("tool_use".to_string()),
			stop_sequence: None,
			usage: Usage {
				input_tokens: 100,
				output_tokens: 50,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				service_tier: None,
				rest: Default::default(),
			},
			content: vec![
				Content {
					text: Some("I'll call the tool.".to_string()),
					rest: serde_json::json!({"type": "text"}),
				},
				Content {
					text: None,
					rest: serde_json::json!({
						"type": "tool_use",
						"id": "toolu_01A",
						"name": "get_weather",
						"input": {"location": "San Francisco"}
					}),
				},
			],
			input_audio_tokens: None,
			output_audio_tokens: None,
			rest: Default::default(),
		};

		let llm_response = response.to_llm_response(crate::LogContentFields {
			completion: true,
			tool_calls: true,
		});

		let messages = llm_response
			.output_messages
			.expect("output_messages should be Some");
		assert_eq!(messages[0].role.as_str(), "assistant");
		assert_eq!(messages[0].finish_reason.as_deref(), Some("tool_use"));

		assert_eq!(messages[0].content.len(), 1);

		let tool_calls = messages[0].tool_calls();
		assert_eq!(tool_calls.len(), 1);
		assert_eq!(tool_calls[0].id.as_str(), "toolu_01A");
		assert_eq!(tool_calls[0].name.as_str(), "get_weather");
		assert_eq!(
			tool_calls[0].arguments,
			serde_json::json!({"location":"San Francisco"})
		);
	}
}
