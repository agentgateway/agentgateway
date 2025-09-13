//! Anthropic Messages API types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Core request structure for Anthropic Messages API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesRequest {
	/// Required: Model to use (e.g., "claude-sonnet-4-20250514")
	pub model: String,

	/// Required: Maximum tokens to generate (minimum 1)
	pub max_tokens: u32,

	/// Required: Array of input messages
	pub messages: Vec<InputMessage>,

	/// Optional: System prompt (string or array of RequestTextBlock)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<SystemPrompt>,

	/// Optional: Tool definitions
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tools: Option<Vec<Tool>>,

	/// Optional: Tool choice configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_choice: Option<ToolChoice>,

	/// Optional: Enable streaming responses
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream: Option<bool>,

	/// Optional: Temperature (0.0 to 1.0, default 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,

	/// Optional: Top-p (0.0 to 1.0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,

	/// Optional: Top-k (minimum 0)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_k: Option<u32>,

	/// Optional: Stop sequences
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_sequences: Option<Vec<String>>,

	/// Optional: Request metadata
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metadata: Option<RequestMetadata>,

	/// Optional: Thinking configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub thinking: Option<ThinkingConfig>,
}

/// System prompt can be string or array of text blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
	String(String),
	Blocks(Vec<RequestTextBlock>),
}

/// Input message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMessage {
	pub role: MessageRole,
	pub content: MessageContent,
}

/// Message content can be string or array of content blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
	String(String),
	Blocks(Vec<RequestContentBlock>),
}

/// Message roles
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
	User,
	Assistant,
}

/// Request content blocks (input)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestContentBlock {
	Text(RequestTextBlock),
	Image(RequestImageBlock),
	Document(RequestDocumentBlock),
	ToolUse(RequestToolUseBlock),
	ToolResult(RequestToolResultBlock),
	Thinking(RequestThinkingBlock),
	SearchResult(RequestSearchResultBlock),
}

/// Text content block with optional citations and cache control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestTextBlock {
	#[serde(rename = "type", default = "default_text_type")]
	pub block_type: String,
	pub text: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<Vec<Citation>>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Image content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestImageBlock {
	pub source: ImageSource,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Image source types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
	Base64 {
		media_type: String, // "image/jpeg", "image/png", "image/gif", "image/webp"
		data: String,
	},
	Url {
		url: String,
	},
	File {
		file_id: String,
	},
}

/// Document content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestDocumentBlock {
	pub source: DocumentSource,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub context: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<RequestCitationsConfig>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Document source types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentSource {
	Base64Pdf {
		media_type: String, // "application/pdf"
		data: String,
	},
	PlainText {
		media_type: String, // "text/plain"
		data: String,
	},
	ContentBlock {
		content_blocks: Vec<RequestContentBlock>,
	},
	UrlPdf {
		url: String,
	},
	File {
		file_id: String,
	},
}

/// Citations configuration for documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCitationsConfig {
	pub enabled: bool,
}

/// Tool use content block (request)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestToolUseBlock {
	/// Tool use ID (pattern: ^[a-zA-Z0-9_-]+$)
	pub id: String,

	/// Tool name
	pub name: String,

	/// Tool input parameters
	pub input: serde_json::Value,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Tool result content block (request)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestToolResultBlock {
	/// Must match tool_use_id from corresponding tool use
	pub tool_use_id: String,

	/// Result content
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<ToolResultContent>,

	/// Whether result represents an error
	#[serde(skip_serializing_if = "Option::is_none")]
	pub is_error: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Tool result content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
	Text(String),
	Blocks(Vec<RequestContentBlock>),
	Unknown(serde_json::Value),
}

/// Thinking content block (request)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestThinkingBlock {
	pub thinking: String,
	pub signature: String,
}

/// Search result content block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSearchResultBlock {
	pub source: String,
	pub title: String,
	pub content: Vec<RequestTextBlock>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<RequestCitationsConfig>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Citation structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
	/// Citation identifier
	pub id: String,

	/// Citation text or content
	pub content: String,

	/// Optional metadata
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metadata: Option<serde_json::Value>,
}

/// Cache control configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControlEphemeral {
	#[serde(rename = "type")]
	pub cache_type: String, // "ephemeral"

	/// Time-to-live: "5m" or "1h" (default "5m")
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ttl: Option<String>,
}

impl Default for CacheControlEphemeral {
	fn default() -> Self {
		Self {
			cache_type: "ephemeral".to_string(),
			ttl: Some("5m".to_string()),
		}
	}
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
	/// Tool name (1-128 chars, pattern: ^[a-zA-Z0-9_-]+$)
	pub name: String,

	/// Optional tool description
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,

	/// Required: JSON Schema for tool input
	pub input_schema: serde_json::Value,

	/// Optional cache control
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_control: Option<CacheControlEphemeral>,
}

/// Tool choice configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
	/// Let model decide whether to use tools
	Auto {
		#[serde(skip_serializing_if = "Option::is_none")]
		disable_parallel_tool_use: Option<bool>,
	},

	/// Force model to use any available tool
	Any {
		#[serde(skip_serializing_if = "Option::is_none")]
		disable_parallel_tool_use: Option<bool>,
	},

	/// Force model to use specific tool
	Tool {
		name: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		disable_parallel_tool_use: Option<bool>,
	},

	/// Disable all tool usage
	None,
}

/// Thinking configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
	/// Type of thinking ("enabled" or "disabled")
	#[serde(rename = "type")]
	pub thinking_type: String,

	/// Budget tokens for thinking (optional, minimum 1024)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub budget_tokens: Option<u32>,
}

/// Request metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
	/// Optional user ID for tracking
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user_id: Option<String>,

	/// Additional custom fields
	#[serde(flatten)]
	pub additional: HashMap<String, serde_json::Value>,
}

/// Service tier for priority capacity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
	Auto,
	StandardOnly,
}

/// MCP server URL definition for request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMCPServerURLDefinition {
	/// Server name
	pub name: String,

	/// Server type (always "url")
	#[serde(rename = "type")]
	pub server_type: String,

	/// Server URL
	pub url: String,

	/// Optional authorization token
	#[serde(skip_serializing_if = "Option::is_none")]
	pub authorization_token: Option<String>,

	/// Optional tool configuration
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_configuration: Option<RequestMCPServerToolConfiguration>,
}

/// MCP server tool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMCPServerToolConfiguration {
	/// List of allowed tools (empty means all tools allowed)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub allowed_tools: Option<Vec<String>>,

	/// Whether MCP tools are enabled for this server
	#[serde(skip_serializing_if = "Option::is_none")]
	pub enabled: Option<bool>,
}

// === RESPONSE TYPES ===

/// Messages response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesResponse {
	/// Unique message identifier
	pub id: String,

	/// Always "message"
	#[serde(rename = "type", default = "default_message_type")]
	pub r#type: String,

	/// Always "assistant" for responses
	pub role: String,

	/// Response content blocks
	pub content: Vec<ResponseContentBlock>,

	/// Model that handled the request
	pub model: String,

	/// Stop reason
	pub stop_reason: Option<StopReason>,

	/// Custom stop sequence matched (if any)
	pub stop_sequence: Option<String>,

	/// Usage and billing information
	pub usage: Usage,

	/// Container information (if used)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub container: Option<ContainerInfo>,
}

/// Response content blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseContentBlock {
	Text(ResponseTextBlock),
	Thinking(ResponseThinkingBlock),
	RedactedThinking(ResponseRedactedThinkingBlock),
	ToolUse(ResponseToolUseBlock),
}

/// Text response block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTextBlock {
	pub text: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub citations: Option<Vec<Citation>>,
}

/// Thinking response block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseThinkingBlock {
	pub thinking: String,
	pub signature: String,
}

/// Redacted thinking block (when thinking was used but not shown)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRedactedThinkingBlock {
	/// Redacted thinking data
	pub data: String,
}

/// Tool use response block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseToolUseBlock {
	pub id: String,
	pub name: String,
	pub input: serde_json::Value,
}

/// Stop reasons
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
	EndTurn,
	MaxTokens,
	StopSequence,
	ToolUse,
	PauseTurn,
	Refusal,
}

/// Usage and billing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
	pub input_tokens: u32,
	pub output_tokens: u32,

	/// Cache-related token counts
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_creation_input_tokens: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_read_input_tokens: Option<u32>,

	/// Cache creation details
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_creation: Option<CacheCreation>,

	/// Server tool usage
	#[serde(skip_serializing_if = "Option::is_none")]
	pub server_tool_use: Option<ServerToolUsage>,

	/// Service tier used
	#[serde(skip_serializing_if = "Option::is_none")]
	pub service_tier: Option<ServiceTier>,
}

/// Cache creation details - breakdown by TTL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheCreation {
	/// Tokens used for 5-minute ephemeral cache
	pub ephemeral_5m_input_tokens: u32,

	/// Tokens used for 1-hour ephemeral cache  
	pub ephemeral_1h_input_tokens: u32,
}

/// Server tool usage details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerToolUsage {
	/// Number of web search requests made
	pub web_search_requests: u32,
}

/// Container information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
	pub id: String,
	pub status: String,
}

// === STREAMING TYPES ===

/// Streaming events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
	MessageStart {
		message: MessagesResponse,
	},
	ContentBlockStart {
		index: usize,
		content_block: ResponseContentBlock,
	},
	ContentBlockDelta {
		index: usize,
		delta: ContentDelta,
	},
	ContentBlockStop {
		index: usize,
	},
	MessageDelta {
		delta: MessageDelta,
	},
	MessageStop,
	Ping,
	Error {
		error: ErrorResponse,
	},
}

/// Content delta types for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
	TextDelta { text: String },
	InputJsonDelta { partial_json: String },
	ThinkingDelta { thinking: String },
	SignatureDelta { signature: String },
	CitationsDelta { citation: Citation },
}

/// Message-level delta for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDelta {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_reason: Option<StopReason>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_sequence: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<Usage>,
}

// === ERROR TYPES ===

/// Error response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
	#[serde(rename = "type")]
	pub error_type: String,
	pub message: String,
}

/// API error responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesErrorResponse {
	#[serde(rename = "type")]
	pub response_type: String, // "error"
	pub error: ApiError,
}

/// API error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
	#[serde(rename = "type")]
	pub error_type: String, // "invalid_request_error", "rate_limit_error", etc.
	pub message: String,
}

// === TOKEN COUNTING ===

/// Token counting request (uses same schema as Messages but without max_tokens/stream)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCountRequest {
	pub model: String,
	pub messages: Vec<InputMessage>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub system: Option<SystemPrompt>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub tools: Option<Vec<Tool>>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_choice: Option<ToolChoice>,
}

/// Token counting response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCountResponse {
	pub input_tokens: u32,
}

// === UTILITY IMPLEMENTATIONS ===

impl MessagesRequest {
	/// Create a new basic request
	pub fn new(model: String, max_tokens: u32, messages: Vec<InputMessage>) -> Self {
		Self {
			model,
			max_tokens,
			messages,
			system: None,
			tools: None,
			tool_choice: None,
			stream: None,
			temperature: None,
			top_p: None,
			top_k: None,
			stop_sequences: None,
			metadata: None,
			thinking: None,
		}
	}

	/// Add system prompt
	pub fn with_system(mut self, system: SystemPrompt) -> Self {
		self.system = Some(system);
		self
	}

	/// Enable streaming
	pub fn with_streaming(mut self, stream: bool) -> Self {
		self.stream = Some(stream);
		self
	}

	/// Add tools
	pub fn with_tools(mut self, tools: Vec<Tool>, tool_choice: Option<ToolChoice>) -> Self {
		self.tools = Some(tools);
		self.tool_choice = tool_choice;
		self
	}

	/// Add thinking configuration
	pub fn with_thinking(mut self, thinking: ThinkingConfig) -> Self {
		self.thinking = Some(thinking);
		self
	}
}

impl InputMessage {
	/// Create user message with text (shorthand)
	pub fn user_text(text: String) -> Self {
		Self {
			role: MessageRole::User,
			content: MessageContent::String(text),
		}
	}

	/// Create assistant message with text (shorthand)
	pub fn assistant_text(text: String) -> Self {
		Self {
			role: MessageRole::Assistant,
			content: MessageContent::String(text),
		}
	}

	/// Create user message with content blocks
	pub fn user_blocks(blocks: Vec<RequestContentBlock>) -> Self {
		Self {
			role: MessageRole::User,
			content: MessageContent::Blocks(blocks),
		}
	}

	/// Create assistant message with content blocks
	pub fn assistant_blocks(blocks: Vec<RequestContentBlock>) -> Self {
		Self {
			role: MessageRole::Assistant,
			content: MessageContent::Blocks(blocks),
		}
	}
}

impl MessageContent {
	/// Convert to blocks (expanding string to text block if needed)
	pub fn to_blocks(self) -> Vec<RequestContentBlock> {
		match self {
			MessageContent::String(text) => vec![RequestContentBlock::Text(RequestTextBlock {
				block_type: "text".to_string(),
				text,
				citations: None,
				cache_control: None,
			})],
			MessageContent::Blocks(blocks) => blocks,
		}
	}

	/// Get blocks as a slice (for iteration)
	pub fn as_blocks(&self) -> &[RequestContentBlock] {
		match self {
			MessageContent::String(_) => {
				// Return empty slice for string content - callers should use to_blocks() if they need the expanded form
				&[]
			},
			MessageContent::Blocks(blocks) => blocks,
		}
	}

	/// Get first block if available
	pub fn first(&self) -> Option<&RequestContentBlock> {
		match self {
			MessageContent::String(_) => None,
			MessageContent::Blocks(blocks) => blocks.first(),
		}
	}

	/// Iterate over blocks
	pub fn iter(&self) -> impl Iterator<Item = &RequestContentBlock> {
		self.as_blocks().iter()
	}

	/// Iterate over blocks mutably  
	pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, RequestContentBlock> {
		match self {
			MessageContent::String(_) => [].iter_mut(),
			MessageContent::Blocks(blocks) => blocks.iter_mut(),
		}
	}

	/// Check if content is string shorthand
	pub fn is_string(&self) -> bool {
		matches!(self, MessageContent::String(_))
	}

	/// Check if content is blocks
	pub fn is_blocks(&self) -> bool {
		matches!(self, MessageContent::Blocks(_))
	}
}

/// Helper to create cache control
pub fn ephemeral_cache(ttl: Option<String>) -> CacheControlEphemeral {
	CacheControlEphemeral {
		cache_type: "ephemeral".to_string(),
		ttl,
	}
}

/// Helper to create 5-minute cache
pub fn ephemeral_cache_5m() -> CacheControlEphemeral {
	ephemeral_cache(Some("5m".to_string()))
}

/// Helper to create 1-hour cache
pub fn ephemeral_cache_1h() -> CacheControlEphemeral {
	ephemeral_cache(Some("1h".to_string()))
}

/// Helper to create auto tool choice
pub fn auto_tool_choice(disable_parallel: Option<bool>) -> ToolChoice {
	ToolChoice::Auto {
		disable_parallel_tool_use: disable_parallel,
	}
}

/// Default message type for responses
fn default_message_type() -> String {
	"message".to_string()
}

/// Default text block type
fn default_text_type() -> String {
	"text".to_string()
}
