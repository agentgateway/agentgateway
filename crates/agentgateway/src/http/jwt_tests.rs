use itertools::Itertools;
use serde_json::json;

use super::{Jwt, Mode, TokenError};
use super::Provider;

#[test]
pub fn test_azure_jwks() {
	// Regression test for https://github.com/agentgateway/agentgateway/issues/477
	let azure_ad = json!({
		"keys": [{
			"kty": "RSA",
			"use": "sig",
			"kid": "PoVKeirIOvmTyLQ9G9BenBwos7k",
			"x5t": "PoVKeirIOvmTyLQ9G9BenBwos7k",
			"n": "ruYyUq1ElSb8QCCt0XWWRSFpUq0JkyfEvvlCa4fPDi0GZbSGgJg3qYa0co2RsBIYHczXkc71kHVpktySAgYK1KMK264e-s7Vymeq-ypHEDpRsaWric_kKEIvKZzRsyUBUWf0CUhtuUvAbDTuaFnQ4g5lfoa7u3vtsv1za5Gmn6DUPirrL_-xqijP9IsHGUKaTmB4M_qnAu6vUHCpXZnN0YTJDoK7XrVJFaKj8RrTdJB89GFJeTFHA2OX472ToyLdCDn5UatYwmht62nXGlH7_G1kW1YMpeSSwzpnMEzUUk7A8UXrvFTHXEpfXhsv0LA59dm9Hi1mIXaOe1w-icA_rQ",
			"e": "AQAB",
			"x5c": [
				"MIIC/jCCAeagAwIBAgIJAM52mWWK+FEeMA0GCSqGSIb3DQEBCwUAMC0xKzApBgNVBAMTImFjY291bnRzLmFjY2Vzc2NvbnRyb2wud2luZG93cy5uZXQwHhcNMjUwMzIwMDAwNTAyWhcNMzAwMzIwMDAwNTAyWjAtMSswKQYDVQQDEyJhY2NvdW50cy5hY2Nlc3Njb250cm9sLndpbmRvd3MubmV0MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAruYyUq1ElSb8QCCt0XWWRSFpUq0JkyfEvvlCa4fPDi0GZbSGgJg3qYa0co2RsBIYHczXkc71kHVpktySAgYK1KMK264e+s7Vymeq+ypHEDpRsaWric/kKEIvKZzRsyUBUWf0CUhtuUvAbDTuaFnQ4g5lfoa7u3vtsv1za5Gmn6DUPirrL/+xqijP9IsHGUKaTmB4M/qnAu6vUHCpXZnN0YTJDoK7XrVJFaKj8RrTdJB89GFJeTFHA2OX472ToyLdCDn5UatYwmht62nXGlH7/G1kW1YMpeSSwzpnMEzUUk7A8UXrvFTHXEpfXhsv0LA59dm9Hi1mIXaOe1w+icA/rQIDAQABoyEwHzAdBgNVHQ4EFgQUcZ2MLLOas+d9WbkFSnPdxag09YIwDQYJKoZIhvcNAQELBQADggEBABPXBmwv703IlW8Zc9Kj7W215+vyM5lrJjUubnl+s8vQVXvyN7bh5xP2hzEKWb+u5g/brSIKX/A7qP3m/z6C8R9GvP5WRtF2w1CAxYZ9TWTzTS1La78edME546QejjveC1gX9qcLbEwuLAbYpau2r3vlIqgyXo+8WLXA0neGIRa2JWTNy8FJo0wnUttGJz9LQE4L37nR3HWIxflmOVgbaeyeaj2VbzUE7MIHIkK1bqye2OiKU82w1QWLV/YCny0xdLipE1g2uNL8QVob8fTU2zowd2j54c1YTBDy/hTsxpXfCFutKwtELqWzYxKTqYfrRCc1h0V4DGLKzIjtggTC+CY="
			],
			"cloud_instance_name": "microsoftonline.com",
			"issuer": "https://login.microsoftonline.com/{tenantid}/v2.0"
	}]});
	let jwks = serde_json::from_value(azure_ad).unwrap();
	let p = Provider::from_jwks(
		jwks,
		"https://login.microsoftonline.com/test/v2.0".to_string(),
		vec!["test-aud".to_string()],
	)
	.unwrap();
	assert_eq!(
		p.keys.keys().collect_vec(),
		vec!["PoVKeirIOvmTyLQ9G9BenBwos7k"]
	);
}

#[test]
pub fn test_basic_jwks() {
	let azure_ad = json!({
		"keys": [
			{
				"use": "sig",
				"kty": "EC",
				"kid": "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI",
				"crv": "P-256",
				"alg": "ES256",
				"x": "XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo",
				"y": "g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE"
			}
		]
	});
	let jwks = serde_json::from_value(azure_ad).unwrap();
	let p = Provider::from_jwks(
		jwks,
		"https://example.com".to_string(),
		vec!["test-aud".to_string()],
	)
	.unwrap();
	assert_eq!(
		p.keys.keys().collect_vec(),
		vec!["XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI"]
	);
}

fn setup_test_jwt() -> (Jwt, &'static str, &'static str, &'static str) {
	let jwks = json!({
		"keys": [
			{
				"use": "sig",
				"kty": "EC",
				"kid": "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI",
				"crv": "P-256",
				"alg": "ES256",
				"x": "XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo",
				"y": "g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE"
			}
		]
	});
	let jwks = serde_json::from_value(jwks).unwrap();

	let issuer = "https://example.com";
	let allowed_aud = "allowed-aud";
	let kid = "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI";

	let mut provider = Provider::from_jwks(
		jwks,
		issuer.to_string(),
		vec![allowed_aud.to_string()],
	)
	.unwrap();
	// Test-only: allow synthetic tokens without a real signature
	provider
		.keys
		.get_mut(kid)
		.unwrap()
		.validation
		.insecure_disable_signature_validation();

	(
		Jwt {
			mode: Mode::Strict,
			providers: vec![provider],
		},
		kid,
		issuer,
		allowed_aud,
	)
}

fn build_unsigned_token(kid: &str, iss: &str, aud: &str, exp: u64) -> String {
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
	let header = json!({ "alg": "ES256", "kid": kid });
	let payload = json!({ "iss": iss, "aud": aud, "exp": exp });
	let h = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
	let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
	let s = URL_SAFE_NO_PAD.encode(b"sig");
	format!("{h}.{p}.{s}")
}

#[test]
pub fn test_jwt_rejections_table() {
	use std::time::{SystemTime, UNIX_EPOCH};
	use jsonwebtoken::errors::ErrorKind;

	let (jwt, kid, issuer, allowed_aud) = setup_test_jwt();
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

	#[derive(Copy, Clone)]
	enum Expected {
		Aud,
		Iss,
		Exp,
	}
	let cases = [
		(
			"aud_mismatch",
			issuer,
			"wrong-aud",
			now + 600,
			Expected::Aud,
		),
		(
			"iss_mismatch",
			"https://wrong.example.com",
			allowed_aud,
			now + 600,
			Expected::Iss,
		),
		(
			"expired",
			issuer,
			allowed_aud,
			now - 100_000,
			Expected::Exp,
		),
	];

	for (name, iss, aud, exp, expected) in cases {
		let token = build_unsigned_token(kid, iss, aud, exp);
		let res = jwt.validate_claims(&token);
		match res {
			Err(TokenError::Invalid(e)) => match expected {
				Expected::Aud => assert!(matches!(e.kind(), ErrorKind::InvalidAudience), "{name}"),
				Expected::Iss => assert!(matches!(e.kind(), ErrorKind::InvalidIssuer), "{name}"),
				Expected::Exp => assert!(matches!(e.kind(), ErrorKind::ExpiredSignature), "{name}"),
			},
			other => panic!("{name}: expected Invalid(..), got {:?}", other),
		}
	}
}
