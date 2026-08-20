use ed25519_dalek::{Signature, Verifier, VerifyingKey};
#[cfg(test)]
use ed25519_dalek::{Signer, SigningKey};
#[cfg(test)]
use rand::Rng;

use crate::http::aauth::sig::encoding::base64url_decode;
#[cfg(test)]
use crate::http::aauth::sig::encoding::base64url_encode;
use crate::http::aauth::sig::errors::Error;

#[cfg(test)]
pub type PrivateKey = SigningKey;
pub type PublicKey = VerifyingKey;

/// Ed25519 public/private key length in bytes (RFC 8032).
const ED25519_KEY_LEN: usize = 32;
/// Ed25519 signature length in bytes (RFC 8032).
const ED25519_SIG_LEN: usize = 64;

/// Generate a new Ed25519 keypair.
#[cfg(test)]
pub fn generate_keypair() -> (PrivateKey, PublicKey) {
	let mut secret_bytes = [0u8; ED25519_KEY_LEN];
	rand::rng().fill_bytes(&mut secret_bytes);
	let signing_key = SigningKey::from_bytes(&secret_bytes);
	let verifying_key = signing_key.verifying_key();
	(signing_key, verifying_key)
}

/// Sign `data` with `private_key` (Ed25519/EdDSA, RFC 8032).
#[cfg(test)]
pub fn sign(data: &[u8], private_key: &PrivateKey) -> Vec<u8> {
	private_key.sign(data).to_bytes().to_vec()
}

/// Verify an Ed25519 signature over `data`. Returns `false` for any malformed input or signature
/// mismatch — callers must NOT treat verification errors as proof of validity.
pub fn verify(data: &[u8], signature: &[u8], public_key: &PublicKey) -> bool {
	let Ok(sig_bytes) = <[u8; ED25519_SIG_LEN]>::try_from(signature) else {
		return false;
	};
	let sig = Signature::from_bytes(&sig_bytes);
	public_key.verify(data, &sig).is_ok()
}

/// Decode a base64url-encoded Ed25519 public key (32 bytes).
pub fn public_key_from_bytes(bytes: &str) -> Result<PublicKey, Error> {
	let decoded = base64url_decode(bytes)?;
	let key_bytes: [u8; ED25519_KEY_LEN] = decoded
		.try_into()
		.map_err(|v: Vec<u8>| Error::InvalidKey(format!("invalid key length: {}", v.len())))?;
	VerifyingKey::from_bytes(&key_bytes)
		.map_err(|e| Error::InvalidKey(format!("invalid public key: {}", e)))
}

/// Encode an Ed25519 public key as base64url without padding.
#[cfg(test)]
pub fn public_key_to_base64url(key: &PublicKey) -> String {
	base64url_encode(key.as_bytes())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_generate_sign_verify_round_trip() {
		let (private_key, public_key) = generate_keypair();
		let data = b"hello world";
		let signature = sign(data, &private_key);
		assert!(verify(data, &signature, &public_key));
		// Different data must not verify against the same signature.
		assert!(!verify(b"different", &signature, &public_key));
	}

	#[test]
	fn test_verify_rejects_malformed_signature() {
		let (_, public_key) = generate_keypair();
		assert!(!verify(b"data", &[0u8; 63], &public_key));
		assert!(!verify(b"data", &[0u8; 65], &public_key));
	}

	#[test]
	fn test_public_key_encoding_round_trip() {
		let (_, public_key) = generate_keypair();
		let encoded = public_key_to_base64url(&public_key);
		let decoded = public_key_from_bytes(&encoded).unwrap();
		assert_eq!(public_key.as_bytes(), decoded.as_bytes());
	}

	#[test]
	fn test_public_key_rejects_wrong_length() {
		// 31-byte input
		let too_short = base64url_encode(&[0u8; 31]);
		assert!(public_key_from_bytes(&too_short).is_err());
	}
}
