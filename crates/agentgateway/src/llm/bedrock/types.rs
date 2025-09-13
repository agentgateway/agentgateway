//! Bedrock Converse API types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// === REQUEST TYPES ===

/// Bedrock Converse request structure
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseRequest {
	/// Required: Model ID to invoke (URI parameter in actual API)
	#[serde(skip)]
	pub model_id: String,

	/// Optional: Array of messages (max conversation length varies by model)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub messages: Option<Vec<Message>>,

	/// Optional: System prompt content blocks
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<Vec<SystemContentBlock>>,

	/// Optional: Inference configuration parameters
	#[serde(skip_serializing_if = "Option::is_none")]
	pub inference_config: Option<InferenceConfiguration>,

	/// Optional: Tool configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_config: Option<ToolConfiguration>,

	/// Optional: Guardrail configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_config: Option<GuardrailConfiguration>,

	/// Optional: Model-specific additional request fields
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_request_fields: Option<serde_json::Value>,

	/// Optional: Response field paths to include
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_field_paths: Option<Vec<String>>,

	/// Optional: Performance configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,

	/// Optional: Request metadata (string key-value pairs)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub request_metadata: Option<HashMap<String, String>>,
}

/// Message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
	pub role: ConversationRole,
	pub content: Vec<ContentBlock>,
}

/// Conversation roles
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConversationRole {
	User,
	Assistant,
}

// Alias for backward compatibility with existing bedrock.rs code
pub type Role = ConversationRole;

/// CRITICAL: ContentBlock is a strict UNION type - exactly one variant must be set
/// Multiple variants will cause validation errors
/// FORMAT: Tuple variants to match AWS SDK serialization exactly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentBlock {
	/// Text content block
	#[serde(rename = "text")]
	Text(String),

	/// Image content block (Claude 3 only)  
	#[serde(rename = "image")]
	Image(ImageBlock),

	/// Document content block
	#[serde(rename = "document")]
	Document(DocumentBlock),

	/// Tool use content block
	#[serde(rename = "toolUse")]
	ToolUse(ToolUseBlock),

	/// Tool result content block
	#[serde(rename = "toolResult")]
	ToolResult(ToolResultBlock),

	/// Cache point for performance optimization
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),

	/// Reasoning content (for thinking mode)
	#[serde(rename = "reasoningContent")]
	ReasoningContent(ReasoningContentBlock),
}

/// Image content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBlock {
	/// Image format
	pub format: ImageFormat,

	/// Image source
	pub source: ImageSource,
}

/// Supported image formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
	Png,
	Jpeg,
	Gif,
	Webp,
}

/// Image source types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
	/// Base64-encoded image data
	Bytes {
		#[serde(rename = "bytes")]
		data: String, // Base64 encoded
	},

	/// S3 location
	S3Location {
		#[serde(rename = "s3Location")]
		s3_location: S3Location,
	},
}

/// S3 location structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Location {
	pub uri: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub bucket_owner: Option<String>,
}

/// Document content block  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentBlock {
	/// Document name (required, 1-200 chars)
	pub name: String,

	/// Document source (required)
	pub source: DocumentSource,

	/// Document format (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub format: Option<String>,

	/// Citations configuration (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<CitationsConfig>,

	/// Context information (optional)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub context: Option<String>,
}

/// Citations configuration for documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationsConfig {
	/// Whether citations are enabled
	pub enabled: bool,
}

/// Supported document formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
	Pdf,
	Csv,
	Doc,
	Docx,
	Xls,
	Xlsx,
	Html,
	Txt,
	Md,
}

/// Document source types  
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocumentSource {
	/// Base64-encoded document data
	Bytes {
		#[serde(rename = "bytes")]
		data: String,
	},

	/// S3 location
	S3Location {
		#[serde(rename = "s3Location")]
		s3_location: S3Location,
	},
}

/// Tool use content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlock {
	/// Tool use identifier
	pub tool_use_id: String,

	/// Tool name
	pub name: String,

	/// Tool input (JSON object)
	pub input: serde_json::Value,
}

/// Tool result content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultBlock {
	/// Tool use ID this result corresponds to
	pub tool_use_id: String,

	/// Result content blocks - Must use ContentBlock to match Bedrock API
	pub content: Vec<ContentBlock>,

	/// Result status (Claude 3 only)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub status: Option<ToolResultStatus>,
}

/// Tool result status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolResultStatus {
	Success,
	Error,
}

/// Cache point for performance optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePointBlock {
	#[serde(rename = "type")]
	pub cache_type: CachePointType,
}

/// Cache point types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CachePointType {
	Default,
}

/// Reasoning content (chain of thought) - UNION type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContentBlock {
	/// Reasoning text with optional signature
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reasoning_text: Option<ReasoningTextBlock>,

	/// Redacted content (base64-encoded)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub redacted_content: Option<String>,
}

/// Reasoning text block with signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningTextBlock {
	pub text: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub signature: Option<String>,
}

/// System content blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemContentBlock {
	/// Text system content
	#[serde(rename = "text")]
	Text(String),

	/// Cache point in system
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),
}

/// Inference configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfiguration {
	/// Maximum tokens to generate (minimum 1)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens: Option<i32>,

	/// Stop sequences
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_sequences: Option<Vec<String>>,

	/// Temperature (0.0 to 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,

	/// Top-p nucleus sampling (0.0 to 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,
}

/// Tool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfiguration {
	/// Array of tools
	pub tools: Vec<Tool>,

	/// Tool choice configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_choice: Option<ToolChoice>,
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
	/// Cache point
	#[serde(rename = "cachePoint")]
	CachePoint(CachePointBlock),

	/// Tool specification
	#[serde(rename = "toolSpec")]
	ToolSpec(ToolSpecification),
}

/// Tool specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpecification {
	/// Tool name
	pub name: String,

	/// Tool description
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,

	/// Input schema
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_schema: Option<ToolInputSchema>,
}

/// Tool input schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolInputSchema {
	/// JSON schema
	#[serde(rename = "json")]
	Json(serde_json::Value),
}

/// Tool choice options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
	/// Model decides (default)
	Auto(AutoToolChoice),

	/// Must use any tool  
	Any(AnyToolChoice),

	/// Must use specific tool (Claude 3 only)
	Tool(ToolChoiceSpecific),
}

/// Auto tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoToolChoice {
	pub auto: serde_json::Value, // Empty object {}
}

/// Any tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnyToolChoice {
	pub any: serde_json::Value, // Empty object {}
}

/// Specific tool choice (Bedrock format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceSpecific {
	pub tool: ToolChoiceToolSpec,
}

/// Tool specification for specific tool choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceToolSpec {
	pub name: String,
}

/// Guardrail configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailConfiguration {
	/// Guardrail identifier
	pub guardrail_identifier: String,

	/// Guardrail version
	pub guardrail_version: String,

	/// Enable trace output
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trace: Option<String>,
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceConfiguration {
	/// Latency optimization mode
	#[serde(skip_serializing_if = "Option::is_none")]
	pub latency: Option<LatencyMode>,
}

/// Latency optimization modes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LatencyMode {
	Standard,
	Optimized,
}

// === RESPONSE TYPES ===

/// Converse response structure
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseResponse {
	/// Response output
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output: Option<ConverseOutput>,

	/// Stop reason
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_reason: Option<StopReason>,

	/// Token usage
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<TokenUsage>,

	/// Request metrics
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metrics: Option<ConverseMetrics>,

	/// Error message (if this is actually an error response in success format)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub message: Option<String>,

	/// Additional model response fields
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_fields: Option<serde_json::Value>,

	/// Performance configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,

	/// Trace information
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trace: Option<ConverseTrace>,
}

/// Response output types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ConverseOutput {
	/// Message output with nested message field
	Message { message: Message },
}

/// Stop reasons
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
	EndTurn,
	ToolUse,
	MaxTokens,
	StopSequence,
	GuardrailIntervened,
	ContentFiltered,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
	pub input_tokens: u32,
	pub output_tokens: u32,
	pub total_tokens: u32,

	/// Cache-specific token counts
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_read_input_tokens: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_write_input_tokens: Option<u32>,
}

/// Request metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseMetrics {
	/// Latency in milliseconds
	pub latency_ms: u64,
}

/// Trace information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConverseTrace {
	/// Guardrail trace data
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail: Option<serde_json::Value>,
}

// === STREAMING TYPES ===

/// ConverseStream output event types
#[derive(Debug, Clone)]
pub enum ConverseStreamOutput {
	MessageStart(MessageStartEvent),
	ContentBlockStart(ContentBlockStartEvent),
	ContentBlockDelta(ContentBlockDeltaEvent),
	ContentBlockStop(ContentBlockStopEvent),
	MessageStop(MessageStopEvent),
	Metadata(ConverseStreamMetadataEvent),

	// Error events
	InternalServerException(StreamErrorEvent),
	ModelStreamErrorException(StreamErrorEvent),
	ServiceUnavailableException(StreamErrorEvent),
	ThrottlingException(StreamErrorEvent),
	ValidationException(StreamErrorEvent),
}

impl ConverseStreamOutput {
	/// Deserialize from AWS event-stream message
	pub fn deserialize(message: aws_event_stream_parser::Message) -> anyhow::Result<Self> {
		// Extract event type from headers
		let event_type = message
			.headers
			.headers
			.iter()
			.find(|h| h.key.as_str() == ":event-type")
			.and_then(|v| match &v.value {
				aws_event_stream_parser::HeaderValue::String(s) => Some(s.as_str()),
				_ => None,
			})
			.ok_or_else(|| anyhow::anyhow!("Missing :event-type header"))?;

		// Parse body based on event type
		match event_type {
			"messageStart" => Ok(ConverseStreamOutput::MessageStart(serde_json::from_slice(
				&message.body,
			)?)),
			"contentBlockStart" => Ok(ConverseStreamOutput::ContentBlockStart(
				serde_json::from_slice(&message.body)?,
			)),
			"contentBlockDelta" => Ok(ConverseStreamOutput::ContentBlockDelta(
				serde_json::from_slice(&message.body)?,
			)),
			"contentBlockStop" => Ok(ConverseStreamOutput::ContentBlockStop(
				serde_json::from_slice(&message.body)?,
			)),
			"messageStop" => Ok(ConverseStreamOutput::MessageStop(serde_json::from_slice(
				&message.body,
			)?)),
			"metadata" => Ok(ConverseStreamOutput::Metadata(serde_json::from_slice(
				&message.body,
			)?)),

			// Error events
			"internalServerException" => Ok(ConverseStreamOutput::InternalServerException(
				serde_json::from_slice(&message.body)?,
			)),
			"modelStreamErrorException" => Ok(ConverseStreamOutput::ModelStreamErrorException(
				serde_json::from_slice(&message.body)?,
			)),
			"serviceUnavailableException" => Ok(ConverseStreamOutput::ServiceUnavailableException(
				serde_json::from_slice(&message.body)?,
			)),
			"throttlingException" => Ok(ConverseStreamOutput::ThrottlingException(
				serde_json::from_slice(&message.body)?,
			)),
			"validationException" => Ok(ConverseStreamOutput::ValidationException(
				serde_json::from_slice(&message.body)?,
			)),

			unknown => Err(anyhow::anyhow!("Unknown event type: {}", unknown)),
		}
	}
}

/// Message start event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStartEvent {
	pub role: ConversationRole,
}

/// Content block start event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStartEvent {
	pub content_block_index: usize,
	pub start: ContentBlockStart,
}

/// Content block start types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ContentBlockStart {
	ToolUse(ToolUseBlockStart),
	Text(String),      // Usually empty for text blocks
	Reasoning(String), // For reasoning blocks
}

/// Tool use block start
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseBlockStart {
	pub tool_use_id: String,
	pub name: String,
}

/// Content block delta event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockDeltaEvent {
	pub content_block_index: usize,
	pub delta: ContentBlockDelta,
}

/// Content block delta types
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ContentBlockDelta {
	/// Text content delta - matches {"text": "content"}
	Text { text: String },

	/// Tool use input delta - matches {"toolUse": {"input": "..."}}
	ToolUse {
		#[serde(rename = "toolUse")]
		tool_use: ToolUseBlockDelta,
	},

	/// Reasoning content delta
	ReasoningContent(ReasoningContentBlockDelta),

	/// Citations delta
	Citation(CitationsDelta),
}

/// Tool use block delta
#[derive(Debug, Clone, Deserialize)]
pub struct ToolUseBlockDelta {
	/// Incremental JSON input string
	pub input: String,
}

/// Reasoning content delta - UNION type (exactly one variant)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ReasoningContentBlockDelta {
	/// Text reasoning delta
	Text(String),
}

/// Citations delta with location tracking
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationsDelta {
	/// Citation location information
	#[serde(skip_serializing_if = "Option::is_none")]
	pub location: Option<CitationLocation>,

	/// Source content fragments
	#[serde(skip_serializing_if = "Option::is_none")]
	pub source_content: Option<Vec<CitationSourceContentDelta>>,

	/// Citation title
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,

	/// Legacy citations array (backward compatibility)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<Vec<Citation>>,
}

/// Citation location for incremental building
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationLocation {
	/// Start position
	pub start: usize,

	/// End position
	pub end: usize,

	/// Location type (page, character, block, etc.)
	pub location_type: String,
}

/// Citation source content delta
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationSourceContentDelta {
	/// Content fragment
	pub content: String,

	/// Fragment position
	pub position: usize,
}

/// Citation structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
	pub source: String,
	pub content: String,
}

/// Content block stop event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStopEvent {
	pub content_block_index: usize,
}

/// Message stop event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStopEvent {
	pub stop_reason: StopReason,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_response_fields: Option<serde_json::Value>,
}

/// Stream metadata event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConverseStreamMetadataEvent {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<TokenUsage>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub metrics: Option<ConverseMetrics>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub performance_config: Option<PerformanceConfiguration>,
}

// === ERROR TYPES ===

/// Bedrock error response
#[derive(Debug, Clone, Deserialize)]
pub struct ConverseErrorResponse {
	/// Error message
	pub message: String,

	/// Error type (if available)
	#[serde(rename = "__type")]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error_type: Option<String>,
}

/// Stream error event structure
#[derive(Debug, Clone, Deserialize)]
pub struct StreamErrorEvent {
	pub message: String,
}

// === UTILITY IMPLEMENTATIONS ===

impl ConverseRequest {
	/// Create new request with model ID
	pub fn new(model_id: String) -> Self {
		Self {
			model_id,
			messages: None,
			system: None,
			inference_config: None,
			tool_config: None,
			guardrail_config: None,
			additional_model_request_fields: None,
			additional_model_response_field_paths: None,
			performance_config: None,
			request_metadata: None,
		}
	}

	/// Add messages
	pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
		self.messages = Some(messages);
		self
	}

	/// Add system prompt
	pub fn with_system(mut self, system: Vec<SystemContentBlock>) -> Self {
		self.system = Some(system);
		self
	}

	/// Add inference config
	pub fn with_inference_config(mut self, config: InferenceConfiguration) -> Self {
		self.inference_config = Some(config);
		self
	}

	/// Add tool configuration
	pub fn with_tools(mut self, tools: Vec<Tool>, tool_choice: Option<ToolChoice>) -> Self {
		self.tool_config = Some(ToolConfiguration { tools, tool_choice });
		self
	}
}

impl Message {
	/// Create user message with text
	pub fn user_text(text: String) -> Self {
		Self {
			role: ConversationRole::User,
			content: vec![ContentBlock::Text(text)],
		}
	}

	/// Create assistant message with text
	pub fn assistant_text(text: String) -> Self {
		Self {
			role: ConversationRole::Assistant,
			content: vec![ContentBlock::Text(text)],
		}
	}

	/// Create user message with tool result
	pub fn user_tool_result(
		tool_use_id: String,
		content: Vec<ContentBlock>,
		status: Option<ToolResultStatus>,
	) -> Self {
		Self {
			role: ConversationRole::User,
			content: vec![ContentBlock::ToolResult(ToolResultBlock {
				tool_use_id,
				content,
				status,
			})],
		}
	}
}

impl InferenceConfiguration {
	/// Create basic inference config
	pub fn new(max_tokens: i32) -> Self {
		Self {
			max_tokens: Some(max_tokens),
			stop_sequences: None,
			temperature: None,
			top_p: None,
		}
	}
}

/// Helper to create tool specification
pub fn tool_spec(
	name: String,
	description: Option<String>,
	input_schema: Option<serde_json::Value>,
) -> Tool {
	Tool::ToolSpec(ToolSpecification {
		name,
		description,
		input_schema: input_schema.map(ToolInputSchema::Json),
	})
}

/// Helper to create auto tool choice
pub fn auto_tool_choice() -> ToolChoice {
	ToolChoice::Auto(AutoToolChoice {
		auto: serde_json::Value::Object(serde_json::Map::new()),
	})
}

/// Helper to create any tool choice  
pub fn any_tool_choice() -> ToolChoice {
	ToolChoice::Any(AnyToolChoice {
		any: serde_json::Value::Object(serde_json::Map::new()),
	})
}

/// Helper to create specific tool choice
pub fn tool_choice(name: String) -> ToolChoice {
	ToolChoice::Tool(ToolChoiceSpecific {
		tool: ToolChoiceToolSpec { name },
	})
}
