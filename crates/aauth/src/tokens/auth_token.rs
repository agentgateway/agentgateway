//! Auth token (`aa-auth+jwt`) validation per AAuth.
//!
//! Auth tokens are JWTs that:
//! - Have `typ=aa-auth+jwt` in the header
//! - Are signed by an authorization server
//! - Contain a `cnf.jwk` claim carrying the public key for HTTP signature verification
//! - `iss` identifies the authorization server (HTTPS URL, host-only)
//! - `agent` identifies the agent making the request (matches the agent token's `sub`)
//! - `act` (REQUIRED, RFC 8693) actor claim — `act.sub` MUST match the `agent` claim
//! - `dwk` (REQUIRED) names the discovery document
//! - `jti` is captured when present (same caveat as agent tokens: no replay cache enforced)
//! - `sub` (OPTIONAL) the user who authorized the agent
//! - `scope` (OPTIONAL) granted permissions
//! - At least one of `sub` or `scope` MUST be present

use http_message_sig::keys::jwk::JWK;
use serde_json::{Map, Value};

use crate::errors::AAuthError;
use crate::tokens::validation::{
	decode_jwt_claims_unverified, decode_jwt_header, extract_cnf_jwk, get_scopes, get_string_claim,
	is_acceptable_jwt_issuer_url, validate_jwt,
};

pub const AUTH_TOKEN_TYP: &str = "aa-auth+jwt";

#[derive(Debug, Clone)]
pub struct AuthTokenResult {
	pub issuer: String,
	pub agent_id: String,
	pub act_sub: String,
	pub dwk: String,
	pub jti: Option<String>,
	pub user_id: Option<String>,
	pub scopes: Option<Vec<String>>,
	pub audience: Option<String>,
	pub cnf_jwk: JWK,
	pub claims: Map<String, Value>,
}

fn claim_matches_audience(claims: &Map<String, Value>, expected_audience: &str) -> bool {
	match claims.get("aud") {
		Some(Value::String(aud)) => aud == expected_audience,
		Some(Value::Array(values)) => values
			.iter()
			.filter_map(Value::as_str)
			.any(|aud| aud == expected_audience),
		_ => false,
	}
}

fn extract_act_sub(claims: &Map<String, Value>) -> Result<String, AAuthError> {
	let act = claims
		.get("act")
		.ok_or_else(|| AAuthError::MissingClaim("act".to_string()))?;
	let act_obj = act
		.as_object()
		.ok_or_else(|| AAuthError::JwtValidation("act claim is not an object".to_string()))?;
	act_obj
		.get("sub")
		.and_then(Value::as_str)
		.map(str::to_string)
		.ok_or_else(|| AAuthError::MissingClaim("act.sub".to_string()))
}

/// Validate an `aa-auth+jwt` token.
///
/// The caller is responsible for:
/// 1. Extracting the issuer ([`get_auth_token_issuer`]).
/// 2. Fetching the JWKS for that issuer.
/// 3. Selecting the correct key by `kid` from the JWT header.
///
/// `expected_audience` is the resource server's audience identifier; the token's `aud` claim
/// MUST contain it. `expected_agent`, if provided, is matched against the `agent` claim — used
/// when the resource has already established the calling agent via another channel.
pub fn validate_auth_token(
	jwt: &str,
	signing_jwk: &JWK,
	expected_audience: &str,
	expected_agent: Option<&str>,
	allow_insecure_http_issuer: bool,
) -> Result<AuthTokenResult, AAuthError> {
	let header = decode_jwt_header(jwt)?;
	let typ = header.typ.as_deref().unwrap_or("");
	if typ != AUTH_TOKEN_TYP {
		return Err(AAuthError::JwtValidation(format!(
			"expected typ={}, got typ={}",
			AUTH_TOKEN_TYP, typ
		)));
	}

	let claims = validate_jwt(jwt, signing_jwk, None)?;

	let issuer =
		get_string_claim(&claims, "iss").ok_or_else(|| AAuthError::MissingClaim("iss".to_string()))?;
	if !is_acceptable_jwt_issuer_url(&issuer, allow_insecure_http_issuer) {
		return Err(AAuthError::InvalidIssuerUrl);
	}

	let agent_id = get_string_claim(&claims, "agent")
		.ok_or_else(|| AAuthError::MissingClaim("agent".to_string()))?;
	if let Some(expected) = expected_agent
		&& agent_id != expected
	{
		return Err(AAuthError::JwtValidation(format!(
			"auth token agent mismatch: expected {}, got {}",
			expected, agent_id
		)));
	}

	if !claim_matches_audience(&claims, expected_audience) {
		return Err(AAuthError::AudienceMismatch);
	}

	let act_sub = extract_act_sub(&claims)?;
	if act_sub != agent_id {
		return Err(AAuthError::ActClaimMismatch);
	}

	let dwk =
		get_string_claim(&claims, "dwk").ok_or_else(|| AAuthError::MissingClaim("dwk".to_string()))?;
	let jti = get_string_claim(&claims, "jti");

	let user_id = get_string_claim(&claims, "sub");
	let scopes = get_scopes(&claims);
	let audience = get_string_claim(&claims, "aud");
	if user_id.is_none() && scopes.is_none() {
		return Err(AAuthError::JwtValidation(
			"auth token must contain at least one of sub or scope".to_string(),
		));
	}

	let cnf_jwk = extract_cnf_jwk(&claims)?;

	Ok(AuthTokenResult {
		issuer,
		agent_id,
		act_sub,
		dwk,
		jti,
		user_id,
		scopes,
		audience,
		cnf_jwk,
		claims,
	})
}

/// Read `iss` from an auth token WITHOUT validating the signature.
pub fn get_auth_token_issuer(jwt: &str) -> Result<String, AAuthError> {
	let claims = decode_jwt_claims_unverified(jwt)?;
	get_string_claim(&claims, "iss")
		.ok_or_else(|| AAuthError::JwtValidation("missing iss claim".to_string()))
}

/// Read the `kid` from an auth token header.
pub fn get_auth_token_kid(jwt: &str) -> Result<Option<String>, AAuthError> {
	Ok(decode_jwt_header(jwt)?.kid)
}

/// Extract `cnf.jwk` from an auth token payload WITHOUT validating the signature.
pub fn extract_auth_token_key(jwt: &str) -> Result<JWK, AAuthError> {
	extract_cnf_jwk(&decode_jwt_claims_unverified(jwt)?)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_test_claims() -> Map<String, Value> {
		let mut claims = Map::new();
		claims.insert(
			"iss".to_string(),
			Value::String("https://auth.example.com".to_string()),
		);
		claims.insert(
			"agent".to_string(),
			Value::String("aauth:local@agent.example.com".to_string()),
		);
		claims.insert("sub".to_string(), Value::String("user-456".to_string()));
		claims.insert("scope".to_string(), Value::String("read write".to_string()));
		claims.insert(
			"aud".to_string(),
			Value::String("https://resource.example.com".to_string()),
		);
		claims.insert(
			"dwk".to_string(),
			Value::String("aauth-access.json".to_string()),
		);
		claims.insert("jti".to_string(), Value::String("token-789".to_string()));

		let mut act = serde_json::Map::new();
		act.insert(
			"sub".to_string(),
			Value::String("aauth:local@agent.example.com".to_string()),
		);
		claims.insert("act".to_string(), Value::Object(act));

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
	fn test_audience_match() {
		let claims = make_test_claims();
		assert!(claim_matches_audience(
			&claims,
			"https://resource.example.com"
		));
		assert!(!claim_matches_audience(
			&claims,
			"https://other.example.com"
		));
	}

	#[test]
	fn test_act_sub() {
		let claims = make_test_claims();
		assert_eq!(
			extract_act_sub(&claims).unwrap(),
			"aauth:local@agent.example.com"
		);
	}

	#[test]
	fn test_scopes_from_claims() {
		let claims = make_test_claims();
		assert_eq!(get_scopes(&claims).unwrap(), vec!["read", "write"]);
	}
}
