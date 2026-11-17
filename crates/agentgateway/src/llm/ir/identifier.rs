//! Stable shortening of client identifiers to fit provider limits.

use agent_llm_ir::IdentifierPseudonymizer;

pub(super) struct IdentifierHasher;

impl IdentifierPseudonymizer for IdentifierHasher {
	fn pseudonymize(&self, identifier: &str) -> String {
		hex::encode(crate::crypto::digest::sha256(identifier.as_bytes()))
	}
}
