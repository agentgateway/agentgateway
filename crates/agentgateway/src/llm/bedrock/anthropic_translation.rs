//! Anthropic Messages API and Bedrock Converse API translation

use crate::llm::AIError;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use std::collections::HashMap;
use tokio::time::{Duration, sleep};
use tracing::{self, debug};

use crate::llm::anthropic_types::{
	self as anthropic, MessagesRequest, MessagesResponse, RequestContentBlock, ResponseContentBlock,
};
use crate::llm::bedrock::cache::{
	CachePlannerConfig, estimate_tool_schema_weight, plan_and_insert_cachepoint,
};
use crate::llm::bedrock::types::{
	self as bedrock, ContentBlock, ConverseRequest, ConverseResponse,
};
use http::HeaderMap;

/// Configuration for request translation
#[derive(Debug, Clone)]
pub struct TranslationConfig {
	/// Default region for Bedrock endpoints
	pub aws_region: String,

	/// Whether to enable guardrails
	pub enable_guardrails: bool,

	/// Guardrail configuration (if enabled)
	pub guardrail_identifier: Option<String>,

	/// Enable strategic prompt caching
	pub enable_prompt_caching: bool,

	/// Minimum tokens required before inserting cache point
	pub prompt_cache_min_tokens: Option<usize>,

	/// Safety margin for cache point insertion
	pub prompt_cache_safety_margin: Option<usize>,

	/// Force cache point insertion (for experimentation)
	pub prompt_cache_force: Option<bool>,

	/// Whether to include tool schema weight in cache planning
	pub prompt_cache_include_tool_weight: bool,
	pub guardrail_version: Option<String>,

	/// Additional model request fields
	pub additional_model_fields: Option<serde_json::Value>,
}

/// Extracted Anthropic-specific headers
#[derive(Debug, Clone)]
pub struct AnthropicHeaders {
	/// anthropic-version header (required)
	pub anthropic_version: Option<String>,

	/// anthropic-beta headers (comma-separated features)
	pub anthropic_beta: Option<Vec<String>>,

	/// Conversation ID for tracking (extracted from custom headers or generated)
	pub conversation_id: Option<String>,
}

/// Extract Anthropic-specific headers from HTTP request
pub fn extract_anthropic_headers(headers: &HeaderMap) -> Result<AnthropicHeaders, AIError> {
	// Extract anthropic-version (required)
	let anthropic_version = headers
		.get("anthropic-version")
		.and_then(|v| v.to_str().ok())
		.map(|s| s.to_string());

	// Extract anthropic-beta headers (can be multiple)
	let mut beta_features = Vec::new();
	for value in headers.get_all("anthropic-beta") {
		if let Ok(beta_str) = value.to_str() {
			// Split comma-separated beta features
			for feature in beta_str.split(',') {
				let trimmed = feature.trim();
				if !trimmed.is_empty() {
					beta_features.push(trimmed.to_string());
				}
			}
		}
	}

	let anthropic_beta = if beta_features.is_empty() {
		None
	} else {
		Some(beta_features)
	};

	// Extract conversation ID from custom header if present
	let conversation_id = headers
		.get("x-conversation-id")
		.and_then(|v| v.to_str().ok())
		.map(|s| s.to_string());

	Ok(AnthropicHeaders {
		anthropic_version,
		anthropic_beta,
		conversation_id,
	})
}

/// Validate Anthropic MessagesRequest for correctness
pub fn validate_anthropic_request(request: &MessagesRequest) -> Result<(), AIError> {
	// Validate max_tokens is reasonable
	if request.max_tokens == 0 {
		return Err(AIError::MissingField(
			"max_tokens must be greater than 0".into(),
		));
	}

	// Validate messages are not empty
	if request.messages.is_empty() {
		return Err(AIError::MissingField("messages cannot be empty".into()));
	}

	// Thinking validation handled by thinking_validator module

	// Validate temperature is in valid range
	if let Some(temp) = request.temperature {
		if temp < 0.0 || temp > 1.0 {
			return Err(AIError::MissingField(
				"temperature must be between 0.0 and 1.0".into(),
			));
		}
	}

	// Validate top_p is in valid range
	if let Some(top_p) = request.top_p {
		if top_p < 0.0 || top_p > 1.0 {
			return Err(AIError::MissingField(
				"top_p must be between 0.0 and 1.0".into(),
			));
		}
	}

	// Validate top_k is in valid range
	if let Some(top_k) = request.top_k {
		if top_k == 0 {
			return Err(AIError::MissingField("top_k must be greater than 0".into()));
		}
	}

	// Validate stop_sequences are not empty
	if let Some(stop_sequences) = &request.stop_sequences {
		for (i, seq) in stop_sequences.iter().enumerate() {
			if seq.is_empty() {
				return Err(AIError::MissingField(
					format!("Stop sequence {} cannot be empty", i).into(),
				));
			}
		}
	}

	// Validate tools if present
	if let Some(tools) = &request.tools {
		for tool in tools.iter() {
			validate_tool_name(&tool.name)?;
		}
	}

	Ok(())
}

/// Main translation function: Anthropic MessagesRequest → Bedrock ConverseRequest
#[tracing::instrument(skip_all, fields(model = %anthropic_request.model, max_tokens = anthropic_request.max_tokens))]
pub async fn translate_request(
	anthropic_request: MessagesRequest,
	config: &TranslationConfig,
) -> Result<ConverseRequest, AIError> {
	// CRITICAL: Resolve Anthropic model name to Bedrock model ID FIRST
	let resolved_model_id = super::models::resolve_model_global(&anthropic_request.model)?;

	debug!(
		original_model = %anthropic_request.model,
		resolved_model = %resolved_model_id,
		"Model resolved in translation"
	);

	let mut bedrock_request = ConverseRequest::new(resolved_model_id.clone());

	// Translate messages
	if !anthropic_request.messages.is_empty() {
		let messages = translate_messages(anthropic_request.messages).await?;
		bedrock_request = bedrock_request.with_messages(messages);
	}

	// Translate system prompt with strategic caching
	if let Some(system) = anthropic_request.system {
		let mut system_blocks = translate_system_prompt(system)?;

		// Apply strategic prompt caching if enabled
		if config.enable_prompt_caching {
			let tool_weight = if config.prompt_cache_include_tool_weight {
				anthropic_request
					.tools
					.as_ref()
					.map(|tools| estimate_tool_schema_weight(tools))
					.unwrap_or(0)
			} else {
				0
			};

			let cache_config = CachePlannerConfig {
				min_tokens: config.prompt_cache_min_tokens.unwrap_or(1024),
				safety_margin: config.prompt_cache_safety_margin.unwrap_or(76),
				enabled: config.enable_prompt_caching,
				force: config.prompt_cache_force.unwrap_or(false),
			};

			let cache_plan = plan_and_insert_cachepoint(&mut system_blocks, tool_weight, &cache_config);

			if cache_plan.inserted {
				tracing::debug!(
					"Cache point inserted: estimated_tokens={}, position={:?}, reason={}",
					cache_plan.estimated_tokens,
					cache_plan.insertion_position,
					cache_plan.reason
				);
			} else {
				tracing::debug!("No cache point inserted: {}", cache_plan.reason);
			}
		}

		bedrock_request = bedrock_request.with_system(system_blocks);
	}

	// Translate inference configuration
	let inference_config = translate_inference_config(
		anthropic_request.max_tokens as i32,
		anthropic_request.temperature,
		anthropic_request.top_p,
		anthropic_request.top_k,
		anthropic_request.stop_sequences,
	)?;
	bedrock_request = bedrock_request.with_inference_config(inference_config);

	// Translate tools and tool choice
	if let Some(tools) = anthropic_request.tools {
		let (bedrock_tools, tool_choice) =
			translate_tools_and_choice(tools, anthropic_request.tool_choice)?;
		bedrock_request = bedrock_request.with_tools(bedrock_tools, tool_choice);
	}

	// Add guardrail configuration if enabled
	if config.enable_guardrails {
		if let (Some(identifier), Some(version)) =
			(&config.guardrail_identifier, &config.guardrail_version)
		{
			bedrock_request.guardrail_config = Some(bedrock::GuardrailConfiguration {
				guardrail_identifier: identifier.clone(),
				guardrail_version: version.clone(),
				trace: Some("enabled".to_string()),
			});
		}
	}

	// Add metadata if present
	if let Some(metadata) = anthropic_request.metadata {
		let mut request_metadata = HashMap::new();

		// Add user_id if present
		if let Some(user_id) = metadata.user_id {
			request_metadata.insert("user_id".to_string(), user_id);
		}

		// Add other custom fields
		for (key, value) in metadata.additional {
			if let Ok(string_value) = serde_json::to_string(&value) {
				request_metadata.insert(key, string_value);
			}
		}

		if !request_metadata.is_empty() {
			bedrock_request.request_metadata = Some(request_metadata);
		}
	}

	// Add additional model fields including thinking configuration
	let mut additional_fields = serde_json::Map::new();

	// Map thinking field to Bedrock format
	if let Some(thinking) = &anthropic_request.thinking {
		let mut thinking_json = serde_json::json!({ "type": thinking.thinking_type });
		if let Some(budget) = thinking.budget_tokens {
			thinking_json["budget_tokens"] = serde_json::json!(budget);
		}
		debug!(
			"Added thinking configuration to additional model fields: {:?}",
			thinking_json
		);
		additional_fields.insert("thinking".to_string(), thinking_json);
	}

	// Add any configured additional model fields
	if let Some(config_fields) = &config.additional_model_fields {
		if let Some(config_obj) = config_fields.as_object() {
			for (key, value) in config_obj {
				additional_fields.insert(key.clone(), value.clone());
			}
		}
	}

	// Only set this field if there are actual additional fields AND the model supports it
	// Older Bedrock models don't support this field and will reject requests with "Unexpected field type"
	// Use resolved model ID for feature support check
	if !additional_fields.is_empty() && supports_additional_model_fields(&resolved_model_id) {
		bedrock_request.additional_model_request_fields =
			Some(serde_json::Value::Object(additional_fields));
	}

	Ok(bedrock_request)
}

/// Check if a Bedrock model supports additional_model_request_fields
fn supports_additional_model_fields(model_id: &str) -> bool {
	// Only newer Claude models support additional_model_request_fields
	// Older models will reject requests with "Unexpected field type" error
	model_id.contains("claude-3-5") || 
	model_id.contains("claude-sonnet-4") ||
	model_id.contains("claude-opus-4") ||
	model_id.contains("claude-haiku-4") ||
	// Add future model versions as they become available
	model_id.contains("claude-") && (
		model_id.contains("-2024") ||
		model_id.contains("-2025") ||
		model_id.contains("-v2:0")
	)
}

async fn translate_messages(
	anthropic_messages: Vec<anthropic::InputMessage>,
) -> Result<Vec<bedrock::Message>, AIError> {
	let mut bedrock_messages = Vec::new();

	for message in anthropic_messages {
		let bedrock_message = translate_message(message).await?;
		bedrock_messages.push(bedrock_message);
	}

	Ok(bedrock_messages)
}

async fn translate_message(
	anthropic_message: anthropic::InputMessage,
) -> Result<bedrock::Message, AIError> {
	let role = match anthropic_message.role {
		anthropic::MessageRole::User => bedrock::ConversationRole::User,
		anthropic::MessageRole::Assistant => bedrock::ConversationRole::Assistant,
	};

	let content = translate_content_blocks(anthropic_message.content.to_blocks()).await?;

	Ok(bedrock::Message { role, content })
}

/// CRITICAL: Bedrock ContentBlock is a strict union - exactly one variant must be set
async fn translate_content_blocks(
	anthropic_blocks: Vec<RequestContentBlock>,
) -> Result<Vec<ContentBlock>, AIError> {
	let mut bedrock_blocks = Vec::new();

	for block in anthropic_blocks {
		let bedrock_block = translate_content_block(block).await?;
		bedrock_blocks.push(bedrock_block);
	}

	Ok(bedrock_blocks)
}

/// Translate single Anthropic content block to Bedrock content block with graceful fallback
#[tracing::instrument(skip_all, fields(block_type = ?std::mem::discriminant(&anthropic_block)))]
async fn translate_content_block(
	anthropic_block: RequestContentBlock,
) -> Result<ContentBlock, AIError> {
	tracing::debug!(
		"Translating content block: {:?}",
		std::mem::discriminant(&anthropic_block)
	);
	match translate_content_block_inner(anthropic_block.clone()).await {
		Ok(block) => {
			tracing::debug!(
				"Successfully translated content block to: {:?}",
				std::mem::discriminant(&block)
			);
			// Let's also log what we're actually serializing
			if let Ok(json) = serde_json::to_string(&block) {
				tracing::debug!("ContentBlock serializes to: {}", json);
			}
			Ok(block)
		},
		Err(e) => {
			// Log the error but provide graceful fallback for complex content types
			tracing::error!(
				"Content block translation failed: {}. Attempting graceful fallback.",
				e
			);

			match anthropic_block {
				RequestContentBlock::Image(_) => {
					// Fallback: Convert failed image to descriptive text
					let fallback_text = "[Image content - failed to fetch or process]".to_string();
					tracing::warn!("Image content block failed, using text fallback");
					Ok(ContentBlock::Text(fallback_text))
				},
				RequestContentBlock::Document(_) => {
					// Fallback: Convert failed document to descriptive text
					let fallback_text = "[Document content - failed to fetch or process]".to_string();
					tracing::warn!("Document content block failed, using text fallback");
					Ok(ContentBlock::Text(fallback_text))
				},
				RequestContentBlock::SearchResult(search_result) => {
					// Fallback: Try text flattening as last resort
					match flatten_search_result_to_text(&search_result) {
						Ok(flattened) => {
							tracing::warn!("Search result content block failed, using flattened text fallback");
							Ok(ContentBlock::Text(flattened))
						},
						Err(_) => {
							let fallback_text = format!(
								"[Search result from {} - failed to process]",
								search_result.source
							);
							tracing::warn!(
								"Search result content block and flattening failed, using minimal text fallback"
							);
							Ok(ContentBlock::Text(fallback_text))
						},
					}
				},
				_ => {
					// For other content types, propagate the error as they should not fail
					Err(e)
				},
			}
		},
	}
}

/// Inner content block translation (can fail)
async fn translate_content_block_inner(
	anthropic_block: RequestContentBlock,
) -> Result<ContentBlock, AIError> {
	let result = match anthropic_block {
		RequestContentBlock::Text(text_block) => {
			// Simple text content - most common case
			Ok(ContentBlock::Text(text_block.text))
		},

		RequestContentBlock::Image(image_block) => {
			let bedrock_image = translate_image_block(image_block).await?;
			Ok(ContentBlock::Image(bedrock_image))
		},

		RequestContentBlock::Document(document_block) => {
			let bedrock_document = translate_document_block(document_block).await?;
			Ok(ContentBlock::Document(bedrock_document))
		},

		RequestContentBlock::ToolUse(tool_use_block) => {
			let bedrock_tool_use = bedrock::ToolUseBlock {
				tool_use_id: tool_use_block.id,
				name: tool_use_block.name,
				input: tool_use_block.input,
			};
			Ok(ContentBlock::ToolUse(bedrock_tool_use))
		},

		RequestContentBlock::ToolResult(tool_result_block) => {
			let bedrock_tool_result = translate_tool_result_block(tool_result_block)?;
			Ok(ContentBlock::ToolResult(bedrock_tool_result))
		},

		RequestContentBlock::Thinking(thinking_block) => {
			// Map Anthropic thinking blocks to Bedrock reasoning content
			if thinking_block.signature.is_empty() {
				return Err(AIError::MissingField(
					"thinking.signature cannot be empty".into(),
				));
			}

			let reasoning_text = bedrock::ReasoningTextBlock {
				text: thinking_block.thinking.clone(),
				signature: Some(thinking_block.signature.clone()),
			};

			Ok(ContentBlock::ReasoningContent(
				bedrock::ReasoningContentBlock {
					reasoning_text: Some(reasoning_text),
					redacted_content: None,
				},
			))
		},

		RequestContentBlock::SearchResult(search_result_block) => {
			// Convert search result to formatted text content for Bedrock compatibility
			let flattened_text = flatten_search_result_to_text(&search_result_block)?;

			Ok(ContentBlock::Text(flattened_text))
		},
	};

	result
}

/// Translate Anthropic image block to Bedrock image block
async fn translate_image_block(
	anthropic_image: anthropic::RequestImageBlock,
) -> Result<bedrock::ImageBlock, AIError> {
	let (format, source) = match anthropic_image.source {
		anthropic::ImageSource::Base64 { media_type, data } => {
			let format = match media_type.as_str() {
				"image/jpeg" => bedrock::ImageFormat::Jpeg,
				"image/png" => bedrock::ImageFormat::Png,
				"image/gif" => bedrock::ImageFormat::Gif,
				"image/webp" => bedrock::ImageFormat::Webp,
				_ => return Err(AIError::UnsupportedContent),
			};
			let source = bedrock::ImageSource::Bytes { data };
			(format, source)
		},

		anthropic::ImageSource::Url { url } => {
			// Fetch image from URL and convert to base64 for Bedrock compatibility
			let (format, data) = fetch_and_encode_image(&url).await?;

			let source = bedrock::ImageSource::Bytes { data };
			(format, source)
		},

		anthropic::ImageSource::File { file_id: _ } => {
			// File ID sources would need to be resolved through Files API
			return Err(AIError::UnsupportedContent);
		},
	};

	Ok(bedrock::ImageBlock { format, source })
}

/// Translate Anthropic document block to Bedrock document block
async fn translate_document_block(
	anthropic_doc: anthropic::RequestDocumentBlock,
) -> Result<bedrock::DocumentBlock, AIError> {
	let (format, name, source) = match anthropic_doc.source {
		anthropic::DocumentSource::Base64Pdf {
			media_type: _,
			data,
		} => {
			let format = bedrock::DocumentFormat::Pdf;
			let name = anthropic_doc
				.title
				.unwrap_or_else(|| "document.pdf".to_string());
			let source = bedrock::DocumentSource::Bytes { data };
			(format, name, source)
		},

		anthropic::DocumentSource::PlainText {
			media_type: _,
			data,
		} => {
			let format = bedrock::DocumentFormat::Txt;
			let name = anthropic_doc
				.title
				.unwrap_or_else(|| "document.txt".to_string());
			let source = bedrock::DocumentSource::Bytes { data };
			(format, name, source)
		},

		anthropic::DocumentSource::ContentBlock { content_blocks } => {
			// Flatten content blocks to text format for Bedrock compatibility
			let flattened_text = flatten_content_blocks_to_text(&content_blocks)?;

			let format = bedrock::DocumentFormat::Txt;
			let name = anthropic_doc
				.title
				.unwrap_or_else(|| "document.txt".to_string());
			let source = bedrock::DocumentSource::Bytes {
				data: BASE64.encode(flattened_text.as_bytes()),
			};
			(format, name, source)
		},

		anthropic::DocumentSource::UrlPdf { url } => {
			// Fetch PDF from URL and convert to base64 for Bedrock compatibility
			let data = fetch_and_encode_pdf(&url).await?;

			let format = bedrock::DocumentFormat::Pdf;
			let name = anthropic_doc.title.unwrap_or_else(|| {
				// Extract filename from URL or use default
				url
					.split('/')
					.last()
					.and_then(|s| {
						if s.ends_with(".pdf") {
							Some(s.to_string())
						} else {
							None
						}
					})
					.unwrap_or_else(|| "document.pdf".to_string())
			});
			let source = bedrock::DocumentSource::Bytes { data };
			(format, name, source)
		},

		anthropic::DocumentSource::File { file_id: _ } => {
			return Err(AIError::UnsupportedContent);
		},
	};

	// Convert format enum to string
	let format_str = match format {
		bedrock::DocumentFormat::Pdf => "pdf",
		bedrock::DocumentFormat::Csv => "csv",
		bedrock::DocumentFormat::Doc => "doc",
		bedrock::DocumentFormat::Docx => "docx",
		bedrock::DocumentFormat::Xls => "xls",
		bedrock::DocumentFormat::Xlsx => "xlsx",
		bedrock::DocumentFormat::Html => "html",
		bedrock::DocumentFormat::Txt => "txt",
		bedrock::DocumentFormat::Md => "md",
	};

	Ok(bedrock::DocumentBlock {
		name,
		source,
		format: Some(format_str.to_string()),
		citations: None, // TODO: Map from Anthropic citations config
		context: None,   // TODO: Map from Anthropic context
	})
}

/// Translate Anthropic request content blocks to Bedrock ContentBlocks for tool results
fn translate_request_content_blocks_to_content_blocks(
	blocks: &[RequestContentBlock],
) -> Result<Vec<bedrock::ContentBlock>, AIError> {
	blocks
		.iter()
		.map(|block| match block {
			RequestContentBlock::Text(text_block) => {
				Ok(bedrock::ContentBlock::Text(text_block.text.clone()))
			},
			RequestContentBlock::Image(image_block) => {
				// Convert to proper Image ContentBlock
				match &image_block.source {
					anthropic::ImageSource::Base64 { media_type, data } => {
						let bedrock_image = bedrock::ImageBlock {
							format: match media_type.as_str() {
								"image/jpeg" => bedrock::ImageFormat::Jpeg,
								"image/png" => bedrock::ImageFormat::Png,
								"image/gif" => bedrock::ImageFormat::Gif,
								"image/webp" => bedrock::ImageFormat::Webp,
								_ => bedrock::ImageFormat::Png, // Default fallback
							},
							source: bedrock::ImageSource::Bytes { data: data.clone() },
						};
						Ok(bedrock::ContentBlock::Image(bedrock_image))
					},
					_ => {
						// For URL or File sources, convert to text representation for tool results
						Ok(bedrock::ContentBlock::Text("[Image content]".to_string()))
					},
				}
			},
			RequestContentBlock::Document(doc_block) => {
				// Convert document to text content for tool results
				// Tool results typically need text representation
				Ok(bedrock::ContentBlock::Text(format!(
					"[Document: {}]",
					doc_block.title.as_deref().unwrap_or("Untitled")
				)))
			},
			RequestContentBlock::ToolUse(tool_use_block) => {
				// Convert tool use to ToolUse ContentBlock
				Ok(bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
					tool_use_id: tool_use_block.id.clone(),
					name: tool_use_block.name.clone(),
					input: tool_use_block.input.clone(),
				}))
			},
			RequestContentBlock::ToolResult(tool_result_block) => {
				// Convert nested tool result to ToolResult ContentBlock
				let translated = translate_tool_result_block(tool_result_block.clone())?;
				Ok(bedrock::ContentBlock::ToolResult(translated))
			},
			RequestContentBlock::Thinking(thinking_block) => {
				// Convert thinking to text for tool results
				Ok(bedrock::ContentBlock::Text(format!(
					"[Thinking: {}]",
					thinking_block
						.thinking
						.chars()
						.take(100)
						.collect::<String>()
				)))
			},
			RequestContentBlock::SearchResult(search_result_block) => {
				// Convert search result to text for tool results
				Ok(bedrock::ContentBlock::Text(format!(
					"[Search Result: {}]",
					search_result_block.title
				)))
			},
		})
		.collect()
}

/// Translate arbitrary content (Unknown variant) to Bedrock ContentBlocks
fn translate_arbitrary_content_to_content_blocks(
	value: serde_json::Value,
) -> Result<Vec<bedrock::ContentBlock>, AIError> {
	match value {
		// Simple string content
		serde_json::Value::String(text) => Ok(vec![bedrock::ContentBlock::Text(text)]),
		// Array of content blocks
		serde_json::Value::Array(arr) => {
			arr
				.into_iter()
				.map(|item| {
					// First try to parse as known content block types
					if let Ok(block) = serde_json::from_value::<RequestContentBlock>(item.clone()) {
						// Recursively translate known blocks
						match translate_request_content_blocks_to_content_blocks(&[block]) {
							Ok(mut blocks) => {
								if blocks.is_empty() {
									Ok(bedrock::ContentBlock::Text("[Empty content]".to_string()))
								} else {
									Ok(blocks.remove(0))
								}
							},
							Err(_) => {
								// Fallback to text representation
								if let serde_json::Value::String(text) = item {
									Ok(bedrock::ContentBlock::Text(text))
								} else {
									Ok(bedrock::ContentBlock::Text("[Complex content]".to_string()))
								}
							},
						}
					} else {
						// If it's a plain string, convert to Text block
						if let serde_json::Value::String(text) = item {
							Ok(bedrock::ContentBlock::Text(text))
						} else {
							// Complex structure - convert to text representation
							Ok(bedrock::ContentBlock::Text("[Complex content]".to_string()))
						}
					}
				})
				.collect()
		},
		// Any other JSON structure - convert to text representation
		_ => {
			if let Some(text) = value.as_str() {
				Ok(vec![bedrock::ContentBlock::Text(text.to_string())])
			} else {
				Ok(vec![bedrock::ContentBlock::Text(
					"[Complex content]".to_string(),
				)])
			}
		},
	}
}

/// Translate Anthropic tool result block to Bedrock tool result block
fn translate_tool_result_block(
	anthropic_result: anthropic::RequestToolResultBlock,
) -> Result<bedrock::ToolResultBlock, AIError> {
	let content = match anthropic_result.content {
		Some(anthropic::ToolResultContent::Text(text)) => {
			vec![bedrock::ContentBlock::Text(text)]
		},
		Some(anthropic::ToolResultContent::Blocks(blocks)) => {
			translate_request_content_blocks_to_content_blocks(&blocks)?
		},
		Some(anthropic::ToolResultContent::Unknown(value)) => {
			translate_arbitrary_content_to_content_blocks(value)?
		},
		None => vec![], // Empty content
	};

	let status = match anthropic_result.is_error {
		Some(true) => Some(bedrock::ToolResultStatus::Error),
		Some(false) => Some(bedrock::ToolResultStatus::Success),
		None => None, // Let Bedrock infer
	};

	Ok(bedrock::ToolResultBlock {
		tool_use_id: anthropic_result.tool_use_id,
		content,
		status,
	})
}

/// Translate Anthropic system prompt to Bedrock system content blocks
fn translate_system_prompt(
	anthropic_system: anthropic::SystemPrompt,
) -> Result<Vec<bedrock::SystemContentBlock>, AIError> {
	match anthropic_system {
		anthropic::SystemPrompt::String(text) => Ok(vec![bedrock::SystemContentBlock::Text(text)]),

		anthropic::SystemPrompt::Blocks(blocks) => {
			// Convert text blocks to system content blocks
			let system_blocks: Result<Vec<_>, AIError> = blocks
				.into_iter()
				.map(|block| {
					// Cache control is handled by the cache planning logic, not here
					// Just convert the text content
					Ok(bedrock::SystemContentBlock::Text(block.text))
				})
				.collect();

			system_blocks
		},
	}
}

fn translate_inference_config(
	max_tokens: i32,
	temperature: Option<f32>,
	top_p: Option<f32>,
	_top_k: Option<u32>, // Bedrock doesn't have top_k in inference config
	stop_sequences: Option<Vec<String>>,
) -> Result<bedrock::InferenceConfiguration, AIError> {
	Ok(bedrock::InferenceConfiguration {
		max_tokens: Some(max_tokens),
		temperature,
		top_p,
		stop_sequences,
	})
}

fn translate_tools_and_choice(
	anthropic_tools: Vec<anthropic::Tool>,
	anthropic_tool_choice: Option<anthropic::ToolChoice>,
) -> Result<(Vec<bedrock::Tool>, Option<bedrock::ToolChoice>), AIError> {
	// Handle ToolChoice::None - don't include any tools in the request
	if let Some(anthropic::ToolChoice::None) = anthropic_tool_choice {
		return Ok((Vec::new(), None));
	}

	// Translate tools
	let bedrock_tools: Result<Vec<_>, AIError> =
		anthropic_tools.into_iter().map(translate_tool).collect();
	let bedrock_tools = bedrock_tools?;

	// Translate tool choice
	let bedrock_tool_choice = anthropic_tool_choice.map(translate_tool_choice);

	Ok((bedrock_tools, bedrock_tool_choice))
}

/// Validate tool name according to Bedrock API constraints (stricter than Anthropic)
fn validate_tool_name(name: &str) -> Result<(), AIError> {
	// Check name is not empty and within Bedrock limits
	if name.is_empty() {
		return Err(AIError::MissingField("Tool name cannot be empty".into()));
	}

	// Bedrock has stricter limit than Anthropic (64 vs 128 chars)
	// Use Bedrock limit to prevent runtime failures
	if name.len() > 64 {
		return Err(AIError::MissingField(
			format!(
				"Tool name exceeds Bedrock limit of 64 characters: '{}' ({} chars)",
				name,
				name.len()
			)
			.into(),
		));
	}

	// Check pattern: ^[a-zA-Z0-9_-]+$
	let valid_chars = name
		.chars()
		.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

	if !valid_chars {
		return Err(AIError::MissingField(
			format!("Tool name contains invalid characters. Only alphanumeric, underscore, and hyphen are allowed: '{}'", name).into()
		));
	}

	Ok(())
}

/// Translate single Anthropic tool to Bedrock tool specification
fn translate_tool(anthropic_tool: anthropic::Tool) -> Result<bedrock::Tool, AIError> {
	// Validate tool name according to Anthropic specs
	validate_tool_name(&anthropic_tool.name)?;

	let tool_spec = bedrock::ToolSpecification {
		name: anthropic_tool.name,
		description: anthropic_tool.description,
		input_schema: Some(bedrock::ToolInputSchema::Json(anthropic_tool.input_schema)),
	};

	// Check if cache control is present - if so, we'd need to add cache points
	if anthropic_tool.cache_control.is_some() {
		// Cache points for tools would be handled here in a full implementation
		// For now, just create the tool spec
	}

	Ok(bedrock::Tool::ToolSpec(tool_spec))
}

/// Retry helper for URL fetching operations with exponential backoff
async fn retry_with_backoff<F, Fut, T>(
	operation: F,
	max_retries: u32,
	base_delay_ms: u64,
	operation_name: &str,
) -> Result<T, AIError>
where
	F: Fn() -> Fut,
	Fut: std::future::Future<Output = Result<T, AIError>>,
{
	let mut last_error = None;

	for attempt in 0..=max_retries {
		match operation().await {
			Ok(result) => {
				if attempt > 0 {
					tracing::info!(
						"Retry succeeded for {}: attempt {}/{}",
						operation_name,
						attempt + 1,
						max_retries + 1
					);
				}
				return Ok(result);
			},
			Err(e) => {
				last_error = Some(e);

				if attempt < max_retries {
					let delay_ms = base_delay_ms * 2_u64.pow(attempt);
					tracing::warn!(
						"Retry attempt {}/{} failed for {}, retrying in {}ms: {}",
						attempt + 1,
						max_retries + 1,
						operation_name,
						delay_ms,
						last_error.as_ref().unwrap()
					);
					sleep(Duration::from_millis(delay_ms)).await;
				}
			},
		}
	}

	Err(last_error.unwrap_or_else(|| AIError::RequestTooLarge))
}

/// Translate Anthropic tool choice to Bedrock tool choice
/// Note: ToolChoice::None is handled at a higher level and will not reach this function
fn translate_tool_choice(anthropic_choice: anthropic::ToolChoice) -> bedrock::ToolChoice {
	match anthropic_choice {
		anthropic::ToolChoice::Auto { .. } => {
			bedrock::ToolChoice::Auto(bedrock::AutoToolChoice {
				auto: serde_json::Value::Object(serde_json::Map::new()), // Empty object {}
			})
		},
		anthropic::ToolChoice::Any { .. } => {
			bedrock::ToolChoice::Any(bedrock::AnyToolChoice {
				any: serde_json::Value::Object(serde_json::Map::new()), // Empty object {}
			})
		},
		anthropic::ToolChoice::Tool { name, .. } => {
			bedrock::ToolChoice::Tool(bedrock::ToolChoiceSpecific {
				tool: bedrock::ToolChoiceToolSpec { name },
			})
		},
		anthropic::ToolChoice::None => {
			// This should never be reached as None is handled in translate_tools_and_choice
			unreachable!("ToolChoice::None should be handled at a higher level")
		},
	}
}

/// Fetch image from URL and encode as base64 for Bedrock compatibility
#[tracing::instrument(skip_all, fields(url = %url))]
async fn fetch_and_encode_image(url: &str) -> Result<(bedrock::ImageFormat, String), AIError> {
	// URL fetching not supported - rest of codebase doesn't use HTTP clients
	Err(AIError::UnsupportedContent)
}

/// Fetch PDF from URL and encode as base64 for Bedrock compatibility
async fn fetch_and_encode_pdf(_url: &str) -> Result<String, AIError> {
	// URL fetching not supported - rest of codebase doesn't use HTTP clients
	Err(AIError::UnsupportedContent)
}

/// Flatten content blocks to text representation
fn flatten_content_blocks_to_text(blocks: &[RequestContentBlock]) -> Result<String, AIError> {
	let mut text_parts = Vec::new();

	for block in blocks {
		match block {
			RequestContentBlock::Text(text_block) => {
				text_parts.push(text_block.text.clone());
			},
			RequestContentBlock::Image(_) => {
				text_parts.push("[Image]".to_string());
			},
			RequestContentBlock::Document(_) => {
				text_parts.push("[Document]".to_string());
			},
			RequestContentBlock::ToolUse(tool_use) => {
				text_parts.push(format!("[Tool: {}]", tool_use.name));
			},
			RequestContentBlock::ToolResult(tool_result) => {
				text_parts.push(format!("[Tool Result: {}]", tool_result.tool_use_id));
			},
			RequestContentBlock::Thinking(thinking) => {
				text_parts.push(format!(
					"[Thinking: {}]",
					thinking.thinking.chars().take(100).collect::<String>()
				));
			},
			RequestContentBlock::SearchResult(search_result) => {
				text_parts.push(format!("[Search: {}]", search_result.title));
			},
		}
	}

	Ok(text_parts.join("\n"))
}

/// Flatten search result to text representation
fn flatten_search_result_to_text(
	search_result: &anthropic::RequestSearchResultBlock,
) -> Result<String, AIError> {
	let mut parts = Vec::new();

	parts.push(format!("Title: {}", search_result.title));
	parts.push(format!("Source: {}", search_result.source));

	// Extract text from content blocks
	let content_text: Vec<String> = search_result
		.content
		.iter()
		.map(|block| block.text.clone())
		.collect();

	if !content_text.is_empty() {
		let combined_content = content_text.join(" ");
		parts.push(format!(
			"Content: {}",
			combined_content.chars().take(500).collect::<String>()
		));
	}

	Ok(parts.join("\n"))
}

/// Generate a unique response ID for Anthropic responses
fn generate_response_id() -> String {
	format!("msg_{:016x}", Utc::now().timestamp_millis())
}

/// Translate Bedrock stop reason to Anthropic stop reason
fn translate_stop_reason(bedrock_stop_reason: bedrock::StopReason) -> anthropic::StopReason {
	match bedrock_stop_reason {
		bedrock::StopReason::EndTurn => anthropic::StopReason::EndTurn,
		bedrock::StopReason::MaxTokens => anthropic::StopReason::MaxTokens,
		bedrock::StopReason::StopSequence => anthropic::StopReason::StopSequence,
		bedrock::StopReason::ToolUse => anthropic::StopReason::ToolUse,
		bedrock::StopReason::ContentFiltered => anthropic::StopReason::Refusal, // Map content filter to refusal
		bedrock::StopReason::GuardrailIntervened => anthropic::StopReason::Refusal, // Map guardrails to refusal
	}
}

/// Translate Bedrock usage to Anthropic usage
fn translate_usage(
	bedrock_usage: Option<bedrock::TokenUsage>,
) -> Result<anthropic::Usage, AIError> {
	match bedrock_usage {
		Some(usage) => Ok(anthropic::Usage {
			input_tokens: usage.input_tokens as u32,
			output_tokens: usage.output_tokens as u32,
			cache_creation_input_tokens: usage.cache_write_input_tokens.map(|t| t as u32),
			cache_read_input_tokens: usage.cache_read_input_tokens.map(|t| t as u32),
			cache_creation: None, // Bedrock doesn't provide detailed cache creation breakdown
			server_tool_use: None, // Bedrock doesn't track server tool usage
			service_tier: None,   // Bedrock doesn't expose service tier
		}),
		None => Err(AIError::MissingField("usage information".into())),
	}
}

/// Translate Bedrock response to Anthropic MessagesResponse
pub fn translate_response(
	bedrock_response: ConverseResponse,
	model_id: &str,
) -> Result<MessagesResponse, AIError> {
	// Extract the message from the response output
	let output_message = match bedrock_response.output {
		Some(bedrock::ConverseOutput::Message { message }) => message,
		None => {
			return Err(AIError::MissingField(
				"output message in Bedrock response".into(),
			));
		},
	};

	// Translate content blocks
	let content = translate_response_content_blocks(output_message.content)?;

	// Translate stop reason
	let stop_reason = bedrock_response.stop_reason.map(translate_stop_reason);

	// Translate usage information
	let usage = translate_usage(bedrock_response.usage)?;

	// Generate response ID (Bedrock doesn't provide one)
	let id = generate_response_id();

	// Build the Anthropic response
	let anthropic_response = MessagesResponse {
		id,
		r#type: "message".to_string(),
		role: "assistant".to_string(), // Always assistant for responses
		content,
		model: model_id.to_string(),
		stop_reason,
		stop_sequence: None, // Bedrock doesn't provide matched stop sequence details
		usage,
		container: None, // Bedrock doesn't provide container information
	};

	Ok(anthropic_response)
}

/// Translate Bedrock content blocks to Anthropic response content blocks
fn translate_response_content_blocks(
	bedrock_blocks: Vec<ContentBlock>,
) -> Result<Vec<ResponseContentBlock>, AIError> {
	bedrock_blocks
		.into_iter()
		.filter_map(|block| translate_response_content_block(block).transpose())
		.collect()
}

/// Translate single Bedrock content block to Anthropic response content block
fn translate_response_content_block(
	bedrock_block: ContentBlock,
) -> Result<Option<ResponseContentBlock>, AIError> {
	let anthropic_block = match bedrock_block {
		ContentBlock::Text(text) => {
			Some(ResponseContentBlock::Text(anthropic::ResponseTextBlock {
				text,
				citations: None, // Bedrock text doesn't include citation information
			}))
		},

		ContentBlock::ToolUse(tool_use) => Some(ResponseContentBlock::ToolUse(
			anthropic::ResponseToolUseBlock {
				id: tool_use.tool_use_id,
				name: tool_use.name,
				input: tool_use.input,
			},
		)),

		ContentBlock::ReasoningContent(reasoning_content) => {
			// Convert reasoning content to thinking blocks
			if let Some(reasoning_text) = reasoning_content.reasoning_text {
				Some(ResponseContentBlock::Thinking(
					anthropic::ResponseThinkingBlock {
						thinking: reasoning_text.text,
						signature: reasoning_text.signature.unwrap_or_default(),
					},
				))
			} else if reasoning_content.redacted_content.is_some() {
				// Handle redacted content - map to redacted thinking block
				Some(ResponseContentBlock::RedactedThinking(
					anthropic::ResponseRedactedThinkingBlock {
						data: reasoning_content.redacted_content.unwrap_or_default(),
					},
				))
			} else {
				None // Invalid reasoning block
			}
		},

		// Skip blocks that don't have direct Anthropic equivalents
		ContentBlock::Image { .. } => None, // Images in responses are rare
		ContentBlock::Document { .. } => None, // Documents in responses are rare
		ContentBlock::ToolResult { .. } => None, // Tool results shouldn't be in assistant responses
		ContentBlock::CachePoint { .. } => None, // Cache points are metadata, not content
	};

	Ok(anthropic_block)
}

/// Validate translated Anthropic response for correctness
pub fn validate_anthropic_response(response: &MessagesResponse) -> Result<(), AIError> {
	// Validate basic fields are present
	if response.id.is_empty() {
		return Err(AIError::MissingField("response ID cannot be empty".into()));
	}

	if response.model.is_empty() {
		return Err(AIError::MissingField(
			"response model cannot be empty".into(),
		));
	}

	if response.role != "assistant" {
		return Err(AIError::MissingField(
			"response role must be 'assistant'".into(),
		));
	}

	// Validate usage information
	if response.usage.input_tokens == 0 && response.usage.output_tokens == 0 {
		tracing::warn!("Response has zero tokens reported in usage");
	}

	// Validate content blocks if present
	for (index, block) in response.content.iter().enumerate() {
		match block {
			ResponseContentBlock::Text(text_block) => {
				// Text blocks should have non-empty text (though empty is technically valid)
				if text_block.text.is_empty() {
					tracing::debug!(block_index = index, "Empty text block in response");
				}
			},
			ResponseContentBlock::ToolUse(tool_block) => {
				// Tool use blocks should have non-empty ID and name
				if tool_block.id.is_empty() {
					return Err(AIError::MissingField(
						format!("tool use block {} has empty ID", index).into(),
					));
				}
				if tool_block.name.is_empty() {
					return Err(AIError::MissingField(
						format!("tool use block {} has empty name", index).into(),
					));
				}
			},
			ResponseContentBlock::Thinking(thinking_block) => {
				// Thinking blocks should have content
				if thinking_block.thinking.is_empty() {
					tracing::debug!(block_index = index, "Empty thinking block in response");
				}
			},
			ResponseContentBlock::RedactedThinking(_) => {
				// Redacted thinking blocks are always valid
			},
		}
	}

	Ok(())
}

/// Extract Bedrock error type from status code and error response
pub fn extract_bedrock_error_type(
	status_code: u16,
	error_response: Option<&super::types::ConverseErrorResponse>,
) -> Option<String> {
	// Map common HTTP status codes to Anthropic error types
	match status_code {
		400 => Some("invalid_request_error".to_string()),
		401 => Some("authentication_error".to_string()),
		403 => Some("permission_error".to_string()),
		404 => Some("not_found_error".to_string()),
		429 => Some("rate_limit_error".to_string()),
		500 => Some("api_error".to_string()),
		502 | 503 | 504 => Some("api_error".to_string()),
		_ => {
			// Try to extract error type from Bedrock error response
			error_response.and_then(|e| e.error_type.as_ref()).map(|t| {
				match t.as_str() {
					"ValidationException" => "invalid_request_error",
					"ThrottlingException" => "rate_limit_error",
					"AccessDeniedException" => "permission_error",
					"ResourceNotFoundException" => "not_found_error",
					"InternalServerException" => "api_error",
					"ServiceUnavailableException" => "overloaded_error",
					_ => "api_error",
				}
				.to_string()
			})
		},
	}
}

/// Translate Bedrock error response to Anthropic error response
pub fn translate_error_response(
	bedrock_error: super::types::ConverseErrorResponse,
	error_type: Option<&str>,
) -> Result<anthropic::MessagesErrorResponse, AIError> {
	let anthropic_error_type = error_type.unwrap_or("api_error").to_string();

	let anthropic_error = anthropic::ApiError {
		error_type: anthropic_error_type,
		message: bedrock_error.message,
	};

	Ok(anthropic::MessagesErrorResponse {
		response_type: "error".to_string(),
		error: anthropic_error,
	})
}

/// Extract Bedrock model path from model ID and streaming mode
pub fn extract_bedrock_model_path(model_id: &str, is_streaming: bool) -> String {
	let endpoint = if is_streaming {
		"converse-stream"
	} else {
		"converse"
	};

	format!("/model/{}/{}", model_id, endpoint)
}
