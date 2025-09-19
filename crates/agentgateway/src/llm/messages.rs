//! Anthropic Messages API implementation
//!
//! This module provides direct Anthropic Messages API support with proper ingress/egress
//! transformation to Universal format for policy processing and provider routing.
//! 
//! # Architecture
//! - `types`: Wire structs for Messages API requests/responses and stream events
//! - `ingress`: Messages -> Universal conversion (request normalizer + validation) 
//! - `egress`: Universal -> Messages conversion (JSON & SSE encoders)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod types {
    //! Anthropic Messages API wire format types
    use super::*;
    
    // ===== REQUEST TYPES =====

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
        Image(Box<RequestImageBlock>),
        Document(Box<RequestDocumentBlock>),
        ToolUse(Box<RequestToolUseBlock>),
        ToolResult(Box<RequestToolResultBlock>),
        Thinking(Box<RequestThinkingBlock>),
        SearchResult(Box<RequestSearchResultBlock>),
    }

    /// Content block type
    #[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
    #[serde(rename_all = "lowercase")]
    pub enum ContentBlockType {
        #[default]
        Text,
    }

    /// Text content block with optional citations and cache control
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestTextBlock {
        #[serde(rename = "type", default)]
        pub block_type: ContentBlockType,
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
            media_type: String,
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
            media_type: String,
            data: String,
        },
        PlainText {
            media_type: String,
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
        pub id: String,
        pub name: String,
        pub input: serde_json::Value,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache_control: Option<CacheControlEphemeral>,
    }

    /// Tool result content block (request)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestToolResultBlock {
        pub tool_use_id: String,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub content: Option<ToolResultContent>,

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
        pub id: String,
        pub content: String,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub metadata: Option<serde_json::Value>,
    }

    /// Cache control configuration
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    pub enum CacheType {
        Ephemeral,
    }

    /// Cache control for ephemeral caching
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CacheControlEphemeral {
        #[serde(rename = "type")]
        pub cache_type: CacheType,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub ttl: Option<String>,
    }

    impl Default for CacheControlEphemeral {
        fn default() -> Self {
            Self {
                cache_type: CacheType::Ephemeral,
                ttl: Some("5m".to_string()),
            }
        }
    }

    /// Tool definition
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Tool {
        pub name: String,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub description: Option<String>,

        pub input_schema: serde_json::Value,

        #[serde(skip_serializing_if = "Option::is_none")]  
        pub cache_control: Option<CacheControlEphemeral>,
    }

    /// Tool choice configuration
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ToolChoice {
        Auto {
            #[serde(skip_serializing_if = "Option::is_none")]
            disable_parallel_tool_use: Option<bool>,
        },
        Any {
            #[serde(skip_serializing_if = "Option::is_none")]
            disable_parallel_tool_use: Option<bool>,
        },
        Tool {
            name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            disable_parallel_tool_use: Option<bool>,
        },
        None,
    }

    /// Thinking configuration type
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ThinkingType {
        Enabled,
        Disabled,
    }

    /// Thinking configuration
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ThinkingConfig {
        #[serde(rename = "type")]
        pub thinking_type: ThinkingType,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub budget_tokens: Option<u32>,
    }

    /// Request metadata
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RequestMetadata {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub user_id: Option<String>,

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

    // ===== RESPONSE TYPES =====

    /// Messages response structure
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MessagesResponse {
        pub id: String,

        #[serde(rename = "type", default = "default_message_type")]
        pub r#type: String,

        pub role: String,
        pub content: Vec<ResponseContentBlock>,
        pub model: String,
        pub stop_reason: Option<StopReason>,
        pub stop_sequence: Option<String>,
        pub usage: Usage,

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

    /// Redacted thinking block
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ResponseRedactedThinkingBlock {
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

        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache_creation_input_tokens: Option<u32>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache_read_input_tokens: Option<u32>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache_creation: Option<CacheCreation>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub server_tool_use: Option<ServerToolUsage>,

        #[serde(skip_serializing_if = "Option::is_none")]
        pub service_tier: Option<ServiceTier>,
    }

    /// Cache creation details
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CacheCreation {
        pub ephemeral_5m_input_tokens: u32,
        pub ephemeral_1h_input_tokens: u32,
    }

    /// Server tool usage details
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ServerToolUsage {
        pub web_search_requests: u32,
    }

    /// Container information
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ContainerInfo {
        pub id: String,
        pub status: String,
    }

    // ===== STREAMING TYPES =====

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

    // ===== ERROR TYPES =====

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
        pub response_type: String,
        pub error: ApiError,
    }

    /// API error details
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ApiError {
        #[serde(rename = "type")]
        pub error_type: String,
        pub message: String,
    }

    // ===== UTILITY IMPLEMENTATIONS =====

    impl MessagesRequest {
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

        pub fn with_system(mut self, system: SystemPrompt) -> Self {
            self.system = Some(system);
            self
        }

        pub fn with_streaming(mut self, stream: bool) -> Self {
            self.stream = Some(stream);
            self
        }

        pub fn with_tools(mut self, tools: Vec<Tool>, tool_choice: Option<ToolChoice>) -> Self {
            self.tools = Some(tools);
            self.tool_choice = tool_choice;
            self
        }

        pub fn with_thinking(mut self, thinking: ThinkingConfig) -> Self {
            self.thinking = Some(thinking);
            self
        }
    }

    impl InputMessage {
        pub fn user_text(text: String) -> Self {
            Self {
                role: MessageRole::User,
                content: MessageContent::String(text),
            }
        }

        pub fn assistant_text(text: String) -> Self {
            Self {
                role: MessageRole::Assistant,
                content: MessageContent::String(text),
            }
        }

        pub fn user_blocks(blocks: Vec<RequestContentBlock>) -> Self {
            Self {
                role: MessageRole::User,
                content: MessageContent::Blocks(blocks),
            }
        }

        pub fn assistant_blocks(blocks: Vec<RequestContentBlock>) -> Self {
            Self {
                role: MessageRole::Assistant,
                content: MessageContent::Blocks(blocks),
            }
        }
    }

    impl MessageContent {
        pub fn to_blocks(self) -> Vec<RequestContentBlock> {
            match self {
                MessageContent::String(text) => vec![RequestContentBlock::Text(RequestTextBlock {
                    block_type: ContentBlockType::Text,
                    text,
                    citations: None,
                    cache_control: None,
                })],
                MessageContent::Blocks(blocks) => blocks,
            }
        }

        pub fn as_blocks(&self) -> &[RequestContentBlock] {
            match self {
                MessageContent::String(_) => &[],
                MessageContent::Blocks(blocks) => blocks,
            }
        }

        pub fn is_string(&self) -> bool {
            matches!(self, MessageContent::String(_))
        }

        pub fn is_blocks(&self) -> bool {
            matches!(self, MessageContent::Blocks(_))
        }
    }

    fn default_message_type() -> String {
        "message".to_string()
    }
}

pub mod ingress {
    //! Messages -> Universal conversion
    use super::*;
    use crate::llm::{universal, AIError};
    use http::HeaderMap;
    use std::collections::{HashMap, HashSet};
    use serde_json::{json, Map, Value};

    // ===== VALIDATION FUNCTIONS (FAIL-FAST 400 ERRORS) =====

    /// Comprehensive validation of Messages API request
    /// Returns first validation error encountered (fail-fast)
    pub fn validate_messages_request(m: &types::MessagesRequest) -> Result<(), AIError> {
        // 1. Basic structure validation
        validate_basic_structure(m)?;
        
        // 2. Role validation - Messages API only supports user|assistant
        validate_message_roles(m)?;
        
        // 3. Max tokens validation - required by Anthropic/Bedrock
        validate_max_tokens(m)?;
        
        // 4. Message sequence validation
        validate_message_sequences(m)?;
        
        // 5. Content validation within messages
        validate_message_content(m)?;
        
        // 6. Tool definition validation (if tools present)
        if let Some(tools) = &m.tools {
            validate_tool_definitions(tools)?;
        }
        
        // 7. Tool use/result pairing validation
        validate_tool_pairing(m)?;
        
        Ok(())
    }

    /// Validate basic request structure
    fn validate_basic_structure(m: &types::MessagesRequest) -> Result<(), AIError> {
        if m.messages.is_empty() {
            return Err(AIError::EmptyMessages);
        }
        Ok(())
    }

    /// Validate that all message roles are user|assistant only
    fn validate_message_roles(m: &types::MessagesRequest) -> Result<(), AIError> {
        for (_i, message) in m.messages.iter().enumerate() {
            match message.role {
                types::MessageRole::User | types::MessageRole::Assistant => {},
                // This shouldn't happen due to enum constraints, but explicit check for robustness
            }
        }
        Ok(())
    }

    /// Validate max_tokens field
    /// Decision: max_tokens is required in MessagesRequest, validate it's > 0
    fn validate_max_tokens(m: &types::MessagesRequest) -> Result<(), AIError> {
        if m.max_tokens == 0 {
            return Err(AIError::InvalidMaxTokens(m.max_tokens));
        }
        Ok(())
    }

    /// Validate message sequences (relaxed validation)
    fn validate_message_sequences(m: &types::MessagesRequest) -> Result<(), AIError> {
        if m.messages.is_empty() {
            return Ok(()); // Already checked in basic validation
        }

        // Only warn about assistant-first conversations in logs, don't reject
        if let Some(first) = m.messages.first() {
            if first.role == types::MessageRole::Assistant {
                tracing::warn!("Conversation starting with assistant message - this is unusual but allowed");
            }
        }

        // No restrictions on consecutive messages - users may have valid use cases
        Ok(())
    }

    /// Validate message content is not empty
    fn validate_message_content(m: &types::MessagesRequest) -> Result<(), AIError> {
        for message in m.messages.iter() {
            match &message.content {
                types::MessageContent::String(text) => {
                    if text.trim().is_empty() {
                        return Err(AIError::EmptyMessageContent);
                    }
                },
                types::MessageContent::Blocks(blocks) => {
                    if blocks.is_empty() {
                        return Err(AIError::EmptyMessageContent);
                    }
                    
                    // Check that at least one block has substantive content
                    let has_content = blocks.iter().any(|block| match block {
                        types::RequestContentBlock::Text(text_block) => !text_block.text.trim().is_empty(),
                        types::RequestContentBlock::Image(_) => true,
                        types::RequestContentBlock::Document(_) => true,
                        types::RequestContentBlock::ToolUse(_) => true,
                        types::RequestContentBlock::ToolResult(_) => true,
                        types::RequestContentBlock::Thinking(thinking) => !thinking.thinking.trim().is_empty(),
                        types::RequestContentBlock::SearchResult(_) => true,
                    });
                    
                    if !has_content {
                        return Err(AIError::EmptyMessageContent);
                    }
                }
            }
        }
        Ok(())
    }

    /// Validate tool definitions
    fn validate_tool_definitions(tools: &[types::Tool]) -> Result<(), AIError> {
        if tools.is_empty() {
            return Ok(());
        }

        let mut tool_names = HashSet::new();
        
        for tool in tools {
            // Check for duplicate tool names
            if !tool_names.insert(&tool.name) {
                return Err(AIError::DuplicateToolName(tool.name.clone()));
            }
            
            // Validate tool name is a valid identifier
            if !is_valid_tool_name(&tool.name) {
                return Err(AIError::InvalidToolName(tool.name.clone()));
            }
            
            // Validate input_schema is a proper JSON schema object
            if !tool.input_schema.is_object() {
                return Err(AIError::InvalidToolDefinition(format!(
                    "tool '{}' input_schema must be a JSON object", tool.name
                )));
            }
            
            // Basic schema validation - should have type property
            if let Some(schema_obj) = tool.input_schema.as_object() {
                if !schema_obj.contains_key("type") {
                    return Err(AIError::InvalidToolDefinition(format!(
                        "tool '{}' input_schema missing required 'type' property", tool.name
                    )));
                }
            }
        }
        
        Ok(())
    }

    /// Check if tool name is valid identifier (alphanumeric + underscore, not starting with number)
    fn is_valid_tool_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        
        let first_char = name.chars().next().unwrap();
        if first_char.is_ascii_digit() {
            return false;
        }
        
        name.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    /// Validate tool use/result pairing with strict Anthropic/Bedrock compliance
    /// Rules:
    /// 1. Every tool_use.id from the last assistant must appear exactly once in the next user message
    /// 2. The next user message must not contain any tool_result ids from older turns  
    /// 3. The count of tool_result blocks must equal the number of prior tool_use blocks
    /// 4. No tool_result blocks in any later user messages
    fn validate_tool_pairing(m: &types::MessagesRequest) -> Result<(), AIError> {
        if m.messages.is_empty() {
            return Ok(());
        }

        // 1) Collect tool_use ids from the LAST assistant turn
        let mut last_tool_uses: Vec<String> = Vec::new();
        let mut last_assistant_idx: Option<usize> = None;
        for (idx, message) in m.messages.iter().enumerate().rev() {
            match message.role {
                types::MessageRole::Assistant => {
                    last_assistant_idx = Some(idx);
                    if let types::MessageContent::Blocks(blocks) = &message.content {
                        for block in blocks {
                            if let types::RequestContentBlock::ToolUse(tu) = block {
                                last_tool_uses.push(tu.id.clone());
                            }
                        }
                    }
                    break; // stop at the last assistant encountered
                }
                types::MessageRole::User => break
            }
        }
        if last_tool_uses.is_empty() {
            return Ok(());
        }

        // 2) Inspect the IMMEDIATE next user message (if any)
        let mut next_user_idx: Option<usize> = None;
        if let Some(a_idx) = last_assistant_idx {
            for i in (a_idx + 1)..m.messages.len() {
                if m.messages[i].role == types::MessageRole::User {
                    next_user_idx = Some(i);
                    break;
                } else if m.messages[i].role == types::MessageRole::Assistant {
                    break; // another assistant appeared; no user to pair with
                }
            }
        }

        use std::collections::HashSet;
        let allowed: HashSet<_> = last_tool_uses.iter().cloned().collect();

        let mut found: Vec<String> = Vec::new();
        if let Some(u_idx) = next_user_idx {
            if let types::MessageContent::Blocks(blocks) = &m.messages[u_idx].content {
                for block in blocks {
                    if let types::RequestContentBlock::ToolResult(tr) = block {
                        found.push(tr.tool_use_id.clone());
                    }
                }
            }
        }

        // 2a) All required results present
        for need in &last_tool_uses {
            if !found.contains(need) {
                return Err(AIError::UnpairedToolUse(need.clone()));
            }
        }
        // 2b) No extras & no foreign ids
        if found.len() != allowed.len() {
            return Err(AIError::InvalidMessageSequence(format!(
                "Expected exactly {} tool_result block(s) in the next user message, found {}",
                allowed.len(),
                found.len()
            )));
        }
        for id in &found {
            if !allowed.contains(id) {
                return Err(AIError::InvalidMessageSequence(format!(
                    "tool_result '{}' does not correspond to a tool_use in the previous assistant turn",
                    id
                )));
            }
        }

        // 3) Forbid ANY tool_result blocks in later user messages
        if let Some(u_idx) = next_user_idx {
            for msg in m.messages.iter().skip(u_idx + 1) {
                if msg.role == types::MessageRole::User {
                    if let types::MessageContent::Blocks(blocks) = &msg.content {
                        if blocks.iter().any(|b| matches!(b, types::RequestContentBlock::ToolResult(_))) {
                            return Err(AIError::InvalidMessageSequence(
                                "tool_result blocks may only appear in the immediate next user message".into(),
                            ));
                        }
                    }
                } else if msg.role == types::MessageRole::Assistant {
                    break; // once a new assistant appears, stop scanning
                }
            }
        }

        Ok(())
    }

    /// Extract Anthropic-specific headers for vendor storage
    pub fn extract_anthropic_headers(headers: &HeaderMap) -> Option<Value> {
        let mut anthropic_headers = Map::new();
        
        // Extract anthropic-version (required)
        if let Some(version_header) = headers.get("anthropic-version") {
            if let Ok(version_str) = version_header.to_str() {
                anthropic_headers.insert("version".to_string(), Value::String(version_str.to_string()));
            }
        }
        
        // Extract anthropic-beta (optional, multiple formats supported)
        let beta_features = extract_beta_features(headers);
        if !beta_features.is_empty() {
            anthropic_headers.insert("beta".to_string(), Value::Array(
                beta_features.into_iter().map(Value::String).collect()
            ));
        }
        
        if anthropic_headers.is_empty() {
            None
        } else {
            Some(json!({
                "anthropic": {
                    "headers": anthropic_headers
                }
            }))
        }
    }

    /// Extract beta features from anthropic-beta headers
    /// 
    /// Supports multiple formats:
    /// 1. Single header: "anthropic-beta: feature1,feature2" 
    /// 2. Multiple headers: "anthropic-beta: feature1" + "anthropic-beta: feature2"
    /// 3. Mixed format combinations
    fn extract_beta_features(headers: &HeaderMap) -> Vec<String> {
        let mut features = Vec::new();
        
        // Get all anthropic-beta header values
        for value in headers.get_all("anthropic-beta") {
            if let Ok(value_str) = value.to_str() {
                // Split by comma and clean up whitespace
                for feature in value_str.split(',') {
                    let trimmed = feature.trim();
                    if !trimmed.is_empty() && !features.contains(&trimmed.to_string()) {
                        features.push(trimmed.to_string());
                    }
                }
            }
        }
        
        features
    }

    /// Build vendor data structure for Universal format
    /// 
    /// Creates the vendor field structure for provider-specific metadata.
    /// Only populates when there's actual vendor data to include.
    pub fn build_vendor_data(headers: &HeaderMap, provider: &str) -> Option<HashMap<String, Value>> {
        let mut vendor_data = HashMap::new();
        
        match provider {
            "anthropic" => {
                if let Some(anthropic_data) = extract_anthropic_headers(headers) {
                    vendor_data.insert("anthropic".to_string(), anthropic_data.get("anthropic").unwrap().clone());
                }
            },
            _ => {
                // For other providers, we don't extract headers yet
                // This is extensible for future provider support
            }
        }
        
        if vendor_data.is_empty() {
            None
        } else {
            Some(vendor_data)
        }
    }

    /// Convert Messages API request to Universal format
    /// 
    /// Implements exact, lossless rules per phase 1B:
    /// - Core knobs: model, max_tokens (required; default if missing), temperature, top_p, stream
    /// - System: lift top-level system → one Universal "system" message (concatenate text blocks)
    /// - Messages: roles are only user|assistant
    /// - Tools: map Anthropic tools → Universal function tools
    /// - Headers: record anthropic-version + optional anthropic-beta for observability
    /// Convert Messages API SystemPrompt to Universal system message
    /// Returns None if system prompt is empty or contains only whitespace
    pub(crate) fn convert_system_prompt(system: &types::SystemPrompt) -> Option<universal::RequestMessage> {
        let system_text = match system {
            types::SystemPrompt::String(text) => {
                // Simple string case - use as-is if non-empty after trimming
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return None;
                }
                trimmed.to_string()
            },
            types::SystemPrompt::Blocks(blocks) => {
                // Extract text from blocks and concatenate with newlines
                let text_parts: Vec<String> = blocks
                    .iter()
                    .map(|block| block.text.trim())  // Trim each block
                    .filter(|text| !text.is_empty()) // Skip empty blocks
                    .map(|text| text.to_string())
                    .collect();
                
                if text_parts.is_empty() {
                    return None;
                }
                
                // Join blocks with single newline
                text_parts.join("\n")
            }
        };
        
        // Create Universal system message
        Some(universal::RequestMessage::System(
            universal::RequestSystemMessage {
                content: universal::RequestSystemMessageContent::Text(system_text),
                name: None, // Messages API doesn't support named system messages
            }
        ))
    }

    /// Validate system prompt for size and content limits
    pub(crate) fn validate_system_prompt(system: &types::SystemPrompt) -> Result<(), AIError> {
        match system {
            types::SystemPrompt::String(text) => {
                // Check for reasonable length limits (100KB)
                if text.len() > 100_000 {
                    return Err(AIError::RequestTooLarge);
                }
            },
            types::SystemPrompt::Blocks(blocks) => {
                // Validate block count and total size
                if blocks.len() > 100 {  // reasonable block limit
                    return Err(AIError::RequestTooLarge);
                }
                
                let total_text_size: usize = blocks
                    .iter()
                    .map(|block| block.text.len())
                    .sum();
                    
                if total_text_size > 100_000 {
                    return Err(AIError::RequestTooLarge);
                }
                
                // Validate individual blocks - only text blocks supported for system prompt
                for block in blocks {
                    if block.block_type != types::ContentBlockType::Text {
                        return Err(AIError::UnsupportedContent);
                    }
                }
            }
        }
        Ok(())
    }

    /// Convert Messages API input message to Universal format and return vendor data
    /// Returns multiple messages - primary user/assistant message plus separate tool messages for tool_results
    fn convert_input_message_with_vendor_data(msg: &types::InputMessage, beta_features: &[String], seen_tool_use_ids: &std::collections::HashSet<String>) -> Result<(Vec<universal::RequestMessage>, VendorData, HashMap<String, bool>), AIError> {
        match &msg.content {
            types::MessageContent::String(text) => {
                let universal_msg = match msg.role {
                    types::MessageRole::User => universal::RequestMessage::User(
                        universal::RequestUserMessage {
                            content: universal::RequestUserMessageContent::Text(text.clone()),
                            name: None,
                        }
                    ),
                    types::MessageRole::Assistant => universal::RequestMessage::Assistant(
                        universal::RequestAssistantMessage {
                            content: Some(universal::RequestAssistantMessageContent::Text(text.clone())),
                            name: None,
                            tool_calls: None,
                            refusal: None,
                            #[allow(deprecated)]
                            function_call: None,
                            audio: None,
                        }
                    ),
                };
                Ok((vec![universal_msg], VendorData::default(), HashMap::new()))
            },
            types::MessageContent::Blocks(blocks) => {
                // Process content blocks into unified structure
                let mut result = ContentBlockConversionResult {
                    text_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    tool_results: HashMap::new(),
                    tool_results_meta: HashMap::new(),
                    vendor_data: VendorData::default(),
                };

                // Process each content block
                for block in blocks {
                    process_content_block(block, &mut result, beta_features)?;
                }

                // Build Universal messages - preserve conversation flow by NOT splitting tool results
                let mut messages = Vec::new();
                
                // Validate tool_use_ids exist for any tool_results
                for tool_use_id in result.tool_results.keys() {
                    if !seen_tool_use_ids.contains(tool_use_id) {
                        return Err(AIError::UnpairedToolResult(tool_use_id.clone()));
                    }
                }
                
                // Create primary user/assistant message
                let primary_msg = match msg.role {
                    types::MessageRole::User => {
                        // User messages: combine text content - tool_results will be handled by vendor bag
                        let combined_text = if result.text_parts.is_empty() {
                            "[Non-text content]".to_string()
                        } else {
                            result.text_parts.join("\n\n")
                        };

                        universal::RequestMessage::User(
                            universal::RequestUserMessage {
                                content: universal::RequestUserMessageContent::Text(combined_text),
                                name: None,
                            }
                        )
                    },
                    types::MessageRole::Assistant => {
                        // Assistant messages: combine text + tool_calls
                        let content = if result.text_parts.is_empty() {
                            None
                        } else {
                            Some(universal::RequestAssistantMessageContent::Text(
                                result.text_parts.join("\n\n")
                            ))
                        };

                        let tool_calls = if result.tool_calls.is_empty() {
                            None
                        } else {
                            Some(result.tool_calls)
                        };

                        universal::RequestMessage::Assistant(
                            universal::RequestAssistantMessage {
                                content,
                                name: None,
                                tool_calls,
                                refusal: None,
                                #[allow(deprecated)]
                                function_call: None,
                                audio: None,
                            }
                        )
                    }
                };
                messages.push(primary_msg);

                // Create separate Tool messages for each tool_result
                // These will be coalesced by the Bedrock adapter to maintain role alternation
                for (tool_use_id, content) in result.tool_results {
                    let tool_msg = universal::RequestMessage::Tool(
                        universal::RequestToolMessage {
                            tool_call_id: tool_use_id,
                            content: universal::RequestToolMessageContent::Text(content),
                        }
                    );
                    messages.push(tool_msg);
                }

                Ok((messages, result.vendor_data, result.tool_results_meta))
            }
        }
    }

    /// Content block conversion result structure
    #[derive(Debug, Default)]
    struct ContentBlockConversionResult {
        /// Accumulated text content from text blocks
        text_parts: Vec<String>,
        /// Tool calls extracted from tool_use blocks
        tool_calls: Vec<universal::MessageToolCall>,
        /// Tool results mapping (tool_use_id -> content) - handled separately
        tool_results: HashMap<String, String>,
        /// Tool results metadata (tool_use_id -> is_error) for vendor bag preservation
        tool_results_meta: HashMap<String, bool>,
        /// Vendor-specific data to preserve for provider egress
        vendor_data: VendorData,
    }

    /// Vendor-specific data preservation for provider egress
    #[derive(Debug, Default)]
    pub(crate) struct VendorData {
        /// Original Anthropic content blocks for lossless reconstruction
        anthropic_content_blocks: Vec<types::RequestContentBlock>,
        /// Thinking blocks for providers that support them
        thinking_blocks: Vec<types::RequestThinkingBlock>,
        /// Search results for enhanced responses (experimental)
        search_results: Vec<types::RequestSearchResultBlock>,
    }

    /// Process individual content block and accumulate results
    /// 
    /// Handles the core content block transformations:
    /// - Text blocks: accumulate for concatenation
    /// - ToolUse blocks: convert to tool_calls array
    /// - ToolResult blocks: store for separate Tool messages (current limitation: not fully implemented)
    /// - Image/Document blocks: stash in vendor data + add placeholder text
    /// - Thinking blocks: EXPERIMENTAL - only allowed with appropriate anthropic-beta header
    /// - SearchResult blocks: EXPERIMENTAL - only allowed with appropriate anthropic-beta header
    fn process_content_block(
        block: &types::RequestContentBlock,
        result: &mut ContentBlockConversionResult,
        beta_features: &[String],
    ) -> Result<(), AIError> {
        match block {
            // Text blocks: accumulate for concatenation
            types::RequestContentBlock::Text(text_block) => {
                if !text_block.text.trim().is_empty() {
                    result.text_parts.push(text_block.text.clone());
                }
            },

            // Tool use blocks: convert to Universal tool_calls
            types::RequestContentBlock::ToolUse(tool_use) => {
                let arguments = serde_json::to_string(&tool_use.input)
                    .map_err(|e| AIError::RequestMarshal(e))?;

                result.tool_calls.push(universal::MessageToolCall {
                    id: tool_use.id.clone(),
                    r#type: universal::ToolType::Function,
                    function: universal::FunctionCall {
                        name: tool_use.name.clone(),
                        arguments,
                    },
                });
            },

            // Tool result blocks: convert to separate Tool role messages
            types::RequestContentBlock::ToolResult(tool_result) => {
                let content = match &tool_result.content {
                    Some(types::ToolResultContent::Text(text)) => text.clone(),
                    Some(types::ToolResultContent::Blocks(blocks)) => {
                        // Extract text from nested blocks (recursive call to extract_text_from_blocks)
                        extract_text_from_blocks(blocks)
                    },
                    Some(types::ToolResultContent::Unknown(value)) => {
                        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
                    },
                    None => String::new(),
                };

                // Add error indicator if present
                let final_content = if tool_result.is_error.unwrap_or(false) {
                    format!("ERROR: {}", content)
                } else {
                    content
                };

                // Check for duplicate tool_result blocks (strict rejection policy)
                if result.tool_results.contains_key(&tool_result.tool_use_id) {
                    return Err(AIError::DuplicateToolResult(tool_result.tool_use_id.clone()));
                }

                // Store for separate Tool message creation
                result.tool_results.insert(tool_result.tool_use_id.clone(), final_content);
                
                // Preserve is_error in vendor bag for provider-specific handling
                if let Some(is_error) = tool_result.is_error {
                    result.tool_results_meta.insert(tool_result.tool_use_id.clone(), is_error);
                }
                
                // No longer add placeholder text - tool_results become separate messages
            },

            // Thinking blocks: EXPERIMENTAL - gate behind beta header
            types::RequestContentBlock::Thinking(thinking) => {
                // Check if thinking blocks are enabled via anthropic-beta header
                if !beta_features.iter().any(|f| f.contains("thinking") || f.contains("experimental")) {
                    return Err(AIError::UnsupportedContent);
                }
                
                result.vendor_data.thinking_blocks.push((**thinking).clone());
                // Add thinking content to text for providers that don't support thinking
                result.text_parts.push(format!("[THINKING] {}", thinking.thinking));
            },

            // Image blocks: standard feature, always allowed
            types::RequestContentBlock::Image(_) => {
                // Store in vendor data for lossless reconstruction
                result.vendor_data.anthropic_content_blocks.push(block.clone());
                // Add descriptive fallback for text-only providers
                result.text_parts.push("[IMAGE_ATTACHMENT]".to_string());
            },

            // Document blocks: EXPERIMENTAL - gate behind beta header
            types::RequestContentBlock::Document(doc) => {
                // Check if document blocks are enabled via anthropic-beta header
                if !beta_features.iter().any(|f| f.contains("document") || f.contains("experimental")) {
                    return Err(AIError::UnsupportedContent);
                }

                // Store in vendor data for lossless reconstruction
                result.vendor_data.anthropic_content_blocks.push(block.clone());
                // Add descriptive fallback for text-only providers
                result.text_parts.push(format!("[DOCUMENT: {}]", doc.title.as_deref().unwrap_or("Untitled")));
            },

            // Search result blocks: EXPERIMENTAL - gate behind beta header
            types::RequestContentBlock::SearchResult(search) => {
                // Check if search result blocks are enabled via anthropic-beta header
                if !beta_features.iter().any(|f| f.contains("search") || f.contains("experimental")) {
                    return Err(AIError::UnsupportedContent);
                }

                result.vendor_data.search_results.push((**search).clone());
                
                // Extract and add searchable content to main text
                let search_text = format!(
                    "[SEARCH_RESULT: {}]\n{}", 
                    search.title,
                    search.content.iter()
                        .map(|block| block.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                result.text_parts.push(search_text);
            },
        }

        Ok(())
    }

    /// Convert Anthropic tools to Universal format
    fn convert_tools(tools: &[types::Tool]) -> Vec<universal::ChatCompletionTool> {
        tools.iter().map(|tool| {
            universal::ChatCompletionTool {
                r#type: universal::ToolType::Function,
                function: universal::FunctionObject {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: Some(tool.input_schema.clone()),
                    strict: None,
                },
            }
        }).collect()
    }

    /// Convert Anthropic tool choice to Universal format
    fn convert_tool_choice(choice: &types::ToolChoice) -> universal::ToolChoiceOption {
        match choice {
            types::ToolChoice::Auto { .. } => universal::ToolChoiceOption::Auto,
            types::ToolChoice::Any { .. } => universal::ToolChoiceOption::Required,
            types::ToolChoice::Tool { name, .. } => {
                universal::ToolChoiceOption::Named(universal::NamedToolChoice {
                    r#type: universal::ToolType::Function,
                    function: universal::FunctionName {
                        name: name.clone(),
                    },
                })
            },
            types::ToolChoice::None => universal::ToolChoiceOption::None,
        }
    }

    /// Build Universal messages array with system message first and collect vendor data
    pub(crate) fn build_universal_messages(
        messages_req: &types::MessagesRequest,
        beta_features: &[String],
    ) -> Result<(Vec<universal::RequestMessage>, VendorData, HashMap<String, bool>), AIError> {
        let mut universal_messages = Vec::new();
        let mut collected_vendor_data = VendorData::default();
        let mut collected_tool_results_meta = HashMap::new();
        let mut seen_tool_use_ids = std::collections::HashSet::new();
        
        // 1. Add system message first (if present)
        if let Some(system) = &messages_req.system {
            if let Some(system_msg) = convert_system_prompt(system) {
                universal_messages.push(system_msg);
            }
        }
        
        // 2. Add conversation messages and collect vendor data
        for msg in &messages_req.messages {
            // First pass: collect tool_use_ids from assistant messages to validate tool_results
            if let types::MessageRole::Assistant = msg.role {
                if let types::MessageContent::Blocks(blocks) = &msg.content {
                    for block in blocks {
                        if let types::RequestContentBlock::ToolUse(tool_use) = block {
                            seen_tool_use_ids.insert(tool_use.id.clone());
                        }
                    }
                }
            }
            
            let (msg_universal_messages, msg_vendor_data, msg_tool_results_meta) = convert_input_message_with_vendor_data(msg, beta_features, &seen_tool_use_ids)?;
            universal_messages.extend(msg_universal_messages);
            
            // Merge vendor data from this message
            collected_vendor_data.anthropic_content_blocks.extend(msg_vendor_data.anthropic_content_blocks);
            collected_vendor_data.thinking_blocks.extend(msg_vendor_data.thinking_blocks);
            collected_vendor_data.search_results.extend(msg_vendor_data.search_results);
            
            // Merge tool results metadata
            collected_tool_results_meta.extend(msg_tool_results_meta);
        }
        
        Ok((universal_messages, collected_vendor_data, collected_tool_results_meta))
    }

    pub fn to_universal(
        m: &types::MessagesRequest, 
        headers: &HeaderMap
    ) -> Result<universal::Request, AIError> {
        // FAIL-FAST VALIDATION: Validate request before any conversion
        validate_messages_request(m)?;
        
        // Validate system prompt if present
        if let Some(system) = &m.system {
            validate_system_prompt(system)?;
        }
        
        // Extract vendor-specific headers for Anthropic provider
        let mut vendor_data = build_vendor_data(headers, "anthropic");
        
        // Extract beta features for experimental content gating
        let beta_features = extract_beta_features(headers);
        
        // Build messages array and collect vendor data from content blocks
        let (messages, content_vendor_data, tool_results_meta) = build_universal_messages(m, &beta_features)?;
        
        // Merge content vendor data and Anthropic-specific fields into vendor bag
        let vendor_map = vendor_data.get_or_insert_with(HashMap::new);
        let anthropic_data = vendor_map.entry("anthropic".to_string())
            .or_insert_with(|| serde_json::json!({}));
        
        // Store Anthropic-specific request fields in vendor bag
        anthropic_data["original_max_tokens"] = serde_json::json!(m.max_tokens);
        if let Some(top_k) = m.top_k {
            anthropic_data["top_k"] = serde_json::json!(top_k);
        }
        if let Some(thinking) = &m.thinking {
            anthropic_data["thinking"] = serde_json::to_value(thinking).unwrap_or_else(|_| serde_json::json!({}));
        }
        if let Some(metadata) = &m.metadata {
            anthropic_data["metadata"] = serde_json::to_value(metadata).unwrap_or_else(|_| serde_json::json!({}));
        }
        if let Some(system) = &m.system {
            anthropic_data["system"] = serde_json::to_value(system).unwrap_or_else(|_| serde_json::json!({}));
        }
        
        // Merge content vendor data into vendor bag
        if !content_vendor_data.anthropic_content_blocks.is_empty() {
            anthropic_data["content_blocks"] = serde_json::to_value(&content_vendor_data.anthropic_content_blocks)
                .unwrap_or_else(|_| serde_json::json!([]));
        }
        
        if !content_vendor_data.thinking_blocks.is_empty() {
            anthropic_data["thinking_blocks"] = serde_json::to_value(&content_vendor_data.thinking_blocks)
                .unwrap_or_else(|_| serde_json::json!([]));
        }
        
        if !content_vendor_data.search_results.is_empty() {
            anthropic_data["search_results"] = serde_json::to_value(&content_vendor_data.search_results)
                .unwrap_or_else(|_| serde_json::json!([]));
        }
        
        // Store tool results metadata for provider-specific handling (e.g., Bedrock ToolResult.status)
        if !tool_results_meta.is_empty() {
            anthropic_data["tool_results_meta"] = serde_json::to_value(&tool_results_meta)
                .unwrap_or_else(|_| serde_json::json!({}));
        }
        
        // Ensure we have at least one message
        if messages.is_empty() {
            return Err(AIError::MessageNotFound);
        }

        // Convert stop sequences
        let stop = m.stop_sequences.as_ref().map(|stops| {
            if stops.len() == 1 {
                async_openai::types::Stop::String(stops[0].clone())
            } else {
                async_openai::types::Stop::StringArray(stops.clone())
            }
        });
        
        Ok(universal::Request {
            messages,
            model: Some(m.model.clone()),
            #[allow(deprecated)]
            max_tokens: Some(m.max_tokens),
            temperature: m.temperature,
            top_p: m.top_p,
            stream: m.stream,
            stop,
            tools: m.tools.as_deref().map(convert_tools),
            tool_choice: m.tool_choice.as_ref().map(convert_tool_choice),
            vendor: vendor_data,
            
            // Initialize other fields with defaults
            store: None,
            reasoning_effort: None,
            metadata: None,
            frequency_penalty: None,
            logit_bias: None,
            logprobs: None,
            top_logprobs: None,
            max_completion_tokens: None,
            n: None,
            modalities: None,
            prediction: None,
            audio: None,
            presence_penalty: None,
            response_format: None,
            seed: None,
            service_tier: None,
            stream_options: None,
            parallel_tool_calls: None,
            user: None,
            web_search_options: None,
            #[allow(deprecated)]
            function_call: None,
            #[allow(deprecated)]
            functions: None,
        })
    }

    /// Clean edge decoder: convert Anthropic Messages API request to Universal format
    /// 
    /// This function provides a simple, header-free interface for converting Messages API
    /// requests to the universal format. It focuses on the core conversion logic without
    /// vendor-specific header processing.
    pub fn decode_messages_to_universal(req: &types::MessagesRequest) -> Result<universal::Request, AIError> {
        // FAIL-FAST VALIDATION: Validate request before any conversion
        validate_messages_request(req)?;
        
        // Validate system prompt if present
        if let Some(system) = &req.system {
            validate_system_prompt(system)?;
        }
        
        // Build messages array using our enhanced conversion logic (with tool_result support)
        let (messages, _content_vendor_data, _tool_results_meta) = build_universal_messages(req, &[])?;
        
        // Create vendor context to mark this as Messages API route and store Anthropic-specific data
        let mut vendor_data = std::collections::HashMap::new();
        vendor_data.insert("route_type".to_string(), serde_json::json!("messages"));
        
        // Store Anthropic-specific fields in vendor bag
        let mut anthropic_data = serde_json::Map::new();
        anthropic_data.insert("original_max_tokens".to_string(), serde_json::json!(req.max_tokens));
        if let Some(top_k) = req.top_k {
            anthropic_data.insert("top_k".to_string(), serde_json::json!(top_k));
        }
        if let Some(thinking) = &req.thinking {
            anthropic_data.insert("thinking".to_string(), serde_json::to_value(thinking).unwrap());
        }
        if let Some(metadata) = &req.metadata {
            anthropic_data.insert("metadata".to_string(), serde_json::to_value(metadata).unwrap());
        }
        if let Some(system) = &req.system {
            anthropic_data.insert("system".to_string(), serde_json::to_value(system).unwrap());
        }
        vendor_data.insert("anthropic".to_string(), serde_json::Value::Object(anthropic_data));
        
        // Convert tools if present
        let universal_tools = req.tools.as_deref().map(convert_tools);
        let universal_tool_choice = req.tool_choice.as_ref().map(convert_tool_choice);
        
        // Build stop sequences in Universal format
        let stop = req.stop_sequences.as_ref().map(|stops| {
            if stops.len() == 1 {
                async_openai::types::Stop::String(stops[0].clone())
            } else {
                async_openai::types::Stop::StringArray(stops.clone())
            }
        });
        
        Ok(universal::Request {
            messages,
            model: Some(req.model.clone()),
            stream: req.stream,
            temperature: req.temperature,
            top_p: req.top_p,
            stop,
            tools: universal_tools,
            tool_choice: universal_tool_choice,
            vendor: Some(vendor_data),
            
            // Use max_tokens field (universal OpenAI field) for token limit
            #[allow(deprecated)]
            max_tokens: Some(req.max_tokens),
            max_completion_tokens: None,
            
            // Initialize remaining fields with defaults
            store: None,
            reasoning_effort: None,
            metadata: None,
            frequency_penalty: None,
            logit_bias: None,
            logprobs: None,
            top_logprobs: None,
            n: None,
            modalities: None,
            prediction: None,
            audio: None,
            presence_penalty: None,
            response_format: None,
            seed: None,
            service_tier: None,
            stream_options: None,
            parallel_tool_calls: None,
            user: None,
            web_search_options: None,
            #[allow(deprecated)]
            function_call: None,
            #[allow(deprecated)]
            functions: None,
        })
    }
}

pub mod egress {
    //! Universal -> Messages conversion
    use super::*;
    use crate::llm::{universal, AIError};

    /// Clean edge encoder: convert Universal response to Anthropic Messages API format
    /// 
    /// This function provides the response encoding counterpart to decode_messages_to_universal(),
    /// converting universal responses back to Messages API format while preserving vendor-specific
    /// metadata and handling route-specific response formatting.
    pub fn encode_universal_to_messages(u: &universal::Response) -> Result<types::MessagesResponse, AIError> {
        // Extract the assistant message from choices
        let choice = u.choices.first()
            .ok_or_else(|| AIError::MessageNotFound)?;

        let assistant_message = &choice.message;
        
        // Convert content blocks
        let mut content_blocks = Vec::new();
        
        // Add text content if present
        if let Some(content) = &assistant_message.content {
            content_blocks.push(types::ResponseContentBlock::Text(
                types::ResponseTextBlock {
                    text: content.clone(),
                    citations: None,
                }
            ));
        }
        
        // Add tool use blocks if present
        if let Some(tool_calls) = &assistant_message.tool_calls {
            for tool_call in tool_calls {
                content_blocks.push(types::ResponseContentBlock::ToolUse(
                    types::ResponseToolUseBlock {
                        id: tool_call.id.clone(),
                        name: tool_call.function.name.clone(),
                        input: serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
                    }
                ));
            }
        }
        
        // Map finish reasons from Universal to Messages API format
        let stop_reason = match choice.finish_reason.as_ref() {
            Some(universal::FinishReason::Stop) => Some(types::StopReason::EndTurn),
            Some(universal::FinishReason::Length) => Some(types::StopReason::MaxTokens),
            Some(universal::FinishReason::ToolCalls) => Some(types::StopReason::ToolUse),
            Some(universal::FinishReason::ContentFilter) => Some(types::StopReason::Refusal),
            _ => Some(types::StopReason::EndTurn),
        };

        // Check if response was stopped by a custom stop sequence
        // This info should be preserved in vendor bag from the original request
        let stop_sequence = None; // TODO: Extract from vendor bag if available

        Ok(types::MessagesResponse {
            id: u.id.clone(),
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: content_blocks,
            model: u.model.clone(),
            stop_reason,
            stop_sequence,
            usage: types::Usage {
                input_tokens: u.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                output_tokens: u.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                cache_creation: None,
                server_tool_use: None,
                service_tier: None,
            },
            container: None,
        })
    }

    /// Build Anthropic JSON response from Universal response
    /// 
    /// Legacy function - prefer encode_universal_to_messages() for new code
    pub fn from_universal_json(u: &universal::Response) -> Result<types::MessagesResponse, AIError> {
        encode_universal_to_messages(u)
    }

}

// Re-export commonly used types for convenience
pub use types::{
    MessagesRequest, MessagesResponse, StreamEvent, Usage, StopReason,
    InputMessage, MessageContent, RequestContentBlock, ResponseContentBlock,
    Tool, ToolChoice, MessagesErrorResponse, ApiError
};

pub use ingress::{to_universal, validate_messages_request, decode_messages_to_universal};
pub use egress::{from_universal_json, encode_universal_to_messages};

/// Fast token estimator for Messages API requests (optional optimization)
pub fn estimate_tokens(request: &MessagesRequest) -> u32 {
    let mut token_count = 0u32;
    
    // Rough estimation: ~4 chars per token for English text
    if let Some(system) = &request.system {
        match system {
            types::SystemPrompt::String(text) => {
                token_count += (text.len() / 4) as u32;
            },
            types::SystemPrompt::Blocks(blocks) => {
                for block in blocks {
                    token_count += (block.text.len() / 4) as u32;
                }
            }
        }
    }
    
    // Count message tokens
    for message in &request.messages {
        match &message.content {
            MessageContent::String(text) => {
                token_count += (text.len() / 4) as u32;
            },
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        RequestContentBlock::Text(text_block) => {
                            token_count += (text_block.text.len() / 4) as u32;
                        },
                        RequestContentBlock::ToolUse(_) => token_count += 10,
                        RequestContentBlock::ToolResult(_) => token_count += 5,
                        RequestContentBlock::Image(_) => token_count += 100,
                        RequestContentBlock::Document(_) => token_count += 200,
                        RequestContentBlock::Thinking(thinking) => {
                            token_count += (thinking.thinking.len() / 4) as u32;
                        },
                        RequestContentBlock::SearchResult(search) => {
                            for content in &search.content {
                                token_count += (content.text.len() / 4) as u32;
                            }
                        },
                    }
                }
            }
        }
    }
    
    // Add tool definition overhead if present
    if let Some(tools) = &request.tools {
        token_count += tools.len() as u32 * 20;
    }
    
    token_count
}

/// Extract all text content from Messages request for observability/logging
pub fn extract_all_text(request: &MessagesRequest) -> String {
    let mut text_parts = Vec::new();
    
    // Extract system text
    if let Some(system) = &request.system {
        match system {
            types::SystemPrompt::String(text) => {
                if !text.is_empty() {
                    text_parts.push(format!("System: {}", text));
                }
            },
            types::SystemPrompt::Blocks(blocks) => {
                for block in blocks {
                    if !block.text.is_empty() {
                        text_parts.push(format!("System: {}", block.text));
                    }
                }
            }
        }
    }
    
    // Extract message text
    for message in request.messages.iter() {
        let role_prefix = match message.role {
            types::MessageRole::User => "User",
            types::MessageRole::Assistant => "Assistant",
        };
        
        match &message.content {
            MessageContent::String(text) => {
                if !text.is_empty() {
                    text_parts.push(format!("{}: {}", role_prefix, text));
                }
            },
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        RequestContentBlock::Text(text_block) => {
                            if !text_block.text.is_empty() {
                                text_parts.push(format!("{}: {}", role_prefix, text_block.text));
                            }
                        },
                        RequestContentBlock::ToolUse(tool_use) => {
                            text_parts.push(format!("{}: [Tool: {}]", role_prefix, tool_use.name));
                        },
                        RequestContentBlock::ToolResult(tool_result) => {
                            text_parts.push(format!("{}: [Tool Result: {}]", role_prefix, tool_result.tool_use_id));
                        },
                        RequestContentBlock::Thinking(thinking) => {
                            if !thinking.thinking.is_empty() {
                                text_parts.push(format!("{}: [Thinking: {}...]", role_prefix, 
                                    thinking.thinking.chars().take(50).collect::<String>()));
                            }
                        },
                        RequestContentBlock::Image(_) => {
                            text_parts.push(format!("{}: [Image]", role_prefix));
                        },
                        RequestContentBlock::Document(doc) => {
                            let title = doc.title.as_deref().unwrap_or("Untitled");
                            text_parts.push(format!("{}: [Document: {}]", role_prefix, title));
                        },
                        RequestContentBlock::SearchResult(search) => {
                            text_parts.push(format!("{}: [Search: {}]", role_prefix, search.title));
                        },
                    }
                }
            }
        }
    }
    
    text_parts.join("\n")
}

/// Extract text from content blocks (helper function)
pub fn extract_text_from_blocks(blocks: &[RequestContentBlock]) -> String {
    let mut text_parts = Vec::new();
    
    for block in blocks {
        match block {
            RequestContentBlock::Text(text_block) => {
                if !text_block.text.is_empty() {
                    text_parts.push(text_block.text.clone());
                }
            },
            RequestContentBlock::Thinking(thinking) => {
                if !thinking.thinking.is_empty() {
                    text_parts.push(thinking.thinking.clone());
                }
            },
            _ => {}
        }
    }
    
    text_parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::universal;

    #[test]
    fn test_decode_messages_basic() {
        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            100,
            vec![types::InputMessage::user_text("Hello world".to_string())]
        );
        
        let tokens = estimate_tokens(&request);
        assert!(tokens > 0);
        assert!(tokens < 100);
    }

    #[test]
    fn test_extract_all_text() {
        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![
                types::InputMessage::user_text("Hello".to_string()),
                types::InputMessage::assistant_text("Hi there!".to_string())
            ]
        ).with_system(types::SystemPrompt::String("You are helpful".to_string()));
        
        let text = extract_all_text(&request);
        assert!(text.contains("System: You are helpful"));
        assert!(text.contains("User: Hello"));
        assert!(text.contains("Assistant: Hi there!"));
    }

    // ===== VALIDATION TESTS =====

    #[test]
    fn test_validate_empty_messages() {
        let request = types::MessagesRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            max_tokens: 1000,
            messages: vec![], // Empty messages array
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
        };

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::EmptyMessages)));
    }

    #[test]
    fn test_validate_zero_max_tokens() {
        let request = types::MessagesRequest {
            model: "claude-3-sonnet-20240229".to_string(),
            max_tokens: 0, // Invalid max_tokens
            messages: vec![types::InputMessage::user_text("Hello".to_string())],
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
        };

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::InvalidMaxTokens(0))));
    }

    #[test]
    fn test_validate_empty_content() {
        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![types::InputMessage::user_text("   ".to_string())] // Only whitespace
        );

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::EmptyMessageContent)));
    }

    #[test]
    fn test_validate_duplicate_tool_names() {
        let tool1 = types::Tool {
            name: "duplicate_tool".to_string(),
            description: Some("First tool".to_string()),
            input_schema: serde_json::json!({"type": "object"}),
            cache_control: None,
        };
        
        let tool2 = types::Tool {
            name: "duplicate_tool".to_string(),  // Duplicate name
            description: Some("Second tool".to_string()),
            input_schema: serde_json::json!({"type": "object"}),
            cache_control: None,
        };

        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![types::InputMessage::user_text("Hello".to_string())]
        ).with_tools(vec![tool1, tool2], None);

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::DuplicateToolName(_))));
    }

    #[test]
    fn test_validate_invalid_tool_schema() {
        let tool = types::Tool {
            name: "valid_tool".to_string(),
            description: Some("Tool with invalid schema".to_string()),
            input_schema: serde_json::json!("not_an_object"), // Should be object
            cache_control: None,
        };

        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![types::InputMessage::user_text("Hello".to_string())]
        ).with_tools(vec![tool], None);

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::InvalidToolDefinition(_))));
    }

    #[test]
    fn test_validate_unpaired_tool_use() {
        let tool_use_block = types::RequestContentBlock::ToolUse(Box::new(types::RequestToolUseBlock {
            id: "tool_123".to_string(),
            name: "test_tool".to_string(),
            input: serde_json::json!({"param": "value"}),
            cache_control: None,
        }));

        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![
                types::InputMessage::user_text("Please use the tool".to_string()),
                types::InputMessage::assistant_blocks(vec![tool_use_block]),
                // Missing tool_result block
            ]
        );

        let result = ingress::validate_messages_request(&request);
        assert!(matches!(result, Err(crate::llm::AIError::UnpairedToolUse(_))));
    }

    #[test]
    fn test_tool_result_conversion_to_universal_messages() {
        use crate::llm::universal;
        
        // Test that tool_results in user messages are converted to separate Universal tool messages
        let assistant_tool_use = types::InputMessage {
            role: types::MessageRole::Assistant,
            content: types::MessageContent::Blocks(vec![
                types::RequestContentBlock::ToolUse(Box::new(types::RequestToolUseBlock {
                    id: "tool_123".to_string(),
                    name: "calculator".to_string(),
                    input: serde_json::json!({"expression": "2+2"}),
                    cache_control: None,
                }))
            ]),
        };
        
        let user_tool_result = types::InputMessage {
            role: types::MessageRole::User,
            content: types::MessageContent::Blocks(vec![
                types::RequestContentBlock::Text(types::RequestTextBlock {
                    block_type: types::ContentBlockType::Text,
                    text: "What was the result?".to_string(),
                    citations: None,
                    cache_control: None,
                }),
                types::RequestContentBlock::ToolResult(Box::new(types::RequestToolResultBlock {
                    tool_use_id: "tool_123".to_string(),
                    content: Some(types::ToolResultContent::Text("4".to_string())),
                    is_error: None,
                    cache_control: None,
                }))
            ]),
        };
        
        let messages_req = types::MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            max_tokens: 100,
            messages: vec![assistant_tool_use, user_tool_result],
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
        };
        
        let result = ingress::build_universal_messages(&messages_req, &[]);
        assert!(result.is_ok());
        
        let (universal_messages, _vendor_data, _tool_results_meta) = result.unwrap();
        
        // Should have: assistant message, user message, and separate tool message
        assert_eq!(universal_messages.len(), 3);
        
        // First should be assistant with tool_calls
        match &universal_messages[0] {
            universal::RequestMessage::Assistant(msg) => {
                assert!(msg.tool_calls.is_some());
                let tool_calls = msg.tool_calls.as_ref().unwrap();
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "tool_123");
            },
            _ => panic!("Expected assistant message"),
        }
        
        // Second should be user message with text
        match &universal_messages[1] {
            universal::RequestMessage::User(msg) => {
                match &msg.content {
                    universal::RequestUserMessageContent::Text(text) => {
                        assert_eq!(text, "What was the result?");
                    },
                    _ => panic!("Expected text content"),
                }
            },
            _ => panic!("Expected user message"),
        }
        
        // Third should be tool message with result
        match &universal_messages[2] {
            universal::RequestMessage::Tool(msg) => {
                assert_eq!(msg.tool_call_id, "tool_123");
                match &msg.content {
                    universal::RequestToolMessageContent::Text(text) => {
                        assert_eq!(text, "4");
                    },
                    _ => panic!("Expected text content"),
                }
            },
            _ => panic!("Expected tool message"),
        }
    }

    #[test]
    fn test_decode_messages_to_universal() {
        // Test the clean edge decoder function
        let messages_req = types::MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            max_tokens: 150,
            messages: vec![
                types::InputMessage {
                    role: types::MessageRole::User,
                    content: types::MessageContent::String("Hello, Claude!".to_string()),
                }
            ],
            system: Some(types::SystemPrompt::String("You are a helpful assistant.".to_string())),
            tools: None,
            tool_choice: None,
            stream: Some(true),
            temperature: Some(0.7),
            top_p: Some(0.9),
            top_k: Some(50),
            stop_sequences: Some(vec!["END".to_string()]),
            metadata: None,
            thinking: None,
        };
        
        let result = ingress::decode_messages_to_universal(&messages_req);
        assert!(result.is_ok());
        
        let universal_req = result.unwrap();
        
        // Verify universal fields are populated correctly
        #[allow(deprecated)]
        {
            assert_eq!(universal_req.max_tokens, Some(150)); // Uses universal max_tokens field
        }
        assert_eq!(universal_req.model, Some("claude-3-5-sonnet-20241022".to_string()));
        assert_eq!(universal_req.stream, Some(true));
        assert_eq!(universal_req.temperature, Some(0.7));
        assert_eq!(universal_req.top_p, Some(0.9));
        
        // Verify stop sequences are mapped to universal stop field
        assert!(universal_req.stop.is_some());
        match &universal_req.stop {
            Some(async_openai::types::Stop::StringArray(stops)) => {
                assert_eq!(stops, &vec!["END".to_string()]);
            },
            _ => panic!("Expected string array stop sequences"),
        }
        
        // Verify vendor context is set
        assert!(universal_req.vendor.is_some());
        let vendor = universal_req.vendor.as_ref().unwrap();
        assert_eq!(vendor.get("route_type"), Some(&serde_json::json!("messages")));
        
        // Verify Anthropic-specific data is stored in vendor bag
        let anthropic_data = vendor.get("anthropic").expect("Should have anthropic vendor data");
        assert_eq!(anthropic_data["original_max_tokens"], serde_json::json!(150));
        assert_eq!(anthropic_data["top_k"], serde_json::json!(50));
        assert!(anthropic_data["system"].is_object());
        
        // Verify messages conversion (should include system message + user message)
        assert_eq!(universal_req.messages.len(), 2);
        
        // First should be system message
        match &universal_req.messages[0] {
            universal::RequestMessage::System(msg) => {
                match &msg.content {
                    universal::RequestSystemMessageContent::Text(text) => {
                        assert_eq!(text, "You are a helpful assistant.");
                    },
                    _ => panic!("Expected text content"),
                }
            },
            _ => panic!("Expected system message"),
        }
        
        // Second should be user message
        match &universal_req.messages[1] {
            universal::RequestMessage::User(msg) => {
                match &msg.content {
                    universal::RequestUserMessageContent::Text(text) => {
                        assert_eq!(text, "Hello, Claude!");
                    },
                    _ => panic!("Expected text content"),
                }
            },
            _ => panic!("Expected user message"),
        }
    }

    #[test]
    fn test_encode_universal_to_messages() {
        // Test the clean edge encoder function
        use crate::llm::universal;
        
        // Create a mock Universal response
        let universal_response = universal::Response {
            id: "msg_123".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "claude-3-5-sonnet-20241022".to_string(),
            choices: vec![
                universal::ChatChoice {
                    index: 0,
                    message: universal::ResponseMessage {
                        role: universal::Role::Assistant,
                        content: Some("Hello! How can I help you today?".to_string()),
                        tool_calls: Some(vec![
                            universal::MessageToolCall {
                                id: "tool_456".to_string(),
                                r#type: universal::ToolType::Function,
                                function: universal::FunctionCall {
                                    name: "get_weather".to_string(),
                                    arguments: r#"{"location": "San Francisco"}"#.to_string(),
                                },
                            }
                        ]),
                        #[allow(deprecated)]
                        function_call: None,
                        refusal: None,
                        audio: None,
                    },
                    finish_reason: Some(universal::FinishReason::ToolCalls),
                    logprobs: None,
                }
            ],
            usage: Some(universal::Usage {
                prompt_tokens: 25,
                completion_tokens: 15,
                total_tokens: 40,
                completion_tokens_details: None,
                prompt_tokens_details: None,
            }),
            service_tier: None,
            system_fingerprint: None,
        };
        
        let result = egress::encode_universal_to_messages(&universal_response);
        assert!(result.is_ok());
        
        let messages_response = result.unwrap();
        
        // Verify basic fields
        assert_eq!(messages_response.id, "msg_123");
        assert_eq!(messages_response.r#type, "message");
        assert_eq!(messages_response.role, "assistant");
        assert_eq!(messages_response.model, "claude-3-5-sonnet-20241022");
        assert_eq!(messages_response.stop_reason, Some(types::StopReason::ToolUse));
        
        // Verify content blocks
        assert_eq!(messages_response.content.len(), 2); // text + tool_use
        
        // First should be text content
        match &messages_response.content[0] {
            types::ResponseContentBlock::Text(text_block) => {
                assert_eq!(text_block.text, "Hello! How can I help you today?");
            },
            _ => panic!("Expected text content block"),
        }
        
        // Second should be tool use
        match &messages_response.content[1] {
            types::ResponseContentBlock::ToolUse(tool_block) => {
                assert_eq!(tool_block.id, "tool_456");
                assert_eq!(tool_block.name, "get_weather");
                assert_eq!(tool_block.input["location"], "San Francisco");
            },
            _ => panic!("Expected tool use content block"),
        }
        
        // Verify usage
        assert_eq!(messages_response.usage.input_tokens, 25);
        assert_eq!(messages_response.usage.output_tokens, 15);
    }

    #[test]
    fn test_validate_valid_requests() {
        // Test basic valid request
        let request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![
                types::InputMessage::user_text("Hello Claude".to_string()),
                types::InputMessage::assistant_text("Hello! How can I help you today?".to_string()),
            ]
        );
        assert!(ingress::validate_messages_request(&request).is_ok());

        // Test valid tool pairing
        let tool_use_block = types::RequestContentBlock::ToolUse(Box::new(types::RequestToolUseBlock {
            id: "tool_123".to_string(),
            name: "test_tool".to_string(),
            input: serde_json::json!({"param": "value"}),
            cache_control: None,
        }));

        let tool_result_block = types::RequestContentBlock::ToolResult(Box::new(types::RequestToolResultBlock {
            tool_use_id: "tool_123".to_string(),
            content: Some(types::ToolResultContent::Text("Tool executed successfully".to_string())),
            is_error: Some(false),
            cache_control: None,
        }));

        let tool_request = types::MessagesRequest::new(
            "claude-3-sonnet-20240229".to_string(),
            1000,
            vec![
                types::InputMessage::user_text("Please use the tool".to_string()),
                types::InputMessage::assistant_blocks(vec![tool_use_block]),
                types::InputMessage::user_blocks(vec![tool_result_block]),
            ]
        );
        assert!(ingress::validate_messages_request(&tool_request).is_ok());
    }

    /// Tests for header extraction and vendor data processing
    mod header_tests {
        use super::*;
        use http::{HeaderMap, HeaderValue};
        use serde_json::Value;

        #[test]
        fn test_extract_anthropic_version_header() {
            let mut headers = HeaderMap::new();
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            
            let vendor_data = ingress::extract_anthropic_headers(&headers);
            assert!(vendor_data.is_some());
            
            let data = vendor_data.unwrap();
            let headers_obj = &data["anthropic"]["headers"];
            assert_eq!(headers_obj["version"], "2023-06-01");
        }

        #[test]
        fn test_extract_anthropic_beta_comma_separated() {
            let mut headers = HeaderMap::new();
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            headers.insert("anthropic-beta", HeaderValue::from_static("feature1,feature2,feature3"));
            
            let vendor_data = ingress::extract_anthropic_headers(&headers).unwrap();
            let headers_obj = &vendor_data["anthropic"]["headers"];
            
            let beta_array = headers_obj["beta"].as_array().unwrap();
            assert_eq!(beta_array.len(), 3);
            assert!(beta_array.contains(&Value::String("feature1".to_string())));
            assert!(beta_array.contains(&Value::String("feature2".to_string())));
            assert!(beta_array.contains(&Value::String("feature3".to_string())));
        }

        #[test]
        fn test_no_anthropic_headers() {
            let headers = HeaderMap::new();
            let vendor_data = ingress::extract_anthropic_headers(&headers);
            assert!(vendor_data.is_none());
        }

        #[test]
        fn test_to_universal_with_headers() {
            let request = types::MessagesRequest::new(
                "claude-sonnet-4-20250514".to_string(),
                1000,
                vec![types::InputMessage::user_text("Hello".to_string())]
            );
            
            let mut headers = HeaderMap::new();
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            headers.insert("anthropic-beta", HeaderValue::from_static("files-api-2025-04-14"));
            
            let universal_request = ingress::to_universal(&request, &headers).unwrap();
            
            assert!(universal_request.vendor.is_some());
            let vendor = universal_request.vendor.unwrap();
            assert!(vendor.contains_key("anthropic"));
            
            let anthropic_data = &vendor["anthropic"];
            let headers_obj = &anthropic_data["headers"];
            assert_eq!(headers_obj["version"], "2023-06-01");
        }
    }

    /// Tests for system prompt conversion logic
    mod system_prompt_tests {
        use super::*;
        use crate::llm::{universal, AIError};
        use http::HeaderMap;

        #[test]
        fn test_convert_system_prompt_simple_string() {
            let system = types::SystemPrompt::String("You are a helpful assistant.".to_string());
            let result = ingress::convert_system_prompt(&system).unwrap();
            
            if let universal::RequestMessage::System(sys_msg) = result {
                if let universal::RequestSystemMessageContent::Text(text) = sys_msg.content {
                    assert_eq!(text, "You are a helpful assistant.");
                }
                assert!(sys_msg.name.is_none());
            } else {
                panic!("Expected system message");
            }
        }

        #[test]
        fn test_convert_system_prompt_multiple_content_blocks() {
            let blocks = vec![
                types::RequestTextBlock {
                    block_type: types::ContentBlockType::Text,
                    text: "You are a helpful assistant.".to_string(),
                    citations: None,
                    cache_control: None,
                },
                types::RequestTextBlock {
                    block_type: types::ContentBlockType::Text,
                    text: "Always be concise and accurate.".to_string(),
                    citations: None,
                    cache_control: None,
                }
            ];
            let system = types::SystemPrompt::Blocks(blocks);
            let result = ingress::convert_system_prompt(&system).unwrap();
            
            if let universal::RequestMessage::System(sys_msg) = result {
                if let universal::RequestSystemMessageContent::Text(text) = sys_msg.content {
                    assert_eq!(text, "You are a helpful assistant.\nAlways be concise and accurate.");
                }
            }
        }

        #[test]
        fn test_validate_system_prompt_string_too_large() {
            let text = "A".repeat(150_000); // 150KB - exceeds limit
            let system = types::SystemPrompt::String(text);
            let result = ingress::validate_system_prompt(&system);
            assert!(matches!(result, Err(AIError::RequestTooLarge)));
        }

        #[test]
        fn test_to_universal_with_system_string() {
            let request = types::MessagesRequest::new(
                "claude-sonnet-4-20250514".to_string(),
                1000,
                vec![types::InputMessage::user_text("Hello".to_string())]
            ).with_system(types::SystemPrompt::String("You are helpful".to_string()));
            
            let headers = HeaderMap::new();
            let universal_request = ingress::to_universal(&request, &headers).unwrap();
            
            assert_eq!(universal_request.messages.len(), 2);
            
            // First message should be system
            if let universal::RequestMessage::System(sys_msg) = &universal_request.messages[0] {
                if let universal::RequestSystemMessageContent::Text(text) = &sys_msg.content {
                    assert_eq!(text, "You are helpful");
                }
            } else {
                panic!("Expected first message to be system");
            }
            
            // Second message should be user
            if let universal::RequestMessage::User(_) = &universal_request.messages[1] {
                // Good
            } else {
                panic!("Expected second message to be user");
            }
        }
    }
}
