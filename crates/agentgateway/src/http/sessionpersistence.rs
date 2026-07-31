use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::cel::{Executor, Expression, Value};
use crate::*;

/// Pins requests that carry the same affinity value to the same healthy service endpoint.
///
/// This policy is intentionally stateless. Every gateway replica computes the same rendezvous
/// hash from the request affinity value and the currently available endpoint set.
#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "SessionPersistencePolicy"))]
pub struct Policy {
	/// CEL expression evaluated against request state. It must return a string or bytes value.
	/// Examples: `request.headers["x-session-id"]` or `string(source.address)`.
	pub source: Arc<Expression>,
}

impl Policy {
	/// Returns a stable, non-secret affinity key suitable for endpoint selection.
	/// Evaluation errors, non-string/bytes values, and empty values disable persistence for that
	/// request (it falls back to normal load balancing). Each miss is logged at trace level so a
	/// misconfigured expression or a missing header can be diagnosed without failing the request.
	pub fn affinity_key(&self, req: &http::Request) -> Option<[u8; 32]> {
		let executor = Executor::new_request(req);
		let value = match executor.eval(&self.source) {
			Ok(value) => value,
			Err(err) => {
				trace!(
					expression = %self.source.original_expression,
					error = %err,
					"session affinity: expression evaluation failed; falling back to load balancing"
				);
				return None;
			},
		};
		let bytes: &[u8] = match &value {
			Value::String(value) => value.as_ref().as_bytes(),
			Value::Bytes(value) => value.as_ref(),
			other => {
				trace!(
					expression = %self.source.original_expression,
					value_type = other.type_of().as_str(),
					"session affinity: expression did not return a string or bytes value; falling back to load balancing"
				);
				return None;
			},
		};
		if bytes.is_empty() {
			trace!(
				expression = %self.source.original_expression,
				"session affinity: expression returned an empty value; falling back to load balancing"
			);
			return None;
		}
		Some(Sha256::digest(bytes).into())
	}

	pub fn register_expressions(&self, ctx: &mut crate::cel::ContextBuilder) {
		ctx.register_expression(self.source.as_ref());
	}
}

#[apply(schema!)]
#[serde(tag = "t")]
pub enum SessionState {
	#[serde(rename = "http")]
	HTTP(HTTPSessionState),
	#[serde(rename = "mcp")]
	MCP(MCPSessionState),
}

impl SessionState {
	pub fn encode(&self, encoder: &Encoder) -> Result<String, Error> {
		encoder.encrypt(&serde_json::to_string(self)?)
	}

	pub fn decode(session_id: &str, encoder: &Encoder) -> Result<SessionState, Error> {
		let session = encoder.decrypt(session_id)?;
		let state = serde_json::from_slice::<SessionState>(&session)
			.map_err(|_| Error::InvalidSessionEncoding)?;
		Ok(state)
	}
}

#[apply(schema!)]
pub struct HTTPSessionState {
	pub backend: SocketAddr,
}

#[apply(schema!)]
pub struct MCPSessionState {
	#[serde(rename = "s")]
	pub sessions: Vec<MCPSession>,
	/// When an upstream has no session, we need to add our own randomness to avoid session collisions.
	/// This is mostly for logging/etc purposes
	#[serde(default, rename = "r", skip_serializing_if = "Option::is_none")]
	random_identifier: Option<String>,
}

fn session_id() -> String {
	uuid::Uuid::new_v4().to_string()
}

impl MCPSessionState {
	pub fn new(sessions: Vec<MCPSession>) -> Self {
		let random_identifier = if sessions.iter().any(|s| s.session.is_none()) {
			Some(session_id())
		} else {
			None
		};
		Self {
			sessions,
			random_identifier,
		}
	}
}

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub struct MCPSession {
	#[serde(default, rename = "t", skip_serializing_if = "Option::is_none")]
	pub target_name: Option<String>,
	#[serde(default, rename = "s", skip_serializing_if = "Option::is_none")]
	pub session: Option<String>,
	#[serde(default, rename = "b", skip_serializing_if = "Option::is_none")]
	pub backend: Option<SocketAddr>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("invalid session encoding")]
	InvalidSessionEncoding,
	#[error("invalid session format: {0}")]
	InvalidSessionFormat(#[from] serde_json::Error),
	#[error("encryption: {0}")]
	Encryption(#[from] aes::Error),
}

#[derive(Debug, Clone)]
pub enum Encoder {
	Base64(base64::Encoder),
	Aes(Arc<aes::Encoder>),
}

impl Encoder {
	pub fn base64() -> Encoder {
		Encoder::Base64(base64::Encoder)
	}
	pub fn aes(key: &str) -> anyhow::Result<Encoder> {
		let key = hex::decode(key)?;
		// AES-256-GCM requires a 32-byte key (64 hex characters when encoded with `openssl rand -hex 32`).
		if key.len() != 32 {
			anyhow::bail!(
				"invalid AES-256-GCM key length: expected 32 bytes (64 hex characters), got {} bytes ({} hex characters)",
				key.len(),
				key.len() * 2,
			);
		}
		let enc = aes::Encoder::new(key.as_ref())?;
		Ok(Encoder::Aes(Arc::new(enc)))
	}
}

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};

	use super::*;
	use crate::transport::stream::TCPConnectionInfo;

	fn policy(source: &str) -> Policy {
		Policy {
			source: Arc::new(Expression::new_strict(source).unwrap()),
		}
	}

	#[test]
	fn header_affinity_uses_configured_header_value() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let request = |value: &'static str| {
			let mut request = http::Request::new(http::Body::empty());
			request
				.headers_mut()
				.insert("x-session-id", http::HeaderValue::from_static(value));
			request
		};
		let first = request("session-a");
		let second = request("session-a");
		let different = request("session-b");

		assert_eq!(policy.affinity_key(&first), policy.affinity_key(&second));
		assert_ne!(policy.affinity_key(&first), policy.affinity_key(&different));
	}

	#[test]
	fn policy_deserializes_cel_source() {
		let policy: Policy = serde_yaml::from_str(
			r#"
source: request.headers["x-session-id"]
"#,
		)
		.unwrap();
		assert_eq!(
			policy.source.original_expression,
			r#"request.headers["x-session-id"]"#
		);
	}

	#[test]
	fn missing_header_falls_back_to_normal_load_balancing() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let request = http::Request::new(http::Body::empty());
		assert_eq!(policy.affinity_key(&request), None);
	}

	#[test]
	fn empty_header_value_falls_back_to_normal_load_balancing() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let mut request = http::Request::new(http::Body::empty());
		request
			.headers_mut()
			.insert("x-session-id", http::HeaderValue::from_static(""));
		assert_eq!(policy.affinity_key(&request), None);
	}

	#[test]
	fn client_ip_affinity_ignores_source_port() {
		fn request(port: u16) -> http::Request {
			let mut request = http::Request::new(http::Body::empty());
			request.extensions_mut().insert(TCPConnectionInfo {
				peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), port),
				local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80),
				start: Instant::now(),
				raw_peer_addr: None,
			});
			request
		}

		let policy = policy("string(source.address)");
		assert_eq!(
			policy.affinity_key(&request(10000)),
			policy.affinity_key(&request(20000))
		);
	}
}

impl Serialize for Encoder {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			Encoder::Base64(_) => serializer.serialize_str("base64"),
			Encoder::Aes(_) => serializer.serialize_str("aes"),
		}
	}
}

impl Encoder {
	pub fn encrypt(&self, plaintext: &str) -> Result<String, Error> {
		match self {
			Encoder::Base64(e) => Ok(e.encrypt(plaintext)),
			Encoder::Aes(e) => e.encrypt(plaintext).map_err(Into::into),
		}
	}
	pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, Error> {
		match self {
			Encoder::Base64(e) => e
				.decrypt(encoded)
				.map_err(|_| Error::InvalidSessionEncoding),
			Encoder::Aes(e) => e.decrypt(encoded).map_err(Into::into),
		}
	}
}

mod base64 {
	use base64::Engine;
	use base64::engine::general_purpose::URL_SAFE_NO_PAD;

	#[derive(Debug, Clone)]
	pub struct Encoder;

	impl Encoder {
		pub fn encrypt(&self, plaintext: &str) -> String {
			URL_SAFE_NO_PAD.encode(plaintext)
		}
		pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
			URL_SAFE_NO_PAD.decode(encoded)
		}
	}
}

mod aes {
	use aws_lc_rs::aead::{AES_256_GCM, Aad, Nonce, RandomizedNonceKey};
	use base64::Engine;
	use base64::engine::general_purpose::STANDARD;

	#[derive(Debug)]
	pub struct Encoder {
		key: RandomizedNonceKey,
	}

	impl Encoder {
		/// Create from a 32-byte key
		pub fn new(key: &[u8]) -> Result<Self, Error> {
			let key = RandomizedNonceKey::new(&AES_256_GCM, key).map_err(|_| Error::InvalidKey)?;
			Ok(Self { key })
		}

		/// Encrypt and base64 encode
		pub fn encrypt(&self, plaintext: &str) -> Result<String, Error> {
			let mut in_out: Vec<u8> = plaintext.as_bytes().to_vec();
			// Seal automatically generates a random nonce and prepends it
			let nonce = self
				.key
				.seal_in_place_append_tag(Aad::empty(), &mut in_out)
				.map_err(|_| Error::EncryptionFailed)?;

			// Format: nonce || ciphertext+tag
			let mut result = nonce.as_ref().to_vec();
			result.extend_from_slice(&in_out);
			// Base64 encode
			Ok(STANDARD.encode(&result))
		}

		/// Decode and decrypt
		pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, Error> {
			// Base64 decode
			let data = STANDARD.decode(encoded).map_err(|_| Error::InvalidFormat)?;
			if data.len() < 12 {
				return Err(Error::InvalidFormat);
			}

			// Extract nonce and ciphertext
			let (nonce_bytes, ciphertext) = data.split_at(12);
			let nonce =
				Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| Error::InvalidFormat)?;
			let mut in_out = ciphertext.to_vec();
			let plaintext = self
				.key
				.open_in_place(nonce, Aad::empty(), &mut in_out)
				.map_err(|_| Error::DecryptionFailed)?;
			Ok(plaintext.to_vec())
		}
	}

	#[derive(Debug, thiserror::Error)]
	pub enum Error {
		#[error("invalid key")]
		InvalidKey,
		#[error("encryption failed")]
		EncryptionFailed,
		#[error("decryption failed")]
		DecryptionFailed,
		#[error("invalid format")]
		InvalidFormat,
	}

	#[cfg(test)]
	mod tests {
		use base64::Engine;

		use super::{Encoder, Error};

		#[test]
		fn short_ciphertexts_fail_cleanly() {
			let encoder = Encoder::new(&[0u8; 32]).expect("encoder");
			let short = base64::engine::general_purpose::STANDARD.encode([0u8; 11]);
			assert!(matches!(encoder.decrypt(&short), Err(Error::InvalidFormat)));
		}
	}
}
