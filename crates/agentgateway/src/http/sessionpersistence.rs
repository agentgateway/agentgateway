use crate::*;

const SESSION_KEYRING_VERSION: &str = "v1";

#[apply(schema!)]
pub struct Policy {}

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
pub struct MCPSession {
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
	KeyRing(Arc<SessionKeyringEncoder>),
}

impl Encoder {
	pub fn base64() -> Encoder {
		Encoder::Base64(base64::Encoder)
	}
	pub fn aes(key: &str) -> anyhow::Result<Encoder> {
		Ok(Encoder::Aes(Arc::new(aes_encoder_from_hex(key)?)))
	}

	pub(crate) fn session_keyring(keyring: &SessionKeyring) -> anyhow::Result<Encoder> {
		Ok(Encoder::KeyRing(Arc::new(SessionKeyringEncoder::new(
			keyring,
		)?)))
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
			Encoder::KeyRing(_) => serializer.serialize_str("aes"),
		}
	}
}

impl Encoder {
	pub fn encrypt(&self, plaintext: &str) -> Result<String, Error> {
		match self {
			Encoder::Base64(e) => Ok(e.encrypt(plaintext)),
			Encoder::Aes(e) => e.encrypt(plaintext).map_err(Into::into),
			Encoder::KeyRing(e) => e.encrypt(plaintext).map_err(Into::into),
		}
	}
	pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, Error> {
		match self {
			Encoder::Base64(e) => e
				.decrypt(encoded)
				.map_err(|_| Error::InvalidSessionEncoding),
			Encoder::Aes(e) => e.decrypt(encoded).map_err(Into::into),
			Encoder::KeyRing(e) => e.decrypt(encoded),
		}
	}
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionKeyring {
	pub version: String,
	pub primary: String,
	#[serde(default)]
	pub previous: Vec<String>,
	#[serde(default, rename = "rotatedAt")]
	pub rotated_at: Option<String>,
}

impl SessionKeyring {
	pub(crate) fn parse(encoded: &str) -> anyhow::Result<Self> {
		let keyring: Self = serde_json::from_str(encoded)?;
		keyring.validate()?;
		Ok(keyring)
	}

	fn validate(&self) -> anyhow::Result<()> {
		if self.version != SESSION_KEYRING_VERSION {
			anyhow::bail!("unsupported session keyring version {}", self.version);
		}

		let mut seen = std::collections::HashSet::new();
		validate_hex_key(&self.primary)?;
		seen.insert(self.primary.as_str());

		for previous in &self.previous {
			validate_hex_key(previous)?;
			if !seen.insert(previous.as_str()) {
				anyhow::bail!("duplicate session key material");
			}
		}

		if let Some(rotated_at) = &self.rotated_at {
			rotated_at.parse::<chrono::DateTime<chrono::Utc>>()?;
		}

		Ok(())
	}
}

#[derive(Debug, Clone)]
pub struct SessionKeyringEncoder {
	primary: Arc<aes::Encoder>,
	active: Vec<Arc<aes::Encoder>>,
}

impl SessionKeyringEncoder {
	fn new(keyring: &SessionKeyring) -> anyhow::Result<Self> {
		keyring.validate()?;

		let primary = Arc::new(aes_encoder_from_hex(&keyring.primary)?);
		let mut active = Vec::with_capacity(1 + keyring.previous.len());
		active.push(primary.clone());
		for previous in &keyring.previous {
			active.push(Arc::new(aes_encoder_from_hex(previous)?));
		}

		Ok(Self { primary, active })
	}

	fn encrypt(&self, plaintext: &str) -> Result<String, aes::Error> {
		self.primary.encrypt(plaintext)
	}

	fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, Error> {
		let mut last_err: Option<aes::Error> = None;
		for decoder in &self.active {
			match decoder.decrypt(encoded) {
				Ok(plaintext) => return Ok(plaintext),
				Err(err) => last_err = Some(err),
			}
		}

		Err(last_err.map_or_else(|| Error::InvalidSessionEncoding, Error::Encryption))
	}
}

fn aes_encoder_from_hex(key: &str) -> anyhow::Result<aes::Encoder> {
	let key = hex::decode(key)?;
	validate_key_length(&key)?;
	Ok(aes::Encoder::new(key.as_ref())?)
}

fn validate_hex_key(key: &str) -> anyhow::Result<()> {
	let key = hex::decode(key)?;
	validate_key_length(&key)
}

fn validate_key_length(key: &[u8]) -> anyhow::Result<()> {
	if key.len() != 32 {
		anyhow::bail!(
			"invalid AES-256-GCM key length: expected 32 bytes (64 hex characters), got {} bytes ({} hex characters)",
			key.len(),
			key.len() * 2,
		);
	}
	Ok(())
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn session_keyring_encodes_with_primary_and_decodes_with_previous() {
		let old_primary = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
		let new_primary = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

		let old_encoder = Encoder::aes(old_primary).expect("old encoder");
		let keyring = SessionKeyring {
			version: SESSION_KEYRING_VERSION.to_string(),
			primary: new_primary.to_string(),
			previous: vec![old_primary.to_string()],
			rotated_at: Some("2026-03-08T00:00:00Z".to_string()),
		};
		let new_encoder = Encoder::session_keyring(&keyring).expect("keyring encoder");

		let previous_session = SessionState::HTTP(HTTPSessionState {
			backend: "127.0.0.1:8080".parse().expect("socket addr"),
		});
		let previous_token = previous_session
			.encode(&old_encoder)
			.expect("encode old session");
		assert!(SessionState::decode(&previous_token, &new_encoder).is_ok());

		let rotated_token = previous_session
			.encode(&new_encoder)
			.expect("encode rotated session");
		assert!(SessionState::decode(&rotated_token, &new_encoder).is_ok());
		assert!(SessionState::decode(&rotated_token, &old_encoder).is_err());
	}

	#[test]
	fn session_keyring_invalid_entries_fail_closed() {
		let err = SessionKeyring::parse(
			r#"{"version":"v1","primary":"00112233445566778899aabbccddeeff00112233445566778899aabbccddee","previous":[]}"#,
		)
		.expect_err("invalid keyring must fail");
		assert!(err.to_string().contains("invalid AES-256-GCM key length"));
	}
}
