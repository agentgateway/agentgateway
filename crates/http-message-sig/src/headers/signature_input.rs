use crate::errors::Error;

#[derive(Debug, Clone)]
pub struct SignatureInput {
	pub label: String,
	pub components: Vec<String>,
	pub params: SignatureParams,
	/// The full Signature-Input value verbatim (everything after `label=`), preserved so the
	/// verifier can reproduce the `@signature-params` line byte-for-byte — parameter order and
	/// quoting are part of the signature base and any reformatting breaks Ed25519 verification.
	pub raw_value: String,
}

#[derive(Debug, Clone)]
pub struct SignatureParams {
	pub created: u64,
	pub keyid: Option<String>,
	pub nonce: Option<String>,
	pub alg: Option<String>,
}

/// Parse a Signature-Input header value.
///
/// Format: `label=("comp1" "comp2" ...);created=1234567890[;keyid="..."][;nonce="..."][;alg="..."]`.
pub fn parse_signature_input(header: &str) -> Result<SignatureInput, Error> {
	let (label, rest) = header
		.split_once('=')
		.ok_or_else(|| Error::InvalidHeader(format!("invalid signature-input header: {}", header)))?;
	let label = label.trim().to_string();
	let rest = rest.trim();

	let components_start = rest
		.find('(')
		.ok_or_else(|| Error::InvalidHeader("missing components list".to_string()))?;
	let components_end = rest[components_start..]
		.find(')')
		.ok_or_else(|| Error::InvalidHeader("unclosed components list".to_string()))?
		+ components_start;

	let components_str = &rest[components_start + 1..components_end];
	let components: Vec<String> = components_str
		.split_whitespace()
		.map(|s| s.trim_matches('"').to_string())
		.collect();

	let params_str = &rest[components_end + 1..];
	let mut created: Option<u64> = None;
	let mut keyid: Option<String> = None;
	let mut nonce: Option<String> = None;
	let mut alg: Option<String> = None;

	for raw_param in params_str.split(';') {
		let param = raw_param.trim();
		if param.is_empty() {
			continue;
		}
		if let Some(created_str) = param.strip_prefix("created=") {
			created = Some(created_str.parse().map_err(|_| {
				Error::InvalidHeader(format!("invalid created timestamp: {}", created_str))
			})?);
		} else if let Some(keyid_str) = param.strip_prefix("keyid=") {
			keyid = Some(keyid_str.trim_matches('"').to_string());
		} else if let Some(nonce_str) = param.strip_prefix("nonce=") {
			nonce = Some(nonce_str.trim_matches('"').to_string());
		} else if let Some(alg_str) = param.strip_prefix("alg=") {
			alg = Some(alg_str.trim_matches('"').to_string());
		}
	}

	// `created` is REQUIRED per RFC 9421 §4.1; reject its absence rather than silently using 0,
	// which would render the verifier's freshness check meaningless against an operator who
	// happens to configure a very large `timestamp_tolerance`.
	let created = created.ok_or_else(|| {
		Error::InvalidHeader("signature-input missing 'created' parameter".to_string())
	})?;

	Ok(SignatureInput {
		label,
		components,
		params: SignatureParams {
			created,
			keyid,
			nonce,
			alg,
		},
		raw_value: rest.to_string(),
	})
}

/// Build a Signature-Input header value.
///
/// Component identifiers are canonicalized to lowercase here to match
/// `build_signature_base`'s lowercasing of identifiers — without that, a caller passing
/// mixed-case (e.g. `"Authorization"`) would produce a Signature-Input header that
/// disagrees with the signed `@signature-params` line, breaking verification on the
/// other side. RFC 9421 §2 says identifiers SHOULD be lowercase, so canonicalizing here
/// is also the spec-recommended form.
pub fn build_signature_input(label: &str, components: &[&str], params: &SignatureParams) -> String {
	let comps_str = components
		.iter()
		.map(|c| format!("\"{}\"", c.to_lowercase()))
		.collect::<Vec<_>>()
		.join(" ");

	let mut result = format!("{}=({});created={}", label, comps_str, params.created);
	if let Some(keyid) = &params.keyid {
		result.push_str(&format!(";keyid=\"{}\"", keyid));
	}
	if let Some(nonce) = &params.nonce {
		result.push_str(&format!(";nonce=\"{}\"", nonce));
	}
	if let Some(alg) = &params.alg {
		result.push_str(&format!(";alg=\"{}\"", alg));
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_signature_input_basic() {
		let header = r#"sig1=("@method" "@authority" "@path" "signature-key");created=1730217600"#;
		let parsed = parse_signature_input(header).unwrap();
		assert_eq!(parsed.label, "sig1");
		assert_eq!(
			parsed.components,
			vec!["@method", "@authority", "@path", "signature-key"]
		);
		assert_eq!(parsed.params.created, 1730217600);
		assert!(parsed.params.keyid.is_none());
	}

	#[test]
	fn test_parse_signature_input_with_all_params() {
		let header = r#"sig-b26=("date" "@method" "@path");created=1618884473;keyid="test-key";nonce="abc";alg="ed25519""#;
		let parsed = parse_signature_input(header).unwrap();
		assert_eq!(parsed.params.keyid.as_deref(), Some("test-key"));
		assert_eq!(parsed.params.nonce.as_deref(), Some("abc"));
		assert_eq!(parsed.params.alg.as_deref(), Some("ed25519"));
	}

	#[test]
	fn test_parse_signature_input_rejects_bad_timestamp() {
		let header = r#"sig1=("@method");created=notanumber"#;
		assert!(parse_signature_input(header).is_err());
	}

	#[test]
	fn test_parse_signature_input_rejects_missing_created() {
		// RFC 9421 §4.1 makes `created` required. Defaulting it to 0 would silently disable
		// the verifier's freshness check on misconfigured deployments.
		let header = r#"sig1=("@method" "@authority")"#;
		let err = parse_signature_input(header).unwrap_err();
		assert!(
			matches!(err, Error::InvalidHeader(ref m) if m.contains("created")),
			"unexpected error: {err}",
		);
	}

	#[test]
	fn test_build_signature_input_basic() {
		let params = SignatureParams {
			created: 1730217600,
			keyid: None,
			nonce: None,
			alg: None,
		};
		let header = build_signature_input("sig1", &["@method", "@authority", "@path"], &params);
		assert_eq!(
			header,
			r#"sig1=("@method" "@authority" "@path");created=1730217600"#
		);
	}
}
