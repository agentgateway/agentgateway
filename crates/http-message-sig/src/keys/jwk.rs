use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Error;
use crate::keys::ed25519::{PublicKey, public_key_from_bytes};

/// JSON Web Key (RFC 7517).
///
/// Only the fields we exercise are typed; the rest is preserved via `extra`. AAuth itself only
/// uses OKP/Ed25519 keys for HTTP signatures, but other key types (RSA, EC) are accepted so the
/// type can also represent issuer-signing keys retrieved from a JWKS document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JWK {
	pub kty: String, // "OKP", "EC", "RSA"
	#[serde(skip_serializing_if = "Option::is_none")]
	pub crv: Option<String>, // "Ed25519", "P-256", etc.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub x: Option<String>, // Public key X coordinate (base64url)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub y: Option<String>, // EC Y coordinate (base64url)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub d: Option<String>, // Private key (base64url) — never serialized in public contexts
	#[serde(skip_serializing_if = "Option::is_none")]
	pub n: Option<String>, // RSA modulus (base64url)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub e: Option<String>, // RSA exponent (base64url)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub kid: Option<String>, // Key ID
	#[serde(skip_serializing_if = "Option::is_none")]
	pub alg: Option<String>, // Algorithm
	#[serde(flatten)]
	pub extra: serde_json::Map<String, Value>,
}

impl JWK {
	pub fn parse(json: &str) -> Result<Self, Error> {
		serde_json::from_str(json).map_err(Error::from)
	}

	pub fn serialize(&self) -> Result<String, Error> {
		serde_json::to_string(self).map_err(Error::from)
	}

	/// Build canonical JSON for thumbprint per RFC 7638: only the REQUIRED members for the key
	/// type, lexicographically sorted, with no whitespace.
	pub fn canonical_json(&self) -> Result<String, Error> {
		let mut map = serde_json::Map::new();

		match self.kty.as_str() {
			"OKP" => {
				map.insert(
					"crv".to_string(),
					Value::String(
						self
							.crv
							.clone()
							.ok_or_else(|| Error::InvalidKey("OKP missing crv".to_string()))?,
					),
				);
				map.insert("kty".to_string(), Value::String(self.kty.clone()));
				map.insert(
					"x".to_string(),
					Value::String(
						self
							.x
							.clone()
							.ok_or_else(|| Error::InvalidKey("OKP missing x".to_string()))?,
					),
				);
			},
			"EC" => {
				map.insert(
					"crv".to_string(),
					Value::String(
						self
							.crv
							.clone()
							.ok_or_else(|| Error::InvalidKey("EC missing crv".to_string()))?,
					),
				);
				map.insert("kty".to_string(), Value::String(self.kty.clone()));
				map.insert(
					"x".to_string(),
					Value::String(
						self
							.x
							.clone()
							.ok_or_else(|| Error::InvalidKey("EC missing x".to_string()))?,
					),
				);
				map.insert(
					"y".to_string(),
					Value::String(
						self
							.y
							.clone()
							.ok_or_else(|| Error::InvalidKey("EC missing y".to_string()))?,
					),
				);
			},
			"RSA" => {
				map.insert(
					"e".to_string(),
					Value::String(
						self
							.e
							.clone()
							.ok_or_else(|| Error::InvalidKey("RSA missing e".to_string()))?,
					),
				);
				map.insert("kty".to_string(), Value::String(self.kty.clone()));
				map.insert(
					"n".to_string(),
					Value::String(
						self
							.n
							.clone()
							.ok_or_else(|| Error::InvalidKey("RSA missing n".to_string()))?,
					),
				);
			},
			_ => return Err(Error::InvalidKey(format!("unsupported kty: {}", self.kty))),
		}

		serde_json::to_string(&Value::Object(map)).map_err(Error::from)
	}

	/// Convert an OKP/Ed25519 JWK to an Ed25519 PublicKey. Rejects other key types.
	pub fn to_ed25519_public_key(&self) -> Result<PublicKey, Error> {
		if self.kty != "OKP" {
			return Err(Error::InvalidKey(format!(
				"expected kty=OKP, got {}",
				self.kty
			)));
		}
		let crv = self
			.crv
			.as_deref()
			.ok_or_else(|| Error::InvalidKey("OKP missing crv".to_string()))?;
		if crv != "Ed25519" {
			return Err(Error::InvalidKey(format!(
				"expected crv=Ed25519, got {}",
				crv
			)));
		}
		let x = self
			.x
			.as_deref()
			.ok_or_else(|| Error::InvalidKey("OKP missing x".to_string()))?;
		public_key_from_bytes(x)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_jwk_parse_okp() {
		let json = r#"{"kty":"OKP","crv":"Ed25519","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"}"#;
		let jwk = JWK::parse(json).unwrap();
		assert_eq!(jwk.kty, "OKP");
		assert_eq!(jwk.crv.as_deref(), Some("Ed25519"));
		assert_eq!(
			jwk.x.as_deref(),
			Some("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs")
		);
	}

	#[test]
	fn test_jwk_canonical_okp_strips_optional_fields() {
		let json = r#"{"kty":"OKP","crv":"Ed25519","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs","kid":"test"}"#;
		let jwk = JWK::parse(json).unwrap();
		assert_eq!(
			jwk.canonical_json().unwrap(),
			r#"{"crv":"Ed25519","kty":"OKP","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"}"#
		);
	}

	#[test]
	fn test_to_ed25519_public_key_rejects_wrong_kty() {
		let json = r#"{"kty":"EC","crv":"P-256","x":"a","y":"b"}"#;
		let jwk = JWK::parse(json).unwrap();
		assert!(jwk.to_ed25519_public_key().is_err());
	}

	#[test]
	fn test_to_ed25519_public_key_rejects_wrong_curve() {
		let json = r#"{"kty":"OKP","crv":"X25519","x":"AAAA"}"#;
		let jwk = JWK::parse(json).unwrap();
		assert!(jwk.to_ed25519_public_key().is_err());
	}
}
