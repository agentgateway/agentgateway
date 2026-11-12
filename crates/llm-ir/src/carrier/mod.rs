//! # Replay state and client identifiers
//!
//! Some providers return reasoning state that must be sent back on a later turn.
//! The crate stores that state in fields the client already carries, such as a
//! Messages signature, a Responses reasoning item, a Gemini `thoughtSignature`, or
//! a Chat Completions tool-call ID.
//!
//! The encoded state includes the backend, provider, mapped model, and client
//! field. Opening the state checks that they match the expected destination.
//! Carriers are readable and editable client input; they provide no confidentiality
//! or proof that the gateway issued their contents.
//!
//! [`IdentifierPseudonymizer`] replaces long client identifiers with stable values.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Version marker for the encoded replay state.
pub const PREFIX: &str = "agentgateway.llm-ir.v1:";

/// Backend, provider, and model identity bound to replay state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateScope<'a> {
	pub backend: &'a str,
	pub provider: &'a str,
	pub model: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CarrierPurpose {
	ChatCallId,
	GeminiThoughtSignature,
	MessagesSignature,
	ResponsesEncryptedContent,
}

impl CarrierPurpose {
	const fn label(self) -> &'static [u8] {
		match self {
			Self::ChatCallId => b"chat-call-id",
			Self::GeminiThoughtSignature => b"gemini-thought-signature",
			Self::MessagesSignature => b"messages-signature",
			Self::ResponsesEncryptedContent => b"responses-encrypted-content",
		}
	}
}

/// Why replay state could not be opened.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CarrierError {
	#[error("unknown carrier format")]
	UnknownFormat,
	#[error("malformed carrier")]
	Malformed,
	#[error("carrier does not match this destination")]
	WrongScope,
}

/// Encodes replay state for a client-carried field.
pub(crate) fn encode(scope: StateScope<'_>, purpose: CarrierPurpose, state: &[u8]) -> String {
	let scoped = scoped_state(scope, purpose, state);
	let mut out = String::with_capacity(PREFIX.len() + scoped.len() * 4 / 3 + 4);
	out.push_str(PREFIX);
	URL_SAFE_NO_PAD.encode_string(&scoped, &mut out);
	out
}

/// [`encode`] without [`PREFIX`], for a field that marks the carrier itself. The result is
/// base64url, so it uses only `[A-Za-z0-9_-]`.
pub(crate) fn encode_unprefixed(
	scope: StateScope<'_>,
	purpose: CarrierPurpose,
	state: &[u8],
) -> String {
	let scoped = scoped_state(scope, purpose, state);
	URL_SAFE_NO_PAD.encode(&scoped)
}

/// Opens replay state only when its format and destination match.
pub(crate) fn open(
	scope: StateScope<'_>,
	purpose: CarrierPurpose,
	carrier: &str,
) -> Result<Vec<u8>, CarrierError> {
	let carrier = carrier
		.strip_prefix(PREFIX)
		.ok_or(CarrierError::UnknownFormat)?;
	open_unprefixed(scope, purpose, carrier)
}

/// Opens a carrier written by [`encode_unprefixed`].
pub(crate) fn open_unprefixed(
	scope: StateScope<'_>,
	purpose: CarrierPurpose,
	carrier: &str,
) -> Result<Vec<u8>, CarrierError> {
	let state = decode(carrier)?;
	let scope = scope_bytes(scope, purpose);
	state
		.strip_prefix(scope.as_slice())
		.map(<[u8]>::to_vec)
		.ok_or(CarrierError::WrongScope)
}

/// Length prefixes distinguish scopes whose fields concatenate to the same bytes.
fn scope_bytes(scope: StateScope<'_>, purpose: CarrierPurpose) -> Vec<u8> {
	let mut out = Vec::with_capacity(PREFIX.len() + 64);
	out.extend_from_slice(PREFIX.as_bytes());
	for field in [
		scope.backend.as_bytes(),
		scope.provider.as_bytes(),
		scope.model.as_bytes(),
		purpose.label(),
	] {
		out.extend_from_slice(&(field.len() as u64).to_be_bytes());
		out.extend_from_slice(field);
	}
	out
}

fn scoped_state(scope: StateScope<'_>, purpose: CarrierPurpose, state: &[u8]) -> Vec<u8> {
	let mut out = scope_bytes(scope, purpose);
	out.extend_from_slice(state);
	out
}

fn decode(text: &str) -> Result<Vec<u8>, CarrierError> {
	URL_SAFE_NO_PAD
		.decode(text)
		.map_err(|_| CarrierError::Malformed)
}

/// Replaces a client identifier with a stable value that fits provider limits.
pub trait IdentifierPseudonymizer: Send + Sync {
	/// Returns the same value for the same identifier. The result must be at most
	/// 64 bytes, so it fits every provider field where the IR sends it.
	fn pseudonymize(&self, identifier: &str) -> String;
}

#[cfg(test)]
mod tests;
