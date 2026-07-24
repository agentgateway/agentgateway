//! Mantle allow-list for AWS Bedrock: model IDs routed to Mantle under `RuntimePreferred` (empty by default).

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use anyhow::Context as _;
use arc_swap::ArcSwap;

static DEFAULT_MANTLE_MODELS_JSON: &str = include_str!("bedrock_mantle_models.json");

#[derive(serde::Deserialize)]
struct WrappedModelTable {
	models: Vec<String>,
}

/// Parses a model list: either a JSON array or the wrapped `{"source","models"}` form.
pub fn parse_model_list(json: &str) -> anyhow::Result<HashSet<String>> {
	let ids: Vec<String> = if let Ok(wrapped) = serde_json::from_str::<WrappedModelTable>(json) {
		wrapped.models
	} else {
		serde_json::from_str(json).context(
			"bedrock mantle catalog must be a JSON array or \
			 wrapped {\"source\":\"...\",\"models\":[...]}",
		)?
	};
	Ok(ids.into_iter().collect())
}

pub fn embedded_default() -> HashSet<String> {
	parse_model_list(DEFAULT_MANTLE_MODELS_JSON)
		.expect("embedded bedrock_mantle_models.json must be valid JSON")
}

static MANTLE_MODELS: OnceLock<ArcSwap<HashSet<String>>> = OnceLock::new();

fn mantle_models() -> &'static ArcSwap<HashSet<String>> {
	MANTLE_MODELS.get_or_init(|| ArcSwap::from_pointee(embedded_default()))
}

pub fn set_mantle_models(ids: HashSet<String>) {
	mantle_models().store(Arc::new(ids));
}

pub fn is_mantle_only(model_id: &str) -> bool {
	mantle_models().load().contains(model_id)
}

// Serializes tests across this crate that mutate the global MANTLE_MODELS.
#[cfg(test)]
pub(crate) static MODELS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn restore_default() {
	set_mantle_models(embedded_default());
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_default_keeps_everything_on_runtime() {
		let _lock = MODELS_LOCK.lock().unwrap();
		restore_default();
		assert!(!is_mantle_only("anthropic.claude-3-5-sonnet-20241022-v2:0"));
		assert!(!is_mantle_only("amazon.nova-pro-v1:0"));
		assert!(!is_mantle_only("openai.gpt-oss-120b"));
	}

	#[test]
	fn models_on_allow_list_are_mantle_only() {
		let _lock = MODELS_LOCK.lock().unwrap();
		set_mantle_models(["openai.gpt-oss-120b".to_string()].into());
		assert!(is_mantle_only("openai.gpt-oss-120b"));
		assert!(!is_mantle_only("anthropic.claude-3-5-sonnet-20241022-v2:0"));
		restore_default();
	}

	#[test]
	fn parse_model_list_bare_array() {
		let json = r#"["vendor.model-a", "vendor.model-b"]"#;
		let ids = parse_model_list(json).unwrap();
		assert!(ids.contains("vendor.model-a"));
		assert!(ids.contains("vendor.model-b"));
		assert_eq!(ids.len(), 2);
	}

	#[test]
	fn parse_model_list_wrapped_format() {
		let json = r#"{"source":"https://example.com","models":["vendor.model-a","vendor.model-b"]}"#;
		let ids = parse_model_list(json).unwrap();
		assert!(ids.contains("vendor.model-a"));
		assert!(ids.contains("vendor.model-b"));
		assert_eq!(ids.len(), 2);
	}

	#[test]
	fn parse_model_list_rejects_invalid_json() {
		assert!(parse_model_list(r#"not json"#).is_err());
		assert!(parse_model_list(r#"{"models": 42}"#).is_err());
	}

	#[test]
	fn embedded_default_is_valid() {
		let _ = embedded_default();
	}
}
