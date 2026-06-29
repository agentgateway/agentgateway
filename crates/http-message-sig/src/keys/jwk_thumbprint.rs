use sha2::{Digest, Sha256};

use crate::encoding::base64url_encode;
use crate::errors::Error;
use crate::keys::jwk::JWK;

/// Calculate the JWK Thumbprint per RFC 7638.
///
/// Algorithm:
/// 1. Build canonical JSON containing ONLY the required members for the key type, sorted
///    alphabetically (handled by [`JWK::canonical_json`]).
/// 2. SHA-256 hash the canonical JSON bytes.
/// 3. Base64URL-encode the hash WITHOUT padding.
pub fn calculate_jwk_thumbprint(jwk: &JWK) -> Result<String, Error> {
	let canonical = jwk.canonical_json()?;
	let mut hasher = Sha256::new();
	hasher.update(canonical.as_bytes());
	Ok(base64url_encode(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_jwk_thumbprint_ed25519() {
		let json = r#"{"kty":"OKP","crv":"Ed25519","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"}"#;
		let jwk = JWK::parse(json).unwrap();
		let thumbprint = calculate_jwk_thumbprint(&jwk).unwrap();
		assert_eq!(thumbprint, "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U");
	}
}
