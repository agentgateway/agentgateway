use base64::Engine;
use base64::engine::general_purpose;

use crate::errors::Error;

/// Standard Base64 encoding (RFC 4648) with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
	general_purpose::STANDARD.encode(bytes)
}

/// Standard Base64 decoding (RFC 4648) with padding.
pub fn base64_decode(s: &str) -> Result<Vec<u8>, Error> {
	general_purpose::STANDARD.decode(s).map_err(Error::from)
}

/// Base64URL encoding without padding (for JWK values).
pub fn base64url_encode(bytes: &[u8]) -> String {
	general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Base64URL decoding (handles missing padding).
pub fn base64url_decode(s: &str) -> Result<Vec<u8>, Error> {
	general_purpose::URL_SAFE_NO_PAD
		.decode(s)
		.map_err(Error::from)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_base64_round_trip() {
		let input = b"hello world";
		assert_eq!(base64_encode(input), "aGVsbG8gd29ybGQ=");
		assert_eq!(base64_decode("aGVsbG8gd29ybGQ=").unwrap(), input);
	}

	#[test]
	fn test_base64url_round_trip() {
		let input = b"hello world";
		assert_eq!(base64url_encode(input), "aGVsbG8gd29ybGQ");
		assert_eq!(base64url_decode("aGVsbG8gd29ybGQ").unwrap(), input);
	}
}
