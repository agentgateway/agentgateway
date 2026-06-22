//! Agent token (`aa-agent+jwt`) validation per AAuth.
//!
//! Agent tokens are JWTs that:
//! - Have `typ=aa-agent+jwt` in the header
//! - Are signed by the agent server (issuer)
//! - Contain a `cnf.jwk` claim carrying the public key for HTTP signature verification
//! - `iss` identifies the agent server (HTTPS URL, host-only)
//! - `sub` (REQUIRED) is the stable agent identifier across key rotations
//! - `dwk` (REQUIRED) MUST be "aauth-agent.json" — names the discovery document
//! - `jti` is captured when present (the draft lists it as REQUIRED for replay detection, but
//!   agentgateway does not maintain a replay cache; reference implementations also omit it)

use http_message_sig::keys::jwk::JWK;
use serde_json::{Map, Value};

use crate::errors::AAuthError;
use crate::tokens::validation::{
	decode_jwt_claims_unverified, decode_jwt_header, extract_cnf_jwk, get_string_claim,
	is_acceptable_jwt_issuer_url, validate_jwt,
};

/// The `typ` header value required for AAuth agent tokens.
pub const AGENT_TOKEN_TYP: &str = "aa-agent+jwt";

/// The required `dwk` claim value for agent tokens.
pub const AGENT_TOKEN_DWK: &str = "aauth-agent.json";

/// Result of validating an `aa-agent+jwt` token.
#[derive(Debug, Clone)]
pub struct AgentTokenResult {
	/// The agent server URL (`iss` claim).
	pub agent_id: String,
	/// The stable agent identifier (`sub` claim).
	pub subject: String,
	/// The discovery document name (`dwk` claim) — always `aauth-agent.json` for agent tokens.
	pub dwk: String,
	/// The unique token identifier (`jti` claim) — captured when present.
	///
	/// The AAuth draft lists `jti` as REQUIRED for replay detection, but agentgateway does not
	/// maintain a replay cache, and reference implementations omit it. We capture the value when
	/// present (so callers can use it for logging or external replay tracking) but do not reject
	/// tokens that lack one.
	pub jti: Option<String>,
	/// The `cnf.jwk` public key for HTTP signature verification.
	pub cnf_jwk: JWK,
	/// All claims from the token, for downstream policy use.
	pub claims: Map<String, Value>,
}

/// Validate an `aa-agent+jwt` token.
///
/// The caller is responsible for:
/// 1. Extracting the issuer ([`get_agent_token_issuer`]).
/// 2. Fetching the JWKS from `{iss}/.well-known/aauth-agent.json`.
/// 3. Selecting the correct key by `kid` from the JWT header.
///
/// When `expected_audience` is `Some`, the token's `aud` claim MUST be present and MUST contain
/// the expected value (matching auth-token semantics and OAuth/JWT best practice). Passing `None`
/// disables the audience check entirely.
pub fn validate_agent_token(
	jwt: &str,
	signing_jwk: &JWK,
	expected_audience: Option<&str>,
	allow_insecure_http_issuer: bool,
) -> Result<AgentTokenResult, AAuthError> {
	let header = decode_jwt_header(jwt)?;
	let typ = header.typ.as_deref().unwrap_or("");
	if typ != AGENT_TOKEN_TYP {
		return Err(AAuthError::JwtValidation(format!(
			"expected typ={}, got typ={}",
			AGENT_TOKEN_TYP, typ
		)));
	}

	let claims = validate_jwt(jwt, signing_jwk, None)?;

	if let Some(expected) = expected_audience {
		let aud_val = claims.get("aud").ok_or(AAuthError::AudienceMismatch)?;
		let has_audience = match aud_val {
			Value::String(s) => s == expected,
			Value::Array(arr) => arr.iter().filter_map(Value::as_str).any(|s| s == expected),
			_ => false,
		};
		if !has_audience {
			return Err(AAuthError::AudienceMismatch);
		}
	}

	let agent_id =
		get_string_claim(&claims, "iss").ok_or_else(|| AAuthError::MissingClaim("iss".to_string()))?;
	if !is_acceptable_jwt_issuer_url(&agent_id, allow_insecure_http_issuer) {
		return Err(AAuthError::InvalidIssuerUrl);
	}

	let subject =
		get_string_claim(&claims, "sub").ok_or_else(|| AAuthError::MissingClaim("sub".to_string()))?;

	let dwk =
		get_string_claim(&claims, "dwk").ok_or_else(|| AAuthError::MissingClaim("dwk".to_string()))?;
	if dwk != AGENT_TOKEN_DWK {
		return Err(AAuthError::JwtValidation(format!(
			"agent token dwk must be {:?}, got {:?}",
			AGENT_TOKEN_DWK, dwk
		)));
	}

	let jti = get_string_claim(&claims, "jti");

	let cnf_jwk = extract_cnf_jwk(&claims)?;

	Ok(AgentTokenResult {
		agent_id,
		subject,
		dwk,
		jti,
		cnf_jwk,
		claims,
	})
}

/// Read `iss` from an agent token WITHOUT validating the signature.
///
/// Use only to determine which JWKS to fetch — do not trust the returned value for anything else.
pub fn get_agent_token_issuer(jwt: &str) -> Result<String, AAuthError> {
	let claims = decode_jwt_claims_unverified(jwt)?;
	get_string_claim(&claims, "iss")
		.ok_or_else(|| AAuthError::JwtValidation("missing iss claim".to_string()))
}

/// Read the `kid` from an agent token header.
pub fn get_agent_token_kid(jwt: &str) -> Result<Option<String>, AAuthError> {
	Ok(decode_jwt_header(jwt)?.kid)
}

/// Extract `cnf.jwk` from an agent token payload WITHOUT validating the signature.
pub fn extract_agent_token_key(jwt: &str) -> Result<JWK, AAuthError> {
	extract_cnf_jwk(&decode_jwt_claims_unverified(jwt)?)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_test_claims() -> Map<String, Value> {
		let mut claims = Map::new();
		claims.insert(
			"iss".to_string(),
			Value::String("https://agent.example.com".to_string()),
		);
		claims.insert(
			"sub".to_string(),
			Value::String("aauth:local@agent.example.com".to_string()),
		);
		claims.insert(
			"dwk".to_string(),
			Value::String(AGENT_TOKEN_DWK.to_string()),
		);
		claims.insert(
			"jti".to_string(),
			Value::String("unique-id-123".to_string()),
		);

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
		claims
	}

	#[test]
	fn test_extract_cnf_jwk_from_claims() {
		let claims = make_test_claims();
		let jwk = extract_cnf_jwk(&claims).unwrap();
		assert_eq!(jwk.kty, "OKP");
	}
}
