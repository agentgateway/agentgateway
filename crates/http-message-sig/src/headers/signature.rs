use crate::encoding::{base64_decode, base64_encode, base64url_decode};
use crate::errors::Error;

/// Parse a Signature header value: `label=:base64signature:`.
///
/// Returns the label and decoded signature bytes. Accepts both standard base64 (the RFC 9421
/// canonical form) and base64url. Some clients send the latter, so we try both.
pub fn parse_signature(header: &str) -> Result<(String, Vec<u8>), Error> {
	let (label, value) = header
		.split_once('=')
		.ok_or_else(|| Error::InvalidHeader(format!("invalid signature header: {}", header)))?;
	let label = label.trim().to_string();
	let value = value.trim();

	if !value.starts_with(':') || !value.ends_with(':') || value.len() < 2 {
		return Err(Error::InvalidHeader(format!(
			"signature value must be wrapped in colons: {}",
			value
		)));
	}
	let inner = &value[1..value.len() - 1];
	// RFC 9421 mandates standard base64; some clients use base64url. Try standard first.
	let signature_bytes = base64_decode(inner).or_else(|_| base64url_decode(inner))?;
	Ok((label, signature_bytes))
}

/// Build a Signature header value: `label=:base64signature:`.
pub fn build_signature(label: &str, signature: &[u8]) -> String {
	format!("{}=:{}:", label, base64_encode(signature))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_signature_ed25519() {
		let header = "sig1=:wqcAqbmYJ2ji2glfAMaRy4gruYYnx2nEFN2HN6jrnDnQCK1u02Gb04v9EDgwUPiu4A0w6vuQv5lIp5WPpBKRCw==:";
		let (label, sig_bytes) = parse_signature(header).unwrap();
		assert_eq!(label, "sig1");
		assert_eq!(sig_bytes.len(), 64);
	}

	#[test]
	fn test_build_signature_round_trip() {
		let sig_bytes = vec![0u8; 64];
		let header = build_signature("sig1", &sig_bytes);
		let (label, decoded) = parse_signature(&header).unwrap();
		assert_eq!(label, "sig1");
		assert_eq!(decoded, sig_bytes);
	}

	#[test]
	fn test_parse_signature_rejects_missing_colons() {
		assert!(parse_signature("sig1=abcd").is_err());
	}
}
