//! JWT validation utilities shared by agent-token and auth-token validation.

use http_message_sig::keys::jwk::JWK;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::errors::AAuthError;

/// Whether `iss` is acceptable as an AAuth server identifier.
///
/// Per AAuth, server identifiers MUST be HTTPS URLs containing only scheme+host: no path, query,
/// or fragment, and the host must be lowercase.
///
/// When `allow_insecure_http` is true, `http://` issuers are also accepted, but the host MUST
/// be a loopback address (`localhost`, `127.0.0.0/8`, `::1`). A port IS allowed under the http
/// branch because dev servers commonly bind to non-default ports. Production deployments MUST
/// leave the flag disabled.
pub(crate) fn is_acceptable_jwt_issuer_url(iss: &str, allow_insecure_http: bool) -> bool {
	// Reject any uppercase ASCII in the raw issuer string. The url crate normalizes the host on
	// parse, so we have to check the original input — otherwise `https://Agent.Example` would
	// silently succeed.
	if iss.chars().any(|c| c.is_ascii_uppercase()) {
		return false;
	}

	let Ok(parsed) = url::Url::parse(iss) else {
		return false;
	};
	let scheme = parsed.scheme();
	let is_https = scheme == "https";
	let is_http = scheme == "http";

	if !(is_https || allow_insecure_http && is_http) {
		return false;
	}

	// host required for both branches
	if parsed.host_str().is_none() {
		return false;
	}
	// path must be empty or "/"
	let path = parsed.path();
	if !path.is_empty() && path != "/" {
		return false;
	}
	// no query or fragment in either branch
	if parsed.query().is_some() || parsed.fragment().is_some() {
		return false;
	}
	// HTTPS issuers may not carry a port (production must use 443); http (dev) issuers may.
	if is_https && parsed.port().is_some() {
		return false;
	}
	// http (dev) is loopback-only — the flag is documented as enabling local development,
	// so an attacker (or misconfiguration) can't point it at an external host.
	if is_http && !is_loopback_host(&parsed) {
		return false;
	}

	true
}

/// Whether a parsed URL's host is a loopback address: `localhost`, an IPv4 address in
/// 127.0.0.0/8, or the IPv6 address `::1`.
fn is_loopback_host(parsed: &url::Url) -> bool {
	match parsed.host() {
		Some(url::Host::Domain(d)) => d == "localhost",
		Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
		Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
		None => false,
	}
}

/// The `cnf` (confirmation) claim carrying the proof-of-possession JWK (RFC 7800).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CnfClaim {
	pub jwk: JWK,
}

/// Result of validating an AAuth JWT.
#[derive(Debug, Clone)]
pub struct JwtValidationResult {
	pub issuer: String,
	pub subject: Option<String>,
	pub agent_id: Option<String>,
	pub agent_delegate: Option<String>,
	pub scopes: Option<Vec<String>>,
	pub cnf_jwk: JWK,
	pub claims: Map<String, Value>,
}

/// Decode a JWT header without validating the signature.
pub fn decode_jwt_header(jwt: &str) -> Result<jsonwebtoken::Header, AAuthError> {
	decode_header(jwt).map_err(|e| AAuthError::JwtValidation(format!("invalid JWT header: {}", e)))
}

/// Decode JWT claims WITHOUT validating the signature.
///
/// Use this ONLY to extract claims needed for key discovery (e.g. `iss` to know which JWKS to
/// fetch). Callers MUST re-validate via [`validate_jwt`] before trusting any claims.
pub fn decode_jwt_claims_unverified(jwt: &str) -> Result<Map<String, Value>, AAuthError> {
	let parts: Vec<&str> = jwt.split('.').collect();
	if parts.len() != 3 {
		return Err(AAuthError::JwtValidation("invalid JWT format".to_string()));
	}
	let payload_bytes =
		base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
			.map_err(|e| AAuthError::JwtValidation(format!("invalid JWT payload encoding: {}", e)))?;
	serde_json::from_slice(&payload_bytes)
		.map_err(|e| AAuthError::JwtValidation(format!("invalid JWT payload JSON: {}", e)))
}

/// Validate a JWT signature using a JWK from the issuer's JWKS.
///
/// Supports:
/// - OKP/Ed25519 (EdDSA) — used by AAuth agent tokens
/// - RSA (RS256/RS384/RS512) — typical for OAuth/OIDC auth servers
/// - EC (ES256/ES384)
///
/// `exp` and `iat` are required per the AAuth spec. Audience validation is left to the caller
/// because AAuth resource servers need to compare against their own derived audience.
pub fn validate_jwt(
	jwt: &str,
	signing_jwk: &JWK,
	expected_typ: Option<&str>,
) -> Result<Map<String, Value>, AAuthError> {
	if let Some(expected) = expected_typ {
		let header = decode_jwt_header(jwt)?;
		let typ = header.typ.as_deref().unwrap_or("");
		if typ != expected {
			return Err(AAuthError::JwtValidation(format!(
				"expected typ={}, got typ={}",
				expected, typ
			)));
		}
	}

	let (decoding_key, algorithms) = match signing_jwk.kty.as_str() {
		"OKP" => {
			let crv = signing_jwk.crv.as_deref().unwrap_or("");
			if crv != "Ed25519" {
				return Err(AAuthError::JwtValidation(format!(
					"unsupported OKP curve: {}",
					crv
				)));
			}
			let x = signing_jwk
				.x
				.as_deref()
				.ok_or_else(|| AAuthError::JwtValidation("OKP JWK missing x parameter".to_string()))?;
			let key = DecodingKey::from_ed_components(x)
				.map_err(|e| AAuthError::JwtValidation(format!("invalid Ed25519 key: {}", e)))?;
			(key, vec![Algorithm::EdDSA])
		},
		"RSA" => {
			let n = signing_jwk
				.n
				.as_deref()
				.ok_or_else(|| AAuthError::JwtValidation("RSA JWK missing n parameter".to_string()))?;
			let e = signing_jwk
				.e
				.as_deref()
				.ok_or_else(|| AAuthError::JwtValidation("RSA JWK missing e parameter".to_string()))?;
			let key = DecodingKey::from_rsa_components(n, e)
				.map_err(|err| AAuthError::JwtValidation(format!("invalid RSA key: {}", err)))?;
			(
				key,
				vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
			)
		},
		"EC" => {
			let x = signing_jwk
				.x
				.as_deref()
				.ok_or_else(|| AAuthError::JwtValidation("EC JWK missing x parameter".to_string()))?;
			let y = signing_jwk
				.y
				.as_deref()
				.ok_or_else(|| AAuthError::JwtValidation("EC JWK missing y parameter".to_string()))?;
			let key = DecodingKey::from_ec_components(x, y)
				.map_err(|err| AAuthError::JwtValidation(format!("invalid EC key: {}", err)))?;
			(key, vec![Algorithm::ES256, Algorithm::ES384])
		},
		other => {
			return Err(AAuthError::JwtValidation(format!(
				"unsupported key type: {}",
				other
			)));
		},
	};

	let mut validation = Validation::new(algorithms[0]);
	validation.algorithms = algorithms;
	// Audience is validated by the caller (resource server) against its derived audience.
	validation.validate_aud = false;
	validation.set_required_spec_claims(&["exp", "iat"]);

	let token_data = decode::<Map<String, Value>>(jwt, &decoding_key, &validation)
		.map_err(|e| AAuthError::JwtValidation(format!("JWT validation failed: {}", e)))?;
	Ok(token_data.claims)
}

/// Extract the `cnf.jwk` claim from a JWT payload (RFC 7800).
pub fn extract_cnf_jwk(claims: &Map<String, Value>) -> Result<JWK, AAuthError> {
	let cnf = claims
		.get("cnf")
		.ok_or_else(|| AAuthError::JwtValidation("missing cnf claim".to_string()))?;
	let cnf_obj = cnf
		.as_object()
		.ok_or_else(|| AAuthError::JwtValidation("cnf claim is not an object".to_string()))?;
	let jwk_value = cnf_obj
		.get("jwk")
		.ok_or_else(|| AAuthError::JwtValidation("missing cnf.jwk claim".to_string()))?;
	serde_json::from_value(jwk_value.clone())
		.map_err(|e| AAuthError::JwtValidation(format!("invalid cnf.jwk: {}", e)))
}

/// Extract a string-typed claim by name.
pub fn get_string_claim(claims: &Map<String, Value>, name: &str) -> Option<String> {
	claims.get(name).and_then(Value::as_str).map(str::to_string)
}

/// Extract scopes from the space-separated `scope` claim.
///
/// Returns `None` if the claim is missing, non-string, or yields no scopes once whitespace is
/// stripped — callers can then trust an empty/whitespace-only `scope: ""` to behave the same as
/// an omitted claim, instead of producing `Some(vec![])` that silently passes "must have at least
/// one of sub or scope" checks.
pub fn get_scopes(claims: &Map<String, Value>) -> Option<Vec<String>> {
	let scopes: Vec<String> = claims
		.get("scope")
		.and_then(Value::as_str)?
		.split_whitespace()
		.map(str::to_string)
		.collect();
	if scopes.is_empty() {
		None
	} else {
		Some(scopes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_is_acceptable_jwt_issuer_url_https_strict() {
		assert!(is_acceptable_jwt_issuer_url(
			"https://agent.example.com",
			false
		));
		assert!(is_acceptable_jwt_issuer_url(
			"https://agent.example.com/",
			false
		));
		assert!(!is_acceptable_jwt_issuer_url(
			"https://agent.example.com:8443",
			false
		));
		assert!(!is_acceptable_jwt_issuer_url(
			"https://agent.example.com/path",
			false
		));
		assert!(!is_acceptable_jwt_issuer_url(
			"https://agent.example.com/?q",
			false
		));
		assert!(!is_acceptable_jwt_issuer_url(
			"https://Agent.Example.com",
			false
		));
		assert!(!is_acceptable_jwt_issuer_url(
			"http://agent.example.com",
			false
		));
	}

	#[test]
	fn test_is_acceptable_jwt_issuer_url_http_dev() {
		// Allowed: dev http restricted to loopback hosts (localhost, 127.x, ::1) with optional port.
		assert!(is_acceptable_jwt_issuer_url("http://localhost", true));
		assert!(is_acceptable_jwt_issuer_url("http://localhost:8080", true));
		assert!(is_acceptable_jwt_issuer_url("http://127.0.0.1", true));
		assert!(is_acceptable_jwt_issuer_url("http://127.0.0.1:9099", true));
		assert!(is_acceptable_jwt_issuer_url("http://127.5.6.7", true)); // any 127.0.0.0/8
		assert!(is_acceptable_jwt_issuer_url("http://[::1]", true));
		assert!(is_acceptable_jwt_issuer_url("http://[::1]:8080", true));
		// Rejected: same host-only constraints as https — no path, query, or fragment.
		assert!(!is_acceptable_jwt_issuer_url(
			"http://localhost/issuer-a",
			true
		));
		assert!(!is_acceptable_jwt_issuer_url("http://localhost?x=1", true));
		assert!(!is_acceptable_jwt_issuer_url("http://localhost#frag", true));
		// Rejected: dev flag does NOT enable plaintext to external hosts.
		assert!(!is_acceptable_jwt_issuer_url("http://attacker.com", true));
		assert!(!is_acceptable_jwt_issuer_url("http://192.168.1.1", true));
		assert!(!is_acceptable_jwt_issuer_url("http://10.0.0.5:8080", true));
		// Rejected: still not a URL.
		assert!(!is_acceptable_jwt_issuer_url("not-a-url", true));
		// Rejected: http still off by default.
		assert!(!is_acceptable_jwt_issuer_url("http://localhost", false));
	}

	#[test]
	fn test_get_scopes_empty_becomes_none() {
		// Critical: an empty or whitespace-only `scope` claim must NOT produce Some(vec![]) —
		// that would bypass the "must have at least one of sub or scope" guard in
		// validate_auth_token by passing a non-None `scopes` value.
		let mut claims = Map::new();
		claims.insert("scope".to_string(), Value::String(String::new()));
		assert!(get_scopes(&claims).is_none());
		claims.insert("scope".to_string(), Value::String("   \t  ".to_string()));
		assert!(get_scopes(&claims).is_none());
	}

	#[test]
	fn test_extract_cnf_jwk() {
		let mut claims = Map::new();
		let mut cnf = serde_json::Map::new();
		cnf.insert(
			"jwk".to_string(),
			serde_json::json!({
				"kty": "OKP",
				"crv": "Ed25519",
				"x": "JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs",
			}),
		);
		claims.insert("cnf".to_string(), Value::Object(cnf));

		let jwk = extract_cnf_jwk(&claims).unwrap();
		assert_eq!(jwk.kty, "OKP");
		assert_eq!(jwk.crv.as_deref(), Some("Ed25519"));
	}

	#[test]
	fn test_extract_cnf_jwk_missing() {
		assert!(extract_cnf_jwk(&Map::new()).is_err());
	}

	#[test]
	fn test_get_scopes() {
		let mut claims = Map::new();
		claims.insert(
			"scope".to_string(),
			Value::String("read write admin".to_string()),
		);
		assert_eq!(get_scopes(&claims).unwrap(), vec!["read", "write", "admin"]);
	}
}
