use std::collections::HashMap;

use crate::errors::Error;
use crate::headers::signature_input::SignatureParams;

/// Build the signature base from a raw `Signature-Input` value (`("comp1" ...);created=...`).
///
/// The verifier MUST use this entry point so the `@signature-params` line is reproduced
/// byte-for-byte from the signer's original wire form. Reconstructing the line from parsed
/// fields would silently reorder or normalize parameters and break Ed25519 verification —
/// parameter ordering is part of the signature base.
///
/// `covered_components` MUST match the components list inside `raw_signature_input_value` (the
/// caller is expected to have parsed them out of the same string).
pub fn build_signature_base_raw(
	method: &str,
	authority: &str,
	path: &str,
	query: Option<&str>,
	headers: &HashMap<String, String>,
	covered_components: &[&str],
	raw_signature_input_value: &str,
) -> Result<String, Error> {
	let mut lines =
		build_component_lines(method, authority, path, query, headers, covered_components)?;
	lines.push(format!(
		"\"@signature-params\": {}",
		raw_signature_input_value
	));
	Ok(lines.join("\n"))
}

fn build_component_lines(
	method: &str,
	authority: &str,
	path: &str,
	query: Option<&str>,
	headers: &HashMap<String, String>,
	covered_components: &[&str],
) -> Result<Vec<String>, Error> {
	let mut lines = Vec::with_capacity(covered_components.len() + 1);
	for component in covered_components {
		let value = if let Some(derived) = component.strip_prefix('@') {
			match derived {
				"method" => method.to_uppercase(),
				"authority" => authority.to_lowercase(),
				"path" => path.to_string(),
				"query" => match query {
					Some(q) if q.starts_with('?') => q.to_string(),
					Some(q) => format!("?{}", q),
					None => "?".to_string(),
				},
				_ => {
					return Err(Error::InvalidHeader(format!(
						"unknown derived component: {}",
						component
					)));
				},
			}
		} else {
			let header_value = headers
				.iter()
				.find(|(k, _)| k.eq_ignore_ascii_case(component))
				.map(|(_, v)| v.as_str())
				.ok_or_else(|| Error::InvalidHeader(format!("missing header: {}", component)))?;
			header_value.trim().to_string()
		};
		lines.push(format!("\"{}\": {}", component.to_lowercase(), value));
	}
	Ok(lines)
}

/// Build the signature base per RFC 9421 Section 2.5.
///
/// This is the byte string the signer/verifier feeds to Ed25519 (or any other algorithm). It is
/// the most error-prone part of RFC 9421 — interop bugs almost always trace back to here.
///
/// Algorithm:
/// 1. For each component in `covered_components` (IN ORDER):
///    - Derived components (`@method`, `@authority`, `@path`, `@query`) are computed from the
///      request line.
///    - All other names are looked up case-insensitively in `headers`.
/// 2. Each line is `"<component>": <value>` with the component name double-quoted and lowercased.
/// 3. The `@signature-params` line is appended last: `"@signature-params": (...);created=...;...`.
/// 4. Lines are joined with a single LF (0x0A), no trailing newline.
pub fn build_signature_base(
	method: &str,
	authority: &str,
	path: &str,
	query: Option<&str>,
	headers: &HashMap<String, String>,
	covered_components: &[&str],
	signature_params: &SignatureParams,
) -> Result<String, Error> {
	let mut lines =
		build_component_lines(method, authority, path, query, headers, covered_components)?;

	let comps_str = covered_components
		.iter()
		.map(|c| format!("\"{}\"", c.to_lowercase()))
		.collect::<Vec<_>>()
		.join(" ");

	let mut params_str = format!("({});created={}", comps_str, signature_params.created);
	if let Some(keyid) = &signature_params.keyid {
		params_str.push_str(&format!(";keyid=\"{}\"", keyid));
	}
	if let Some(nonce) = &signature_params.nonce {
		params_str.push_str(&format!(";nonce=\"{}\"", nonce));
	}
	if let Some(alg) = &signature_params.alg {
		params_str.push_str(&format!(";alg=\"{}\"", alg));
	}

	lines.push(format!("\"@signature-params\": {}", params_str));
	Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_signature_base_minimal_get() {
		let mut headers = HashMap::new();
		let sig_key =
			r#"sig1=hwk;kty="OKP";crv="Ed25519";x="JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs""#;
		headers.insert("Signature-Key".to_string(), sig_key.to_string());

		let params = SignatureParams {
			created: 1730217600,
			keyid: None,
			nonce: None,
			alg: None,
		};

		let base = build_signature_base(
			"GET",
			"resource.example",
			"/api/data",
			None,
			&headers,
			&["@method", "@authority", "@path", "signature-key"],
			&params,
		)
		.unwrap();

		let expected = format!(
			"\"@method\": GET\n\"@authority\": resource.example\n\"@path\": /api/data\n\"signature-key\": {}\n\"@signature-params\": (\"@method\" \"@authority\" \"@path\" \"signature-key\");created=1730217600",
			sig_key
		);
		assert_eq!(base, expected);
	}

	#[test]
	fn test_signature_base_includes_query() {
		let mut headers = HashMap::new();
		headers.insert("Signature-Key".to_string(), "sig1=hwk".to_string());

		let params = SignatureParams {
			created: 1,
			keyid: None,
			nonce: None,
			alg: None,
		};

		let base = build_signature_base(
			"GET",
			"example.com",
			"/api",
			Some("user=alice&limit=10"),
			&headers,
			&["@method", "@authority", "@path", "@query", "signature-key"],
			&params,
		)
		.unwrap();

		assert!(base.contains("\"@query\": ?user=alice&limit=10"));
	}

	#[test]
	fn test_signature_base_missing_header_errors() {
		let headers = HashMap::new();
		let params = SignatureParams {
			created: 1,
			keyid: None,
			nonce: None,
			alg: None,
		};
		let result = build_signature_base(
			"GET",
			"example.com",
			"/",
			None,
			&headers,
			&["@method", "signature-key"],
			&params,
		);
		assert!(result.is_err());
	}

	#[test]
	fn test_authority_is_lowercased() {
		let mut headers = HashMap::new();
		headers.insert("Signature-Key".to_string(), "sig1=hwk".to_string());
		let params = SignatureParams {
			created: 1,
			keyid: None,
			nonce: None,
			alg: None,
		};
		let base = build_signature_base(
			"GET",
			"Example.COM:8443",
			"/",
			None,
			&headers,
			&["@authority", "signature-key"],
			&params,
		)
		.unwrap();
		assert!(base.contains("\"@authority\": example.com:8443"));
	}
}
