use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::digest::{calculate_content_digest, parse_content_digest_header};
use crate::errors::Error;
use crate::headers::{SignatureKey, parse_signature, parse_signature_input, parse_signature_key};
use crate::keys::ed25519::{PublicKey, public_key_from_bytes, verify};
#[cfg(test)]
use crate::signing::signature_base::build_signature_base;
use crate::signing::signature_base::build_signature_base_raw;

/// Case-insensitive header lookup (HTTP header names are case-insensitive per RFC 7230 §3.2).
fn get_header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a String> {
	headers.get(name).or_else(|| {
		headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case(name))
			.map(|(_, v)| v)
	})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureScheme {
	/// Pseudonymous: signed by an inline public key with no identity claim.
	Hwk,
	/// Identified: signed by a key discovered via the agent's `/.well-known/...` document.
	Jwks,
	/// Authorized: signed by a key bound to a JWT (cnf.jwk).
	Jwt,
}

#[derive(Debug, Clone)]
pub struct VerificationResult {
	pub valid: bool,
	pub scheme: SignatureScheme,
	/// Set when the scheme conveys an issuer/agent identifier (jwks_uri's `id`).
	pub agent_id: Option<String>,
	/// Populated by higher-level token validation; this layer leaves it `None`.
	pub agent_delegate: Option<String>,
	/// Populated by higher-level token validation; this layer leaves it `None`.
	pub claims: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Verify an HTTP Message Signature per RFC 9421, with the AAuth required-components profile.
///
/// Algorithm:
/// 1. Parse the three signature headers (case-insensitive header lookup).
/// 2. Verify all three carry the same label.
/// 3. Verify `created` is within `timestamp_tolerance` seconds of now.
/// 4. Verify the required AAuth components are covered: `@method`, `@authority`, `@path`,
///    `signature-key`. Verify `content-digest` is covered if the request carries that header,
///    and `authorization` is covered if it does.
/// 5. Resolve the public key via `public_key_resolver`.
/// 6. Rebuild the signature base (overriding `@authority` if `authority_override` is set).
/// 7. Verify the Ed25519 signature.
/// 8. If a body is supplied and `content-digest` is covered, verify the digest against the body.
///
/// `authority_override` exists because the gateway may receive the request at one authority
/// (e.g. an internal listener) while the client signed against another (the route hostname).
pub fn verify_signature(
	method: &str,
	url: &str,
	headers: &HashMap<String, String>,
	body: Option<&[u8]>,
	timestamp_tolerance: u64,
	public_key_resolver: &(dyn Fn(&SignatureKey) -> Result<PublicKey, Error> + Send + Sync),
	authority_override: Option<&str>,
) -> Result<VerificationResult, Error> {
	let sig_key_header = get_header(headers, "Signature-Key").ok_or(Error::MissingSignatureKey)?;
	let sig_input_header =
		get_header(headers, "Signature-Input").ok_or(Error::MissingSignatureInput)?;
	let sig_header = get_header(headers, "Signature").ok_or(Error::MissingSignature)?;

	let sig_key = parse_signature_key(sig_key_header)?;
	let sig_input = parse_signature_input(sig_input_header)?;
	let (sig_label, sig_bytes) = parse_signature(sig_header)?;

	tracing::debug!(
		scheme = %sig_key.scheme,
		label = %sig_key.label,
		"http-message-sig: parsed signature headers"
	);

	if sig_key.label != sig_input.label || sig_key.label != sig_label {
		tracing::debug!(
			sig_key_label = %sig_key.label,
			sig_input_label = %sig_input.label,
			sig_label = %sig_label,
			"http-message-sig: label mismatch across headers"
		);
		return Err(Error::LabelMismatch);
	}

	let now = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_err(|_| Error::InvalidHeader("system clock before UNIX epoch".to_string()))?
		.as_secs();
	let time_diff = now.abs_diff(sig_input.params.created);
	if time_diff > timestamp_tolerance {
		tracing::debug!(
			created = sig_input.params.created,
			now,
			time_diff,
			tolerance = timestamp_tolerance,
			"http-message-sig: signature timestamp outside tolerance"
		);
		return Err(Error::TimestampExpired);
	}

	// Component identifiers are case-insensitive per RFC 9421 §2 — match accordingly so a
	// client that signs `"Authorization"` (capital A) is not falsely rejected as "missing
	// required component" when `build_signature_base_raw` would happily look the header up
	// case-insensitively.
	let covers = |name: &str| {
		sig_input
			.components
			.iter()
			.any(|c| c.eq_ignore_ascii_case(name))
	};

	let required_components = ["@method", "@authority", "@path", "signature-key"];
	for req in required_components {
		if !covers(req) {
			return Err(Error::InvalidSignature(format!(
				"missing required component: {}",
				req
			)));
		}
	}
	if get_header(headers, "content-digest").is_some() && !covers("content-digest") {
		return Err(Error::InvalidSignature(
			"content-digest header present but not covered".to_string(),
		));
	}
	if get_header(headers, "authorization").is_some() && !covers("authorization") {
		return Err(Error::InvalidSignature(
			"authorization header present but not covered".to_string(),
		));
	}

	// Reject unsupported schemes BEFORE invoking the key resolver — the resolver may attempt a
	// network fetch or other expensive work whose failure mode would mask the underlying
	// "this scheme isn't accepted at all" signal.
	let scheme = match sig_key.scheme.as_str() {
		"hwk" => SignatureScheme::Hwk,
		"jwks_uri" => SignatureScheme::Jwks,
		"jwt" => SignatureScheme::Jwt,
		other => return Err(Error::UnsupportedScheme(other.to_string())),
	};
	let public_key = public_key_resolver(&sig_key)?;

	let parsed_url = url::Url::parse(url)?;
	let authority: String = if let Some(override_auth) = authority_override {
		override_auth.to_string()
	} else {
		let host = parsed_url
			.host_str()
			.ok_or_else(|| Error::InvalidHeader("missing host in URL".to_string()))?;
		let port = parsed_url.port();
		match port {
			Some(p) => format!("{}:{}", host, p),
			None => host.to_string(),
		}
	};
	let path = parsed_url.path();
	let query = parsed_url.query();

	let component_refs: Vec<&str> = sig_input.components.iter().map(String::as_str).collect();
	// Use the raw Signature-Input value so the `@signature-params` line is reproduced
	// byte-for-byte. Reconstructing the params line from parsed fields silently changes
	// parameter ordering and breaks Ed25519 verification.
	let signature_base = build_signature_base_raw(
		method,
		&authority,
		path,
		query,
		headers,
		&component_refs,
		&sig_input.raw_value,
	)?;

	if !verify(signature_base.as_bytes(), &sig_bytes, &public_key) {
		tracing::debug!(
			signature_base = %signature_base.replace('\n', "\\n"),
			"http-message-sig: Ed25519 signature verification failed"
		);
		return Err(Error::InvalidSignature(
			"Ed25519 signature verification failed".to_string(),
		));
	}

	if let Some(body_bytes) = body
		&& sig_input
			.components
			.iter()
			.any(|c| c.eq_ignore_ascii_case("content-digest"))
	{
		let cd_header = get_header(headers, "content-digest").ok_or_else(|| {
			Error::InvalidSignature(
				"content-digest covered but header absent during verification".to_string(),
			)
		})?;
		let alg = parse_content_digest_header(cd_header)?;
		let expected = calculate_content_digest(body_bytes, alg);
		if cd_header.as_str() != expected {
			return Err(Error::ContentDigestMismatch);
		}
	}

	let agent_id = match scheme {
		SignatureScheme::Jwks => sig_key.params.get("id").cloned(),
		// JWT and Hwk identity comes from the higher layer (token validation / Hwk has none).
		SignatureScheme::Jwt | SignatureScheme::Hwk => None,
	};

	Ok(VerificationResult {
		valid: true,
		scheme,
		agent_id,
		agent_delegate: None,
		claims: None,
	})
}

/// Extract the inline Ed25519 public key from a `hwk` Signature-Key entry.
pub fn resolve_hwk_public_key(sig_key: &SignatureKey) -> Result<PublicKey, Error> {
	if sig_key.scheme != "hwk" {
		return Err(Error::UnsupportedScheme(sig_key.scheme.clone()));
	}
	let x = sig_key
		.params
		.get("x")
		.ok_or_else(|| Error::InvalidKey("hwk missing x parameter".to_string()))?;
	public_key_from_bytes(x)
}

#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use super::*;
	use crate::keys::ed25519::{generate_keypair, public_key_to_base64url, sign};

	fn now() -> u64 {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
	}

	#[test]
	fn test_round_trip_hwk_signature() {
		// Generate a key, build a signed request by hand, verify it.
		let (private_key, public_key) = generate_keypair();
		let x = public_key_to_base64url(&public_key);
		let created = now();

		let sig_key_value = format!(r#"sig1=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
		let sig_input_value = format!(
			r#"sig1=("@method" "@authority" "@path" "signature-key");created={}"#,
			created
		);

		let mut headers = HashMap::new();
		headers.insert("Signature-Key".to_string(), sig_key_value.clone());

		let base = build_signature_base(
			"GET",
			"example.com",
			"/",
			None,
			&headers,
			&["@method", "@authority", "@path", "signature-key"],
			&crate::headers::SignatureParams {
				created,
				keyid: None,
				nonce: None,
				alg: None,
			},
		)
		.unwrap();
		let signature_bytes = sign(base.as_bytes(), &private_key);
		let signature_value = crate::headers::build_signature("sig1", &signature_bytes);

		headers.insert("Signature-Input".to_string(), sig_input_value);
		headers.insert("Signature".to_string(), signature_value);

		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			60,
			&resolve_hwk_public_key,
			None,
		)
		.unwrap();
		assert!(result.valid);
		assert_eq!(result.scheme, SignatureScheme::Hwk);
	}

	#[test]
	fn test_legacy_jwks_scheme_rejected() {
		// The current draft is `scheme=jwks_uri`. Earlier drafts used `scheme=jwks`; that bare
		// form is intentionally rejected — we don't accept stale wire forms as aliases.
		let mut headers = HashMap::new();
		headers.insert(
			"Signature-Key".to_string(),
			r#"sig1=jwks;id="https://agent.example";dwk="aauth-agent.json";kid="key-1""#.to_string(),
		);
		headers.insert(
			"Signature-Input".to_string(),
			r#"sig1=("@method" "@authority" "@path" "signature-key");created=1"#.to_string(),
		);
		headers.insert("Signature".to_string(), "sig1=:AA==:".to_string());
		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			u64::MAX,
			&|_| Err(Error::InvalidKey("not used".to_string())),
			None,
		);
		assert!(
			matches!(result, Err(Error::UnsupportedScheme(ref s)) if s == "jwks"),
			"expected UnsupportedScheme(\"jwks\"), got {:?}",
			result,
		);
	}

	#[test]
	fn test_missing_signature_key_header_rejected() {
		let headers = HashMap::new();
		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			60,
			&resolve_hwk_public_key,
			None,
		);
		assert!(matches!(result, Err(Error::MissingSignatureKey)));
	}

	#[test]
	fn test_label_mismatch_rejected() {
		let mut headers = HashMap::new();
		headers.insert(
			"Signature-Key".to_string(),
			r#"sig1=hwk;kty="OKP";crv="Ed25519";x="AAAA""#.to_string(),
		);
		headers.insert(
			"Signature-Input".to_string(),
			r#"sig2=("@method" "@authority" "@path" "signature-key");created=1"#.to_string(),
		);
		headers.insert("Signature".to_string(), "sig1=:AA==:".to_string());
		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			u64::MAX,
			&resolve_hwk_public_key,
			None,
		);
		assert!(matches!(result, Err(Error::LabelMismatch)));
	}

	#[test]
	fn test_covered_components_match_case_insensitively() {
		// RFC 9421 §2: HTTP header field names in component identifiers are
		// case-insensitive. (Derived components like `@method` must be lowercase per the
		// spec — only header names get the case-insensitive treatment.) The verifier's
		// "required component" / "covered if present" check must therefore match
		// "Authorization" and "Content-Digest" against "authorization" / "content-digest".
		let (private_key, public_key) = generate_keypair();
		let x = public_key_to_base64url(&public_key);
		let created = now();

		let sig_key_value = format!(r#"sig1=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
		let mut headers = HashMap::new();
		headers.insert("Signature-Key".to_string(), sig_key_value);
		headers.insert("Authorization".to_string(), "Bearer dummy".to_string());

		// Sign with mixed-case `Authorization` in the components list. The required-set
		// uses lowercase identifiers, so this must match case-insensitively.
		let components = [
			"@method",
			"@authority",
			"@path",
			"signature-key",
			"Authorization",
		];
		let params = crate::headers::SignatureParams {
			created,
			keyid: None,
			nonce: None,
			alg: None,
		};
		let base = build_signature_base(
			"GET",
			"example.com",
			"/",
			None,
			&headers,
			&components,
			&params,
		)
		.unwrap();
		let signature_bytes = crate::keys::ed25519::sign(base.as_bytes(), &private_key);
		headers.insert(
			"Signature-Input".to_string(),
			crate::headers::build_signature_input("sig1", &components, &params),
		);
		headers.insert(
			"Signature".to_string(),
			crate::headers::build_signature("sig1", &signature_bytes),
		);

		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			u64::MAX,
			&resolve_hwk_public_key,
			None,
		)
		.expect("mixed-case header identifiers must satisfy the case-insensitive coverage check");
		assert!(result.valid);
	}

	#[test]
	fn test_missing_required_component_rejected() {
		// Signature-Input lacks @method.
		let mut headers = HashMap::new();
		headers.insert(
			"Signature-Key".to_string(),
			r#"sig1=hwk;kty="OKP";crv="Ed25519";x="AAAA""#.to_string(),
		);
		headers.insert(
			"Signature-Input".to_string(),
			r#"sig1=("@authority" "@path" "signature-key");created=1"#.to_string(),
		);
		headers.insert("Signature".to_string(), "sig1=:AA==:".to_string());
		let result = verify_signature(
			"GET",
			"https://example.com/",
			&headers,
			None,
			u64::MAX,
			&resolve_hwk_public_key,
			None,
		);
		assert!(matches!(result, Err(Error::InvalidSignature(_))));
	}

	#[test]
	fn test_authority_override_used() {
		// Verifier overrides `@authority` to "verified.example:8443" before rebuilding the base,
		// regardless of what the URL says.
		let (private_key, public_key) = generate_keypair();
		let x = public_key_to_base64url(&public_key);
		let created = now();
		let signed_authority = "verified.example:8443";

		let sig_key_value = format!(r#"sig1=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
		let mut headers = HashMap::new();
		headers.insert("Signature-Key".to_string(), sig_key_value);

		let base = build_signature_base(
			"GET",
			signed_authority,
			"/api",
			None,
			&headers,
			&["@method", "@authority", "@path", "signature-key"],
			&crate::headers::SignatureParams {
				created,
				keyid: None,
				nonce: None,
				alg: None,
			},
		)
		.unwrap();
		let signature_bytes = sign(base.as_bytes(), &private_key);
		headers.insert(
			"Signature-Input".to_string(),
			format!(
				r#"sig1=("@method" "@authority" "@path" "signature-key");created={}"#,
				created
			),
		);
		headers.insert(
			"Signature".to_string(),
			crate::headers::build_signature("sig1", &signature_bytes),
		);

		// URL has a totally different authority — should still verify with the override.
		let result = verify_signature(
			"GET",
			"https://internal-listener:18080/api",
			&headers,
			None,
			60,
			&resolve_hwk_public_key,
			Some(signed_authority),
		)
		.unwrap();
		assert!(result.valid);
	}
}
