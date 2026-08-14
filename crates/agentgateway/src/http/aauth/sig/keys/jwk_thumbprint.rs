use crate::crypto::digest::sha256;
use crate::http::aauth::sig::encoding::base64url_encode;
use crate::http::aauth::sig::errors::Error;
use crate::http::aauth::sig::keys::jwk::Jwk;

/// Calculate the Jwk Thumbprint per RFC 7638.
///
/// Algorithm:
/// 1. Build canonical JSON containing ONLY the required members for the key type, sorted
///    alphabetically (handled by [`Jwk::canonical_json`]).
/// 2. SHA-256 hash the canonical JSON bytes.
/// 3. Base64URL-encode the hash WITHOUT padding.
pub fn calculate_jwk_thumbprint(jwk: &Jwk) -> Result<String, Error> {
	let canonical = jwk.canonical_json()?;
	Ok(base64url_encode(&sha256(canonical.as_bytes())))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_jwk_thumbprint_ed25519() {
		let json = r#"{"kty":"OKP","crv":"Ed25519","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"}"#;
		let jwk = Jwk::parse(json).unwrap();
		let thumbprint = calculate_jwk_thumbprint(&jwk).unwrap();
		assert_eq!(thumbprint, "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U");
	}
}
