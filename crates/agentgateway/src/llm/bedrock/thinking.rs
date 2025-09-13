//! Thinking validation for Anthropic Messages

use crate::llm::AIError;
use regex::Regex;

use crate::llm::anthropic_types::{
	InputMessage, MessageRole, MessagesRequest, RequestContentBlock,
};

/// Simple thinking validator matching vendor approach
pub struct ThinkingValidator {
	signature_regex: Regex,
	strict_mode: bool,
}

impl ThinkingValidator {
	/// Create new thinking validator
	pub fn new(strict_mode: bool) -> Result<Self, AIError> {
		Ok(Self {
			signature_regex: Regex::new(r"^[A-Za-z0-9+/=_\-]{8,}$")
				.map_err(|_| AIError::MissingField("invalid regex pattern".into()))?,
			strict_mode,
		})
	}

	/// Validate thinking configuration and blocks
	pub fn validate_request(&self, req: &MessagesRequest) -> Result<(), AIError> {
		// Check if thinking is enabled
		let thinking_enabled = req
			.thinking
			.as_ref()
			.map(|t| t.thinking_type == "enabled")
			.unwrap_or(false);

		if !thinking_enabled {
			return Ok(());
		}

		// Validate thinking blocks in messages
		self.validate_thinking_blocks(&req.messages)?;

		// Validate block ordering
		self.validate_block_ordering(&req.messages)?;

		Ok(())
	}

	/// Validate thinking blocks are properly formatted
	fn validate_thinking_blocks(&self, messages: &[InputMessage]) -> Result<(), AIError> {
		for (idx, msg) in messages.iter().enumerate() {
			if msg.role != MessageRole::Assistant {
				continue;
			}

			// message.content is Vec<RequestContentBlock>
			for block in msg.content.iter() {
				self.validate_thinking_block(block, idx)?;
			}
		}
		Ok(())
	}

	/// Validate a single thinking block
	fn validate_thinking_block(
		&self,
		block: &RequestContentBlock,
		msg_idx: usize,
	) -> Result<(), AIError> {
		match block {
			RequestContentBlock::Thinking(thinking_block) => {
				// Validate signature presence
				if thinking_block.signature.is_empty() {
					return Err(AIError::MissingField(
						format!("Empty signature in thinking block at message {}", msg_idx).into(),
					));
				}

				// Validate signature format (warn only for compatibility)
				if !self.signature_regex.is_match(&thinking_block.signature) {
					tracing::warn!(
						message_index = msg_idx,
						signature_length = thinking_block.signature.len(),
						"Thinking signature has unexpected format (continuing anyway)"
					);
				}

				// Validate thinking content
				if thinking_block.thinking.is_empty() && self.strict_mode {
					return Err(AIError::MissingField(
						format!("Empty thinking content at message {}", msg_idx).into(),
					));
				}
			},
			_ => {},
		}
		Ok(())
	}

	/// Validate that thinking blocks come before other content
	fn validate_block_ordering(&self, messages: &[InputMessage]) -> Result<(), AIError> {
		for (idx, msg) in messages.iter().enumerate() {
			if msg.role != MessageRole::Assistant {
				continue;
			}

			let mut saw_thinking = false;
			let mut saw_non_thinking = false;

			// message.content is Vec<RequestContentBlock>
			for block in msg.content.iter() {
				let is_thinking = matches!(block, RequestContentBlock::Thinking(_));

				if is_thinking {
					if saw_non_thinking {
						return Err(AIError::MissingField(
							format!(
								"Thinking blocks must come before other content in message {}",
								idx
							)
							.into(),
						));
					}
					saw_thinking = true;
				} else if saw_thinking {
					saw_non_thinking = true;
				}
			}
		}
		Ok(())
	}
}

/// Simple thinking validation result
#[derive(Debug, Clone)]
pub struct ThinkingValidationResult {
	pub valid: bool,
	pub errors: Vec<String>,
	pub warnings: Vec<String>,
	pub should_disable_thinking: bool,
}

impl ThinkingValidationResult {
	pub fn success() -> Self {
		Self {
			valid: true,
			errors: vec![],
			warnings: vec![],
			should_disable_thinking: false,
		}
	}
}

/// Validate thinking request using simple approach
pub fn validate_thinking_request(
	request: &MessagesRequest,
) -> Result<ThinkingValidationResult, AIError> {
	let validator = ThinkingValidator::new(false)?;

	match validator.validate_request(request) {
		Ok(_) => Ok(ThinkingValidationResult::success()),
		Err(e) => Ok(ThinkingValidationResult {
			valid: false,
			errors: vec![e.to_string()],
			warnings: vec![],
			should_disable_thinking: false,
		}),
	}
}

/// Process thinking blocks in request (minimal implementation)
pub fn process_thinking_request(_request: &mut MessagesRequest) -> Result<(), AIError> {
	// No processing needed in simplified approach
	// Bedrock handles thinking blocks natively
	Ok(())
}
