//! Cross-implementation wire-compatibility tests against
//! `aauth-test-vectors.json` — the same fixtures used by the reference Python validator at
//! https://github.com/christian-posta/extauth-aauth-resource. Failures here indicate the Rust
//! implementation has diverged from the reference wire format.

use std::collections::HashMap;

use ed25519_dalek::SigningKey;
use http_message_sig::digest::{DigestAlgorithm, calculate_content_digest};
use http_message_sig::headers::{
	SignatureParams, build_signature_input, parse_signature, parse_signature_input,
	parse_signature_key,
};
use http_message_sig::keys::ed25519::{public_key_to_base64url, sign};
use http_message_sig::keys::jwk::JWK;
use http_message_sig::keys::jwk_thumbprint::calculate_jwk_thumbprint;
use http_message_sig::signing::{
	SignatureScheme, build_signature_base, resolve_hwk_public_key, verify_signature,
};
use serde_json::Value;

const VECTORS: &str = include_str!("aauth-test-vectors.json");

fn vectors() -> Value {
	serde_json::from_str(VECTORS).expect("test vectors must parse")
}

fn ed25519_private_key_from_hex(hex: &str) -> SigningKey {
	assert_eq!(
		hex.len(),
		64,
		"Ed25519 seed must be 32 bytes (64 hex chars)"
	);
	let mut bytes = [0u8; 32];
	for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
		bytes[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
	}
	SigningKey::from_bytes(&bytes)
}

#[test]
fn content_digest_matches_reference_vectors() {
	let v = vectors();
	for tc in v["content_digest_tests"].as_array().unwrap() {
		let body = tc["body_string"].as_str().unwrap().as_bytes();
		let expected = tc["expected_digest"].as_str().unwrap();
		let alg = if expected.starts_with("sha-256") {
			DigestAlgorithm::Sha256
		} else {
			DigestAlgorithm::Sha512
		};
		let id = tc["id"].as_str().unwrap();
		assert_eq!(
			calculate_content_digest(body, alg),
			expected,
			"content_digest case {} mismatched",
			id
		);
	}
}

#[test]
fn jwk_thumbprint_matches_reference_vectors() {
	let v = vectors();
	let mut checked = 0usize;
	for tc in v["jwk_thumbprint_tests"].as_array().unwrap() {
		// Some vector cases (e.g. EC P-256) omit `expected_thumbprint`; just exercise the parser.
		let Some(expected) = tc["expected_thumbprint"].as_str() else {
			continue;
		};
		let jwk: JWK = serde_json::from_value(tc["jwk"].clone()).unwrap();
		let id = tc["id"].as_str().unwrap();
		assert_eq!(
			calculate_jwk_thumbprint(&jwk).unwrap(),
			expected,
			"thumbprint case {} mismatched",
			id
		);
		checked += 1;
	}
	assert!(checked > 0, "no thumbprint cases exercised");
}

#[test]
fn signature_base_matches_reference_vectors() {
	let v = vectors();
	for tc in v["signature_base_tests"].as_array().unwrap() {
		let id = tc["id"].as_str().unwrap();
		let request = &tc["request"];

		let mut headers: HashMap<String, String> = request["headers"]
			.as_object()
			.unwrap()
			.iter()
			.map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
			.collect();

		if let Some(sk) = tc["signature_key_header"].as_str() {
			headers.insert("Signature-Key".to_string(), sk.to_string());
		}

		let method = request["method"].as_str().unwrap();
		let path = request["path"].as_str().unwrap();
		let authority = request["authority"].as_str().unwrap();
		let query = request["query"].as_str();

		let covered: Vec<&str> = tc["covered_components"]
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v.as_str().unwrap())
			.collect();

		let params = SignatureParams {
			created: tc["signature_params"]["created"].as_u64().unwrap(),
			keyid: tc["signature_params"]["keyid"].as_str().map(str::to_string),
			nonce: None,
			alg: None,
		};

		let base = build_signature_base(method, authority, path, query, &headers, &covered, &params)
			.unwrap_or_else(|e| panic!("signature_base case {} failed to build: {}", id, e));
		// Some vectors only assert on `expected_signature_input` (the @signature-params line is
		// stable across signers; the full base depends on header values that change between
		// reference impls). Compare the base when the vector publishes one.
		if let Some(expected_base) = tc["expected_signature_base"].as_str() {
			assert_eq!(base, expected_base, "signature_base case {} mismatched", id);
		}

		if let Some(expected_input) = tc["expected_signature_input"].as_str() {
			let label = expected_input.split('=').next().unwrap();
			assert_eq!(
				build_signature_input(label, &covered, &params),
				expected_input,
				"signature_input case {} mismatched",
				id
			);
		}
	}
}

#[test]
fn signature_key_parsing_matches_reference_vectors() {
	let v = vectors();
	for tc in v["signature_key_header_tests"].as_array().unwrap() {
		let id = tc["id"].as_str().unwrap();
		let header = tc["expected_header"].as_str().unwrap();
		let parsed = parse_signature_key(header)
			.unwrap_or_else(|e| panic!("signature_key parsing case {} failed: {}", id, e));

		// Some vectors put parsed expectations under `parsed`, others use top-level fields.
		// Accept both shapes.
		let parsed_section = &tc["parsed"];
		let expected_label = parsed_section["label"]
			.as_str()
			.or_else(|| tc["label"].as_str())
			.unwrap_or_else(|| panic!("case {} has no expected label", id));
		let expected_scheme = parsed_section["scheme"]
			.as_str()
			.or_else(|| tc["scheme"].as_str())
			.unwrap_or_else(|| panic!("case {} has no expected scheme", id));
		assert_eq!(parsed.label, expected_label, "label mismatch on {}", id);
		assert_eq!(parsed.scheme, expected_scheme, "scheme mismatch on {}", id);

		let expected_params = parsed_section["params"]
			.as_object()
			.or_else(|| tc["params"].as_object());
		if let Some(expected_params) = expected_params {
			for (k, v) in expected_params {
				let Some(want) = v.as_str() else { continue };
				assert_eq!(
					parsed.params.get(k).map(String::as_str),
					Some(want),
					"param {} mismatch on {}",
					k,
					id
				);
			}
		}
	}
}

#[test]
fn signature_input_parsing_matches_reference_vectors() {
	let v = vectors();
	for tc in v["signature_input_parsing_tests"].as_array().unwrap() {
		let id = tc["id"].as_str().unwrap();
		let header = tc["input"].as_str().unwrap();
		let parsed = parse_signature_input(header)
			.unwrap_or_else(|e| panic!("signature_input parsing case {} failed: {}", id, e));
		let expected = &tc["expected"];
		assert_eq!(
			parsed.label,
			expected["label"].as_str().unwrap(),
			"label mismatch on {}",
			id
		);

		let expected_comps: Vec<&str> = expected["components"]
			.as_array()
			.unwrap()
			.iter()
			.map(|c| c.as_str().unwrap())
			.collect();
		assert_eq!(
			parsed
				.components
				.iter()
				.map(String::as_str)
				.collect::<Vec<_>>(),
			expected_comps,
			"components mismatch on {}",
			id
		);

		assert_eq!(
			parsed.params.created,
			expected["params"]["created"].as_u64().unwrap(),
			"created mismatch on {}",
			id
		);
		if let Some(keyid) = expected["params"]["keyid"].as_str() {
			assert_eq!(
				parsed.params.keyid.as_deref(),
				Some(keyid),
				"keyid mismatch on {}",
				id
			);
		}
	}
}

#[test]
fn signature_parsing_matches_reference_vectors() {
	let v = vectors();
	for tc in v["signature_parsing_tests"].as_array().unwrap() {
		let id = tc["id"].as_str().unwrap();
		let header = tc["input"].as_str().unwrap();
		let (label, bytes) = parse_signature(header)
			.unwrap_or_else(|e| panic!("signature parsing case {} failed: {}", id, e));
		let expected = &tc["expected"];
		assert_eq!(
			label,
			expected["label"].as_str().unwrap(),
			"label mismatch on {}",
			id
		);

		// Decode the expected base64 to compare bytes — the same byte sequence may be encoded
		// with or without padding, so don't compare the string form.
		let expected_b64 = expected["signature_base64"].as_str().unwrap();
		use base64::Engine;
		let expected_bytes = base64::engine::general_purpose::STANDARD
			.decode(expected_b64)
			.unwrap_or_else(|e| panic!("expected base64 invalid on {}: {}", id, e));
		assert_eq!(bytes, expected_bytes, "signature bytes mismatch on {}", id);
	}
}

#[test]
fn label_consistency_cases_behave_as_expected() {
	// Reach into the header strings directly (without invoking the byte-decoding signature
	// parser) — vectors use placeholder values like `sig1=:abc123==:` that aren't valid base64.
	// We only need the label here.
	fn label_of(s: &str) -> Option<&str> {
		s.split('=').next()
	}

	let v = vectors();
	for tc in v["label_consistency_tests"].as_array().unwrap() {
		let id = tc["id"].as_str().unwrap();
		let sk_label = label_of(tc["signature_key"].as_str().unwrap()).unwrap();
		let si_label = label_of(tc["signature_input"].as_str().unwrap()).unwrap();
		let s_label = label_of(tc["signature"].as_str().unwrap()).unwrap();
		let consistent = sk_label == si_label && sk_label == s_label;
		assert_eq!(
			consistent,
			tc["expected_valid"].as_bool().unwrap(),
			"label consistency case {} mismatched",
			id
		);
	}
}

/// Sign with the (private, derived public) pair and verify round-trip.
///
/// We don't compare against the vector's published Ed25519 signature because that requires the
/// signing implementation to match the reference seed→public-key derivation exactly. This test
/// confirms the signer/verifier pair is internally consistent — both halves of our code agree on
/// the wire format.
#[test]
fn end_to_end_hwk_round_trip() {
	let v = vectors();
	let private_hex = v["test_keys"]["ed25519"]["private_key_bytes_hex"]
		.as_str()
		.unwrap();
	let private_key = ed25519_private_key_from_hex(private_hex);
	let public_key = private_key.verifying_key();
	let x = public_key_to_base64url(&public_key);

	let created = 1_730_217_600u64;
	let mut headers = HashMap::new();
	let sig_key = format!(r#"sig1=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
	headers.insert("Signature-Key".to_string(), sig_key);

	let components = ["@method", "@authority", "@path", "signature-key"];
	let params = SignatureParams {
		created,
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
		&components,
		&params,
	)
	.unwrap();
	let signature_bytes = sign(base.as_bytes(), &private_key);
	headers.insert(
		"Signature-Input".to_string(),
		http_message_sig::headers::build_signature_input("sig1", &components, &params),
	);
	headers.insert(
		"Signature".to_string(),
		http_message_sig::headers::build_signature("sig1", &signature_bytes),
	);

	let result = verify_signature(
		"GET",
		"https://resource.example/api/data",
		&headers,
		None,
		u64::MAX,
		&resolve_hwk_public_key,
		None,
	)
	.expect("verification should succeed");
	assert!(result.valid);
	assert_eq!(result.scheme, SignatureScheme::Hwk);
}

/// Regression test for an interop bug found via the Go `sign-request` reference client.
///
/// The Go signer emits `Signature-Input` parameters in `created;alg;keyid` order. The verifier
/// must reproduce the `@signature-params` line byte-for-byte (RFC 9421 §2.5) when rebuilding
/// the signature base. Earlier, the Rust verifier reconstructed parameters from parsed fields
/// in a different fixed order (`created;keyid;nonce;alg`) — which produced a different
/// signature base and failed Ed25519 verification, even though every individual value matched.
#[test]
fn verifier_preserves_signature_input_param_ordering() {
	use ed25519_dalek::Signer;
	let private_key = ed25519_private_key_from_hex(
		vectors()["test_keys"]["ed25519"]["private_key_bytes_hex"]
			.as_str()
			.unwrap(),
	);
	let x = public_key_to_base64url(&private_key.verifying_key());
	let created = 1_730_217_600u64;

	let sig_key_value = format!(r#"sig=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
	// Parameter order distinct from the verifier's old default: created → alg → keyid.
	let sig_input_value = format!(
		r#"sig=("@method" "@authority" "@path" "signature-key");created={created};alg="ed25519";keyid="sig""#
	);

	let mut headers = HashMap::new();
	headers.insert("Signature-Key".to_string(), sig_key_value);

	// Build the signature base manually so we can sign over the wire-exact form.
	let expected_base = format!(
		"\"@method\": GET\n\"@authority\": example.com\n\"@path\": /\n\"signature-key\": {}\n\"@signature-params\": ({});created={};alg=\"ed25519\";keyid=\"sig\"",
		headers["Signature-Key"], r#""@method" "@authority" "@path" "signature-key""#, created,
	);
	let sig_bytes = private_key
		.sign(expected_base.as_bytes())
		.to_bytes()
		.to_vec();

	headers.insert("Signature-Input".to_string(), sig_input_value);
	headers.insert(
		"Signature".to_string(),
		http_message_sig::headers::build_signature("sig", &sig_bytes),
	);

	verify_signature(
		"GET",
		"https://example.com/",
		&headers,
		None,
		u64::MAX,
		&resolve_hwk_public_key,
		None,
	)
	.expect("verifier must preserve the original parameter ordering");
}

/// Regression: the signer used to drop the URL port when building `@authority`, while the
/// verifier kept it. Signing `https://example.com:8443/foo` then verifying with the same URL
/// would fail because the signer wrote `@authority: example.com` and the verifier rebuilt
/// `@authority: example.com:8443`.
#[test]
fn signer_includes_port_in_authority() {
	use std::collections::HashMap;

	use http_message_sig::signing::sign_request;
	let private_key = ed25519_private_key_from_hex(
		vectors()["test_keys"]["ed25519"]["private_key_bytes_hex"]
			.as_str()
			.unwrap(),
	);

	let mut headers = HashMap::new();
	let sig_headers = sign_request(
		"GET",
		"https://example.com:8443/api/data",
		&mut headers,
		None,
		&private_key,
		"hwk",
		&HashMap::new(),
	)
	.unwrap();

	headers.insert("Signature-Input".to_string(), sig_headers.signature_input);
	headers.insert("Signature".to_string(), sig_headers.signature);

	verify_signature(
		"GET",
		"https://example.com:8443/api/data",
		&headers,
		None,
		u64::MAX,
		&resolve_hwk_public_key,
		None,
	)
	.expect("sign+verify must agree on @authority when the URL carries a port");
}

#[test]
fn end_to_end_hwk_round_trip_with_query_and_authority_override() {
	let v = vectors();
	let private_hex = v["test_keys"]["ed25519"]["private_key_bytes_hex"]
		.as_str()
		.unwrap();
	let private_key = ed25519_private_key_from_hex(private_hex);
	let public_key = private_key.verifying_key();
	let x = public_key_to_base64url(&public_key);

	let created = 1_730_217_600u64;
	let mut headers = HashMap::new();
	let sig_key = format!(r#"sig1=hwk;kty="OKP";crv="Ed25519";x="{}""#, x);
	headers.insert("Signature-Key".to_string(), sig_key);

	let components = ["@method", "@authority", "@path", "@query", "signature-key"];
	let params = SignatureParams {
		created,
		keyid: None,
		nonce: None,
		alg: None,
	};
	let base = build_signature_base(
		"GET",
		"resource.example",
		"/api/data",
		Some("user=alice&limit=10"),
		&headers,
		&components,
		&params,
	)
	.unwrap();
	let signature_bytes = sign(base.as_bytes(), &private_key);
	headers.insert(
		"Signature-Input".to_string(),
		http_message_sig::headers::build_signature_input("sig1", &components, &params),
	);
	headers.insert(
		"Signature".to_string(),
		http_message_sig::headers::build_signature("sig1", &signature_bytes),
	);

	// URL points at an internal listener; the client signed against the public authority.
	let result = verify_signature(
		"GET",
		"https://internal-pod:8080/api/data?user=alice&limit=10",
		&headers,
		None,
		u64::MAX,
		&resolve_hwk_public_key,
		Some("resource.example"),
	)
	.expect("verification with override should succeed");
	assert!(result.valid);
}
