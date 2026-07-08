//! Authenticated encryption (AEAD) primitives.
//!
//! This is the single seam through which agentgateway performs symmetric
//! authenticated encryption. It is currently backed by `aws-lc-rs`, which is
//! always linked regardless of the selected TLS provider feature. Additional
//! backends (e.g. SymCrypt) plug in here behind `#[cfg]` without changing call
//! sites.

use aws_lc_rs::aead::{AES_256_GCM, Aad, Nonce, RandomizedNonceKey};

/// Length in bytes of the AES-256-GCM nonce that [`Aes256Gcm::seal`] prepends to
/// each sealed message.
const NONCE_LEN: usize = 12;

/// AES-256-GCM authenticated encryption keyed with a caller-provided 32-byte key.
///
/// [`seal`](Aes256Gcm::seal) generates a fresh random nonce per message and
/// returns `nonce || ciphertext || tag`; [`open`](Aes256Gcm::open) expects that
/// same framing.
#[derive(Debug)]
pub struct Aes256Gcm {
	key: RandomizedNonceKey,
}

impl Aes256Gcm {
	/// Creates an AES-256-GCM key from 32 bytes of key material.
	pub fn new(key: &[u8]) -> Result<Self, AeadError> {
		let key = RandomizedNonceKey::new(&AES_256_GCM, key).map_err(|_| AeadError::InvalidKey)?;
		Ok(Self { key })
	}

	/// Seals `plaintext`, returning `nonce || ciphertext || tag`.
	pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, AeadError> {
		let mut in_out = plaintext.to_vec();
		// Generates a random nonce and appends the authentication tag in place.
		let nonce = self
			.key
			.seal_in_place_append_tag(Aad::empty(), &mut in_out)
			.map_err(|_| AeadError::EncryptionFailed)?;

		let mut result = nonce.as_ref().to_vec();
		result.extend_from_slice(&in_out);
		Ok(result)
	}

	/// Opens data framed as `nonce || ciphertext || tag`, returning the plaintext.
	pub fn open(&self, data: &[u8]) -> Result<Vec<u8>, AeadError> {
		if data.len() < NONCE_LEN {
			return Err(AeadError::InvalidFormat);
		}

		let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
		let nonce =
			Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| AeadError::InvalidFormat)?;
		let mut in_out = ciphertext.to_vec();
		let plaintext = self
			.key
			.open_in_place(nonce, Aad::empty(), &mut in_out)
			.map_err(|_| AeadError::DecryptionFailed)?;
		Ok(plaintext.to_vec())
	}
}

#[derive(Debug, thiserror::Error)]
pub enum AeadError {
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
	use super::{AeadError, Aes256Gcm};

	#[test]
	fn round_trip() {
		let key = Aes256Gcm::new(&[7u8; 32]).expect("key");
		let sealed = key.seal(b"hello world").expect("seal");
		assert_ne!(sealed, b"hello world");
		assert_eq!(key.open(&sealed).expect("open"), b"hello world");
	}

	#[test]
	fn short_input_fails_cleanly() {
		let key = Aes256Gcm::new(&[0u8; 32]).expect("key");
		assert!(matches!(key.open(&[0u8; 11]), Err(AeadError::InvalidFormat)));
	}

	#[test]
	fn tampered_ciphertext_fails() {
		let key = Aes256Gcm::new(&[1u8; 32]).expect("key");
		let mut sealed = key.seal(b"secret").expect("seal");
		let last = sealed.len() - 1;
		sealed[last] ^= 0xff;
		assert!(matches!(
			key.open(&sealed),
			Err(AeadError::DecryptionFailed)
		));
	}

	#[test]
	fn wrong_key_fails() {
		let sealed = Aes256Gcm::new(&[1u8; 32]).expect("key").seal(b"x").expect("seal");
		let other = Aes256Gcm::new(&[2u8; 32]).expect("key");
		assert!(matches!(other.open(&sealed), Err(AeadError::DecryptionFailed)));
	}
}
