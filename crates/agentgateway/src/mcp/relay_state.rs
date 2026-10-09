use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::proxy::ProxyError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RelayState {
	pub client_redirect_uri: String,
	pub client_state: Option<String>,
	pub expires_at_unix: u64,
}

fn encoder_for(
	relay_signing_key: &SecretString,
) -> Result<crate::http::sessionpersistence::Encoder, ProxyError> {
	use secrecy::ExposeSecret;
	crate::http::sessionpersistence::Encoder::aes(relay_signing_key.expose_secret())
		.map_err(|e| ProxyError::ProcessingString(format!("relay signing key invalid: {e}")))
}

fn now_unix() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

pub(super) fn encode(
	relay_signing_key: &SecretString,
	state: &RelayState,
) -> Result<String, ProxyError> {
	let encoder = encoder_for(relay_signing_key)?;
	let json = serde_json::to_string(state)
		.map_err(|e| ProxyError::ProcessingString(format!("relay state serialize failed: {e}")))?;
	let standard_b64 = encoder
		.encrypt(&json)
		.map_err(|e| ProxyError::ProcessingString(format!("relay state encrypt failed: {e}")))?;
	let raw = STANDARD
		.decode(&standard_b64)
		.map_err(|e| ProxyError::ProcessingString(format!("relay state re-encode failed: {e}")))?;
	Ok(URL_SAFE_NO_PAD.encode(raw))
}

pub(super) fn decode(
	relay_signing_key: &SecretString,
	token: &str,
) -> Result<RelayState, ProxyError> {
	let encoder = encoder_for(relay_signing_key)?;
	let raw = URL_SAFE_NO_PAD
		.decode(token)
		.map_err(|_| ProxyError::ProcessingString("relay state decode failed".to_string()))?;
	let standard_b64 = STANDARD.encode(raw);
	let plaintext = encoder
		.decrypt(&standard_b64)
		.map_err(|_| ProxyError::ProcessingString("relay state decrypt failed".to_string()))?;
	let state: RelayState = serde_json::from_slice(&plaintext)
		.map_err(|_| ProxyError::ProcessingString("relay state payload malformed".to_string()))?;
	if state.expires_at_unix <= now_unix() {
		return Err(ProxyError::ProcessingString(
			"relay state expired".to_string(),
		));
	}
	Ok(state)
}

#[cfg(test)]
mod tests {
	use secrecy::SecretString;

	use super::*;

	fn key(byte: u8) -> SecretString {
		SecretString::new(hex::encode([byte; 32]).into_boxed_str())
	}

	fn sample() -> RelayState {
		RelayState {
			client_redirect_uri: "http://127.0.0.1:33418/callback".to_string(),
			client_state: Some("client-csrf-xyz".to_string()),
			expires_at_unix: crate::mcp::relay_state::tests::far_future(),
		}
	}

	pub(super) fn far_future() -> u64 {
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("system clock should be after epoch")
			.as_secs()
			+ 300
	}

	#[test]
	fn round_trips_through_encode_decode() {
		let signing_key = key(0x11);
		let state = sample();

		let token = encode(&signing_key, &state).expect("encode should succeed");
		let decoded = decode(&signing_key, &token).expect("decode should succeed");

		assert_eq!(decoded, state);
	}

	#[test]
	fn rejects_token_encoded_with_a_different_key() {
		let state = sample();
		let token = encode(&key(0xAA), &state).expect("encode should succeed");

		assert!(decode(&key(0xBB), &token).is_err());
	}

	#[test]
	fn rejects_tampered_ciphertext() {
		let signing_key = key(0x11);
		let mut token = encode(&signing_key, &sample()).expect("encode should succeed");
		let mid = token.len() / 2;
		let flipped = if token.as_bytes()[mid] == b'A' {
			'B'
		} else {
			'A'
		};
		token.replace_range(mid..mid + 1, &flipped.to_string());

		assert!(decode(&signing_key, &token).is_err());
	}

	#[test]
	fn encoded_token_contains_no_reserved_standard_base64_characters() {
		let signing_key = key(0x11);
		for i in 0..20u64 {
			let mut state = sample();
			state.client_redirect_uri = format!("http://127.0.0.1:{}/callback?n={i}", 30000 + i);
			let token = encode(&signing_key, &state).expect("encode should succeed");
			assert!(
				!token.contains('+') && !token.contains('/') && !token.contains('='),
				"token should be URL-safe base64 with no STANDARD-base64-only characters: {token}"
			);
		}
	}

	#[test]
	fn rejects_expired_state() {
		let signing_key = key(0x11);
		let mut state = sample();
		state.expires_at_unix = 1;

		let token = encode(&signing_key, &state).expect("encode should succeed");

		assert!(decode(&signing_key, &token).is_err());
	}
}
