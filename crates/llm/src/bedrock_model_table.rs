//! Mantle allow-list for AWS Bedrock: model IDs routed to Mantle under `RuntimePreferred`, fed from the model catalog.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;

static MANTLE_MODELS: OnceLock<ArcSwap<HashSet<String>>> = OnceLock::new();

fn mantle_models() -> &'static ArcSwap<HashSet<String>> {
	MANTLE_MODELS.get_or_init(|| ArcSwap::from_pointee(HashSet::new()))
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
	set_mantle_models(HashSet::new());
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_default_keeps_everything_on_runtime() {
		let _lock = MODELS_LOCK.lock().unwrap();
		restore_default();
		assert!(!is_mantle_only("anthropic.claude-3-5-sonnet-20241022-v2:0"));
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
}
