use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::digest::{DigestAlgorithm, calculate_content_digest};
use crate::errors::Error;
use crate::headers::{
	SignatureParams, build_signature, build_signature_input, build_signature_key_hwk,
	build_signature_key_jwks, build_signature_key_jwt,
};
use crate::keys::ed25519::{PrivateKey, public_key_to_base64url, sign};
use crate::keys::jwk::JWK;
use crate::signing::signature_base::build_signature_base;

pub struct SignatureHeaders {
	pub signature_input: String,
	pub signature: String,
	pub signature_key: String,
}

/// Sign an HTTP request per RFC 9421 with an Ed25519 key, producing the three required headers.
///
/// This is the symmetric inverse of [`super::verify_signature`]. It is useful both for tests
/// (cross-impl wire compatibility) and for any forward-signing use case a caller may add.
///
/// Covered components: always `@method`, `@authority`, `@path`, `signature-key`. `@query` is
/// included if the URL has a query string. `content-type` and `content-digest` are included if a
/// body is provided.
pub fn sign_request(
	method: &str,
	url: &str,
	headers: &mut HashMap<String, String>,
	body: Option<&[u8]>,
	private_key: &PrivateKey,
	scheme: &str,
	scheme_params: &HashMap<String, String>,
) -> Result<SignatureHeaders, Error> {
	let parsed_url = url::Url::parse(url)?;
	let host = parsed_url
		.host_str()
		.ok_or_else(|| Error::InvalidHeader("missing host in URL".to_string()))?;
	// Include the port in `@authority` whenever the URL carries one — this must match the
	// verifier's authority construction byte-for-byte, otherwise a request signed against
	// `https://example.com:8443/...` would build a signature base with authority `example.com`
	// while the verifier reconstructs `example.com:8443`, breaking Ed25519 verification.
	let authority = match parsed_url.port() {
		Some(p) => format!("{}:{}", host, p),
		None => host.to_string(),
	};
	let path = parsed_url.path();
	let query = parsed_url.query();

	let label = "sig1";
	let signature_key = match scheme {
		"hwk" => {
			let public_key = private_key.verifying_key();
			let jwk = JWK {
				kty: "OKP".to_string(),
				crv: Some("Ed25519".to_string()),
				x: Some(public_key_to_base64url(&public_key)),
				y: None,
				d: None,
				n: None,
				e: None,
				kid: None,
				alg: None,
				extra: Default::default(),
			};
			build_signature_key_hwk(label, &jwk)?
		},
		"jwks_uri" => {
			let id = scheme_params
				.get("id")
				.ok_or_else(|| Error::InvalidHeader("jwks_uri scheme missing 'id'".to_string()))?;
			let kid = scheme_params
				.get("kid")
				.ok_or_else(|| Error::InvalidHeader("jwks_uri scheme missing 'kid'".to_string()))?;
			let dwk = scheme_params
				.get("dwk")
				.map(String::as_str)
				.unwrap_or("aauth-agent.json");
			build_signature_key_jwks(label, id, kid, dwk)
		},
		"jwt" => {
			let jwt = scheme_params
				.get("jwt")
				.ok_or_else(|| Error::InvalidHeader("jwt scheme missing 'jwt'".to_string()))?;
			build_signature_key_jwt(label, jwt)
		},
		other => return Err(Error::UnsupportedScheme(other.to_string())),
	};

	headers.insert("Signature-Key".to_string(), signature_key.clone());

	let mut components: Vec<&str> = vec!["@method", "@authority", "@path", "signature-key"];
	if query.is_some() {
		components.push("@query");
	}
	if body.is_some() {
		components.push("content-type");
		components.push("content-digest");
	}

	if let Some(body_bytes) = body {
		let digest = calculate_content_digest(body_bytes, DigestAlgorithm::Sha256);
		headers.insert("Content-Digest".to_string(), digest);
	}

	let created = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_err(|_| Error::InvalidHeader("system clock before UNIX epoch".to_string()))?
		.as_secs();

	let params = SignatureParams {
		created,
		keyid: None,
		nonce: None,
		alg: None,
	};

	let signature_base = build_signature_base(
		method,
		&authority,
		path,
		query,
		headers,
		&components,
		&params,
	)?;
	let signature_bytes = sign(signature_base.as_bytes(), private_key);

	Ok(SignatureHeaders {
		signature_input: build_signature_input(label, &components, &params),
		signature: build_signature(label, &signature_bytes),
		signature_key,
	})
}
