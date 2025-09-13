//! Model mapping for Anthropic → Bedrock translation

use crate::llm::AIError;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// Simple model mapping for name resolution (matches vendor implementation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMap {
	/// Model name mappings (external_name -> bedrock_model_id)
	#[serde(default)]
	pub mappings: HashMap<String, String>,
}

impl ModelMap {
	/// Create new empty model map
	pub fn new() -> Self {
		Self {
			mappings: HashMap::new(),
		}
	}

	/// Create model map with default Anthropic → Bedrock mappings
	pub fn with_defaults() -> Self {
		let mut map = Self::new();
		map.initialize_built_in_mappings();
		map
	}

	/// Initialize built-in model mappings (matches vendor implementation)
	fn initialize_built_in_mappings(&mut self) {
		// Claude 4.1 Opus
		self.add_mapping(
			"claude-opus-4-1-20250805",
			"us.anthropic.claude-opus-4-1-20250805-v1:0",
		);

		// Claude 4 Opus
		self.add_mapping(
			"claude-opus-4-20250514",
			"us.anthropic.claude-opus-4-20250514-v1:0",
		);

		// Claude 4 Sonnet
		self.add_mapping(
			"claude-sonnet-4-20250514",
			"us.anthropic.claude-sonnet-4-20250514-v1:0",
		);

		// Claude 3.7 Sonnet
		self.add_mapping(
			"claude-3-7-sonnet-20250219",
			"us.anthropic.claude-3-7-sonnet-20250219-v1:0",
		);

		// Claude 3.5 Sonnet (newer)
		self.add_mapping(
			"claude-3-5-sonnet-20241022",
			"us.anthropic.claude-3-5-sonnet-20241022-v2:0",
		);

		// Claude 3.5 Sonnet (older)
		self.add_mapping(
			"claude-3-5-sonnet-20240620",
			"anthropic.claude-3-5-sonnet-20240620-v1:0",
		);

		// Claude 3.5 Haiku
		self.add_mapping(
			"claude-3-5-haiku-20241022",
			"us.anthropic.claude-3-5-haiku-20241022-v1:0",
		);

		// Claude 3 Haiku
		self.add_mapping(
			"claude-3-haiku-20240307",
			"anthropic.claude-3-haiku-20240307-v1:0",
		);

		// Claude 3 Opus
		self.add_mapping(
			"claude-3-opus-20240229",
			"anthropic.claude-3-opus-20240229-v1:0",
		);

		// Claude 3 Sonnet
		self.add_mapping(
			"claude-3-sonnet-20240229",
			"anthropic.claude-3-sonnet-20240229-v1:0",
		);

		// Legacy shortcuts - point to latest stable versions
		self.add_mapping(
			"claude-3-5-sonnet",
			"us.anthropic.claude-3-5-sonnet-20241022-v2:0",
		);
		self.add_mapping("claude-3-opus", "anthropic.claude-3-opus-20240229-v1:0");
		self.add_mapping("claude-3-sonnet", "anthropic.claude-3-sonnet-20240229-v1:0");
		self.add_mapping("claude-3-haiku", "anthropic.claude-3-haiku-20240307-v1:0");

		debug!(
			"Created default model mapping with {} entries",
			self.mappings.len()
		);
	}

	pub fn add_mapping<S1: Into<String>, S2: Into<String>>(
		&mut self,
		external: S1,
		bedrock: S2,
	) -> &mut Self {
		let external = external.into();
		let bedrock = bedrock.into();
		debug!("Adding model mapping: '{}' -> '{}'", external, bedrock);
		self.mappings.insert(external, bedrock);
		self
	}

	pub fn remove_mapping(&mut self, external: &str) -> Option<String> {
		self.mappings.remove(external)
	}

	pub fn resolve(&self, external: &str) -> Option<&str> {
		self.mappings.get(external).map(|s| s.as_str())
	}

	pub fn has_mapping(&self, external: &str) -> bool {
		self.mappings.contains_key(external)
	}

	pub fn mappings(&self) -> &HashMap<String, String> {
		&self.mappings
	}

	pub fn len(&self) -> usize {
		self.mappings.len()
	}

	pub fn is_empty(&self) -> bool {
		self.mappings.is_empty()
	}

	pub fn clear(&mut self) {
		debug!("Clearing all model mappings");
		self.mappings.clear();
	}

	/// Resolve external model name to Bedrock model ID
	pub fn resolve_model(&self, model_name: &str) -> Result<String, AIError> {
		debug!(model_name = %model_name, "Resolving model");

		// Check if we have a mapping for this model
		if let Some(bedrock_id) = self.resolve(model_name) {
			debug!(original = %model_name, resolved = %bedrock_id, "Model resolved via mapping");
			return Ok(bedrock_id.to_string());
		}

		// If no mapping exists, validate the model name as-is
		if is_valid_bedrock_model_id(model_name) {
			debug!(model = %model_name, "Model name is already valid Bedrock ID");
			return Ok(model_name.to_string());
		}

		// If we can't resolve it, return an error
		Err(AIError::UnknownModel)
	}
}

impl Default for ModelMap {
	fn default() -> Self {
		Self::with_defaults()
	}
}

/// Check if a string is a valid Bedrock model ID (matches vendor implementation)
pub fn is_valid_bedrock_model_id(model_id: &str) -> bool {
	model_id.contains('.') && model_id.contains(':')
}

/// Global model mapper instance (matches vendor pattern)
pub static GLOBAL_MODEL_MAP: Lazy<ModelMap> = Lazy::new(|| ModelMap::with_defaults());

pub fn resolve_model_global(model_name: &str) -> Result<String, AIError> {
	GLOBAL_MODEL_MAP.resolve_model(model_name)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_built_in_model_resolution() {
		let map = ModelMap::with_defaults();

		// Test basic aliases
		let resolved = map.resolve_model("claude-3-sonnet").unwrap();
		assert_eq!(resolved, "anthropic.claude-3-sonnet-20240229-v1:0");

		// Test newer model with us.anthropic prefix
		let resolved = map.resolve_model("claude-3-5-sonnet").unwrap();
		assert_eq!(resolved, "us.anthropic.claude-3-5-sonnet-20241022-v2:0");

		// Test exact model ID
		let resolved = map.resolve_model("claude-3-5-haiku-20241022").unwrap();
		assert_eq!(resolved, "us.anthropic.claude-3-5-haiku-20241022-v1:0");
	}

	#[test]
	fn test_custom_model_mapping() {
		let mut map = ModelMap::new();
		map.add_mapping("my-claude", "anthropic.claude-3-sonnet-20240229-v1:0");

		let resolved = map.resolve_model("my-claude").unwrap();
		assert_eq!(resolved, "anthropic.claude-3-sonnet-20240229-v1:0");
	}

	#[test]
	fn test_valid_bedrock_id_passthrough() {
		let map = ModelMap::new();

		// Valid Bedrock ID should pass through as-is
		let resolved = map
			.resolve_model("anthropic.claude-3-sonnet-20240229-v1:0")
			.unwrap();
		assert_eq!(resolved, "anthropic.claude-3-sonnet-20240229-v1:0");
	}

	#[test]
	fn test_invalid_model_error() {
		let map = ModelMap::new();
		let result = map.resolve_model("invalid-model-name");
		assert!(result.is_err());
	}

	#[test]
	fn test_bedrock_id_validation() {
		assert!(is_valid_bedrock_model_id(
			"anthropic.claude-3-sonnet-20240229-v1:0"
		));
		assert!(is_valid_bedrock_model_id("us.anthropic.claude-4-opus-v1:0"));
		assert!(!is_valid_bedrock_model_id("claude-3-sonnet"));
		assert!(!is_valid_bedrock_model_id("invalid"));
	}
}
