use std::collections::HashMap;

use crate::errors::Error;
use crate::keys::jwk::JWK;

/// A parsed Signature-Key header entry.
///
/// Per draft-hardt-httpbis-signature-key, this header is an RFC 8941 Structured Field Dictionary
/// keyed by signature label, with the scheme as the dictionary value and key material in
/// parameters. Example: `sig1=hwk;kty="OKP";crv="Ed25519";x="..."`.
#[derive(Debug, Clone)]
pub struct SignatureKey {
	pub label: String,
	/// One of `hwk`, `jwks_uri`, `jwt` (other schemes from the draft are not supported here).
	pub scheme: String,
	pub params: HashMap<String, String>,
}

/// Parse a Signature-Key header value. Accepts both the modern semicolon-parameter form
/// (`label=scheme;param=val`) and the legacy parenthesized form (`label=(scheme=hwk param=val)`)
/// emitted by earlier drafts.
pub fn parse_signature_key(header: &str) -> Result<SignatureKey, Error> {
	let (label, value) = header
		.split_once('=')
		.ok_or_else(|| Error::InvalidHeader(format!("invalid signature-key header: {}", header)))?;
	let label = label.trim().to_string();
	let value = value.trim();

	if value.starts_with('(') && value.ends_with(')') {
		let inner = &value[1..value.len() - 1];
		parse_parenthesized_format(label, inner)
	} else {
		parse_semicolon_format(label, value)
	}
}

fn parse_parenthesized_format(label: String, inner: &str) -> Result<SignatureKey, Error> {
	let mut scheme = String::new();
	let mut params = HashMap::new();

	// Split by whitespace, respecting double-quoted values.
	let mut parts = Vec::new();
	let mut current = String::new();
	let mut in_quotes = false;
	for ch in inner.chars() {
		match ch {
			'"' => {
				in_quotes = !in_quotes;
				current.push(ch);
			},
			' ' | '\t' if !in_quotes => {
				if !current.is_empty() {
					parts.push(std::mem::take(&mut current));
				}
			},
			_ => current.push(ch),
		}
	}
	if !current.is_empty() {
		parts.push(current);
	}

	for part in parts {
		let Some((key, val)) = part.split_once('=') else {
			return Err(Error::InvalidHeader(format!(
				"invalid signature-key parameter: {}",
				part
			)));
		};
		let key = key.trim().to_string();
		let val = val.trim().trim_matches('"').to_string();
		if key == "scheme" {
			scheme = val;
		} else {
			params.insert(key, val);
		}
	}

	if scheme.is_empty() {
		return Err(Error::InvalidHeader(
			"signature-key missing scheme".to_string(),
		));
	}

	Ok(SignatureKey {
		label,
		scheme,
		params,
	})
}

fn parse_semicolon_format(label: String, value: &str) -> Result<SignatureKey, Error> {
	let mut parts = value.split(';');
	let scheme = parts
		.next()
		.ok_or_else(|| Error::InvalidHeader("empty signature-key value".to_string()))?
		.trim()
		.to_string();
	if scheme.is_empty() {
		return Err(Error::InvalidHeader(
			"signature-key missing scheme".to_string(),
		));
	}

	let mut params = HashMap::new();
	for part in parts {
		let part = part.trim();
		if part.is_empty() {
			continue;
		}
		if let Some((key, val)) = part.split_once('=') {
			let key = key.trim().to_string();
			let val = val.trim().trim_matches('"').to_string();
			params.insert(key, val);
		}
	}

	Ok(SignatureKey {
		label,
		scheme,
		params,
	})
}

/// Build a Signature-Key header value for the `hwk` scheme.
///
/// Format: `sig1=hwk;kty="OKP";crv="Ed25519";x="..."`.
pub fn build_signature_key_hwk(label: &str, jwk: &JWK) -> Result<String, Error> {
	let mut parts = vec!["hwk".to_string()];
	parts.push(format!("kty=\"{}\"", jwk.kty));
	if let Some(crv) = &jwk.crv {
		parts.push(format!("crv=\"{}\"", crv));
	}
	if let Some(x) = &jwk.x {
		parts.push(format!("x=\"{}\"", x));
	}
	Ok(format!("{}={}", label, parts.join(";")))
}

/// Build a Signature-Key header value for the `jwks_uri` scheme.
///
/// Format: `sig1=jwks_uri;id="https://...";dwk="aauth-agent.json";kid="key-1"`.
/// `dwk` names the well-known metadata document used to discover the JWKS URI; it is required
/// by the draft.
pub fn build_signature_key_jwks(label: &str, id: &str, kid: &str, dwk: &str) -> String {
	format!(
		"{}=jwks_uri;id=\"{}\";dwk=\"{}\";kid=\"{}\"",
		label, id, dwk, kid
	)
}

/// Build a Signature-Key header value for the `jwt` scheme.
///
/// Format: `sig1=jwt;jwt="eyJ..."`. The JWT must carry `cnf.jwk` with the signing key.
pub fn build_signature_key_jwt(label: &str, jwt: &str) -> String {
	format!("{}=jwt;jwt=\"{}\"", label, jwt)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_hwk_semicolon() {
		let header =
			r#"sig1=hwk;kty="OKP";crv="Ed25519";x="JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs""#;
		let parsed = parse_signature_key(header).unwrap();
		assert_eq!(parsed.label, "sig1");
		assert_eq!(parsed.scheme, "hwk");
		assert_eq!(parsed.params.get("kty").map(String::as_str), Some("OKP"));
		assert_eq!(
			parsed.params.get("crv").map(String::as_str),
			Some("Ed25519")
		);
	}

	#[test]
	fn test_parse_hwk_legacy_parenthesized() {
		let header = r#"sig1=(scheme=hwk kty="OKP" crv="Ed25519" x="JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs")"#;
		let parsed = parse_signature_key(header).unwrap();
		assert_eq!(parsed.scheme, "hwk");
		assert_eq!(parsed.params.get("kty").map(String::as_str), Some("OKP"));
	}

	#[test]
	fn test_parse_jwks_uri() {
		let header = r#"sig1=jwks_uri;id="https://agent.example";dwk="aauth-agent.json";kid="key-1""#;
		let parsed = parse_signature_key(header).unwrap();
		assert_eq!(parsed.scheme, "jwks_uri");
		assert_eq!(
			parsed.params.get("id").map(String::as_str),
			Some("https://agent.example")
		);
		assert_eq!(parsed.params.get("kid").map(String::as_str), Some("key-1"));
		assert_eq!(
			parsed.params.get("dwk").map(String::as_str),
			Some("aauth-agent.json")
		);
	}

	#[test]
	fn test_parse_jwt_scheme() {
		let header = r#"sig1=jwt;jwt="eyJhbGciOiJFZERTQSJ9.e30.sig""#;
		let parsed = parse_signature_key(header).unwrap();
		assert_eq!(parsed.scheme, "jwt");
		assert_eq!(
			parsed.params.get("jwt").map(String::as_str),
			Some("eyJhbGciOiJFZERTQSJ9.e30.sig")
		);
	}

	#[test]
	fn test_parse_rejects_missing_scheme() {
		assert!(parse_signature_key("sig1=").is_err());
	}

	#[test]
	fn test_parse_rejects_malformed_header() {
		assert!(parse_signature_key("not-a-header").is_err());
	}

	#[test]
	fn test_build_hwk() {
		let jwk = JWK {
			kty: "OKP".to_string(),
			crv: Some("Ed25519".to_string()),
			x: Some("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs".to_string()),
			y: None,
			d: None,
			n: None,
			e: None,
			kid: None,
			alg: None,
			extra: Default::default(),
		};
		let header = build_signature_key_hwk("sig1", &jwk).unwrap();
		assert_eq!(
			header,
			r#"sig1=hwk;kty="OKP";crv="Ed25519";x="JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs""#
		);
	}

	#[test]
	fn test_build_jwks() {
		let header =
			build_signature_key_jwks("sig1", "https://agent.example", "key-1", "aauth-agent.json");
		assert_eq!(
			header,
			r#"sig1=jwks_uri;id="https://agent.example";dwk="aauth-agent.json";kid="key-1""#
		);
	}
}
