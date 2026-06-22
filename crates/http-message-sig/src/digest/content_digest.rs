use sha2::{Digest, Sha256, Sha512};

use crate::encoding::base64_encode;
use crate::errors::Error;

/// Content-Digest algorithms supported per RFC 9530.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
	Sha256,
	Sha512,
}

impl DigestAlgorithm {
	pub fn as_str(self) -> &'static str {
		match self {
			DigestAlgorithm::Sha256 => "sha-256",
			DigestAlgorithm::Sha512 => "sha-512",
		}
	}
}

impl std::str::FromStr for DigestAlgorithm {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"sha-256" => Ok(DigestAlgorithm::Sha256),
			"sha-512" => Ok(DigestAlgorithm::Sha512),
			other => Err(Error::UnsupportedAlgorithm(other.to_string())),
		}
	}
}

/// Calculate the Content-Digest header value for `body` per RFC 9530.
///
/// Format: `{algorithm}=:{base64}:`. Example: `sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:`.
pub fn calculate_content_digest(body: &[u8], algorithm: DigestAlgorithm) -> String {
	let hash_bytes: Vec<u8> = match algorithm {
		DigestAlgorithm::Sha256 => {
			let mut hasher = Sha256::new();
			hasher.update(body);
			hasher.finalize().to_vec()
		},
		DigestAlgorithm::Sha512 => {
			let mut hasher = Sha512::new();
			hasher.update(body);
			hasher.finalize().to_vec()
		},
	};

	format!("{}=:{}:", algorithm.as_str(), base64_encode(&hash_bytes))
}

/// Extract the first algorithm name from a Content-Digest header value.
///
/// The header is a Structured Field Dictionary per RFC 8941 (e.g. `sha-256=:...:, sha-512=:...:`).
/// We only need the algorithm name to recompute the digest for comparison; the full byte sequence
/// stays in the header string.
pub fn parse_content_digest_header(header: &str) -> Result<DigestAlgorithm, Error> {
	use std::str::FromStr;
	let first_entry = header
		.split(',')
		.next()
		.ok_or_else(|| Error::InvalidHeader("empty content-digest header".to_string()))?
		.trim();
	let alg = first_entry
		.split('=')
		.next()
		.ok_or_else(|| Error::InvalidHeader("content-digest header missing algorithm".to_string()))?
		.trim()
		.to_lowercase();
	DigestAlgorithm::from_str(&alg)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_content_digest_sha256() {
		let body = br#"{"hello": "world"}"#;
		let digest = calculate_content_digest(body, DigestAlgorithm::Sha256);
		assert_eq!(
			digest,
			"sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:"
		);
	}

	#[test]
	fn test_content_digest_sha512() {
		let body = br#"{"hello": "world"}"#;
		let digest = calculate_content_digest(body, DigestAlgorithm::Sha512);
		assert_eq!(
			digest,
			"sha-512=:WZDPaVn/7XgHaAy8pmojAkGWoRx2UFChF41A2svX+TaPm+AbwAgBWnrIiYllu7BNNyealdVLvRwEmTHWXvJwew==:"
		);
	}

	#[test]
	fn test_parse_content_digest_header() {
		assert_eq!(
			parse_content_digest_header("sha-256=:abc:").unwrap(),
			DigestAlgorithm::Sha256
		);
		assert_eq!(
			parse_content_digest_header(" SHA-512=:abc:").unwrap(),
			DigestAlgorithm::Sha512
		);
		assert!(parse_content_digest_header("md5=:abc:").is_err());
	}
}
