//! Shared by the LLM IR fuzz targets; each uses only part of it.
#![allow(dead_code)]

use std::hash::{DefaultHasher, Hash, Hasher};

use agent_llm_ir::{IdentifierPseudonymizer, StateScope};

/// A short hash, so every identifier fits the 64 bytes providers allow.
pub struct Hashed;

impl IdentifierPseudonymizer for Hashed {
	fn pseudonymize(&self, identifier: &str) -> String {
		let mut hasher = DefaultHasher::new();
		identifier.hash(&mut hasher);
		format!("{:016x}", hasher.finish())
	}
}

pub const SCOPE: StateScope<'static> = StateScope {
	backend: "fuzz",
	provider: "provider",
	model: "model",
};

/// The first byte of a fuzz input chooses an event limit and a chunk size; the rest is input.
pub fn split_config(data: &[u8]) -> Option<(usize, usize, &[u8])> {
	let (&config, input) = data.split_first()?;
	let limit = [usize::MAX, 1, 16, 256][usize::from(config & 3)];
	let chunk = usize::from(config >> 2) + 1;
	Some((limit, chunk, input))
}

/// An HTTP reply body must be JSON whatever its status.
pub fn assert_json_body(body: &[u8]) {
	if let Err(err) = serde_json::from_slice::<serde_json::Value>(body) {
		panic!("{err}: {:?}", String::from_utf8_lossy(body));
	}
}
