use super::state::{HandshakeState, SessionState};
use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct RuntimeCookieSecret([u8; 32]);

impl RuntimeCookieSecret {
	pub(crate) fn as_bytes(&self) -> &[u8; 32] {
		&self.0
	}
}

pub(crate) fn parse_runtime_cookie_secret(raw: &str) -> anyhow::Result<RuntimeCookieSecret> {
	let decoded = hex::decode(raw).map_err(|e| anyhow::anyhow!("invalid hex secret: {e}"))?;
	if decoded.len() != 32 {
		anyhow::bail!(
			"invalid AES-256-GCM key length: expected 32 bytes (64 hex characters), got {} bytes ({} hex characters)",
			decoded.len(),
			decoded.len() * 2,
		);
	}
	let mut key = [0u8; 32];
	key.copy_from_slice(&decoded);
	Ok(RuntimeCookieSecret(key))
}

#[derive(Debug)]
pub(super) struct SessionCodec {
	key: LessSafeKey,
	aad: &'static [u8],
}

impl SessionCodec {
	pub(super) fn new(key_bytes: &[u8], aad: &'static [u8]) -> anyhow::Result<Self> {
		let unbound =
			UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| anyhow::anyhow!("invalid key"))?;
		Ok(Self {
			key: LessSafeKey::new(unbound),
			aad,
		})
	}

	pub(super) fn encrypt_handshake_state(&self, state: &HandshakeState) -> anyhow::Result<String> {
		let json = serde_json::to_vec(state)?;
		self.encrypt(&json)
	}

	pub(super) fn decrypt_handshake_state(&self, encoded: &str) -> anyhow::Result<HandshakeState> {
		let json = self.decrypt(encoded)?;
		Ok(serde_json::from_slice(&json)?)
	}

	pub(super) fn encode_session(&self, session: &SessionState) -> anyhow::Result<String> {
		let json = serde_json::to_vec(session)?;
		self.encrypt(&json)
	}

	pub(super) fn decode_session(&self, encoded: &str) -> anyhow::Result<SessionState> {
		let json = self.decrypt(encoded)?;
		Ok(serde_json::from_slice(&json)?)
	}

	fn encrypt(&self, plaintext: &[u8]) -> anyhow::Result<String> {
		let mut nonce_bytes = [0u8; 12];
		let mut rng = rand::rng();
		rng.fill_bytes(&mut nonce_bytes);
		let nonce = Nonce::assume_unique_for_key(nonce_bytes);

		let mut in_out = Vec::with_capacity(plaintext.len() + AES_256_GCM.tag_len());
		in_out.extend_from_slice(plaintext);

		self
			.key
			.seal_in_place_append_tag(nonce, Aad::from(self.aad), &mut in_out)
			.map_err(|_| anyhow::anyhow!("encryption failed"))?;

		let mut result = Vec::with_capacity(12 + in_out.len());
		result.extend_from_slice(&nonce_bytes);
		result.extend_from_slice(&in_out);

		Ok(URL_SAFE_NO_PAD.encode(result))
	}

	fn decrypt(&self, encoded: &str) -> anyhow::Result<Vec<u8>> {
		let mut data = URL_SAFE_NO_PAD
			.decode(encoded)
			.map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;

		let tag_len = AES_256_GCM.tag_len();
		if data.len() < 12 + tag_len {
			anyhow::bail!("data too short");
		}

		let nonce = Nonce::try_assume_unique_for_key(&data[..12])
			.map_err(|_| anyhow::anyhow!("invalid nonce"))?;

		let plaintext_len = {
			let in_out = &mut data[12..];
			let plaintext = self
				.key
				.open_in_place(nonce, Aad::from(self.aad), in_out)
				.map_err(|_| anyhow::anyhow!("decryption failed"))?;
			plaintext.len()
		};

		data.copy_within(12..12 + plaintext_len, 0);
		data.truncate(plaintext_len);
		Ok(data)
	}
}
