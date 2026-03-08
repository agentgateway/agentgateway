use crate::*;
use rmcp::model::ServerInfo;

#[apply(schema!)]
pub struct Policy {}

#[apply(schema!)]
#[serde(tag = "t")]
pub enum SessionState {
	#[serde(rename = "http")]
	HTTP(HTTPSessionState),
	#[serde(rename = "mcp")]
	MCPSnapshot(MCPSnapshotState),
	#[serde(rename = "mcpi")]
	MCPInstanceRef(MCPInstanceRefState),
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
pub struct MCPInstanceRefState {
	// Live-only MCP sessions use an instance ref instead of a full snapshot so
	// another pod can classify the handle as stale rather than malformed.
	#[serde(rename = "i")]
	pub instance_id: String,
	#[serde(rename = "s")]
	pub session_id: String,
}

impl MCPInstanceRefState {
	pub fn new(instance_id: impl Into<String>, session_id: impl Into<String>) -> Self {
		Self {
			instance_id: instance_id.into(),
			session_id: session_id.into(),
		}
	}
}

#[apply(schema!)]
/// Encodes the authoritative resume contract for multiplexed MCP sessions.
///
/// Snapshots are exact-by-name and carry both upstream session state and the
/// downstream-visible routing shape. Older positional payloads are
/// intentionally unsupported.
pub struct MCPSnapshotState {
	#[serde(rename = "m")]
	pub members: Vec<MCPSnapshotMember>,
	#[serde(rename = "h")]
	pub routing: MCPSnapshotRouting,
	/// When an upstream has no session, we need to add our own randomness to avoid session collisions.
	/// This is mostly for logging/etc purposes
	#[serde(default, rename = "r", skip_serializing_if = "Option::is_none")]
	random_identifier: Option<String>,
}

#[apply(schema!)]
pub struct MCPSnapshotRouting {
	#[serde(default, rename = "d", skip_serializing_if = "Option::is_none")]
	pub default_target_name: Option<String>,
	#[serde(default, rename = "m", skip_serializing_if = "is_false")]
	pub is_multiplexing: bool,
}

fn session_id() -> String {
	uuid::Uuid::new_v4().to_string()
}

impl MCPSnapshotState {
	pub fn new(members: Vec<MCPSnapshotMember>, routing: MCPSnapshotRouting) -> Self {
		let random_identifier = if members.iter().any(|s| s.session.is_none()) {
			Some(session_id())
		} else {
			None
		};
		Self {
			members,
			routing,
			random_identifier,
		}
	}
}

fn is_false(v: &bool) -> bool {
	!*v
}

#[apply(schema!)]
pub struct MCPSession {
	#[serde(default, rename = "s", skip_serializing_if = "Option::is_none")]
	pub session: Option<String>,
	#[serde(default, rename = "b", skip_serializing_if = "Option::is_none")]
	pub backend: Option<SocketAddr>,
}

#[apply(schema!)]
pub struct MCPSnapshotMember {
	#[serde(rename = "n")]
	pub target: String,
	#[serde(default, rename = "s", skip_serializing_if = "Option::is_none")]
	pub session: Option<String>,
	#[serde(default, rename = "b", skip_serializing_if = "Option::is_none")]
	pub backend: Option<SocketAddr>,
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
	#[serde(rename = "i")]
	pub info: ServerInfo,
	#[serde(rename = "f")]
	pub target_fingerprint: String,
}

impl MCPSnapshotMember {
	pub fn new(
		target: impl Into<String>,
		session: MCPSession,
		info: ServerInfo,
		target_fingerprint: impl Into<String>,
	) -> Self {
		Self {
			target: target.into(),
			session: session.session,
			backend: session.backend,
			info,
			target_fingerprint: target_fingerprint.into(),
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("invalid session encoding")]
	InvalidSessionEncoding,
	#[error("invalid session format: {0}")]
	InvalidSessionFormat(#[from] serde_json::Error),
	#[error("multiplexed MCP sessions require encrypted session ids")]
	EncryptedSessionIdsRequired,
	#[error("encryption: {0}")]
	Encryption(#[from] aes::Error),
}

/// Encodes client-visible MCP identifiers.
///
/// `Base64` keeps identifiers ASCII-safe without adding authenticity or
/// confidentiality. `Aes` encrypts the payload so multiplex session state can be
/// carried by the client without being forgeable or inspectable.
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
		let e = aes::Encoder::new(key.as_ref())?;
		Ok(Encoder::Aes(Arc::new(e)))
	}

	pub const fn is_encrypted(&self) -> bool {
		matches!(self, Encoder::Aes(_))
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
}
