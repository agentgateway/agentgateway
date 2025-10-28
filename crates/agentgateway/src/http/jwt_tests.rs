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

#[test]
pub fn test_jwt_audience_rejected() {
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
	use std::time::{SystemTime, UNIX_EPOCH};

	// Build a JWKS with a known EC key
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

	// Provider configured for audience "allowed-aud"
	let mut provider = Provider::from_jwks(
		jwks,
		"https://example.com".to_string(),
		vec!["allowed-aud".to_string()],
	)
	.unwrap();

	// Disable signature validation for the test so we don't need a private key
	let kid = "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI";
	let key = provider.keys.get_mut(kid).unwrap();
	key.validation.insecure_disable_signature_validation();

	let jwt = Jwt {
		mode: Mode::Strict,
		providers: vec![provider],
	};

	// Craft a token with mismatched audience
	let header = json!({
		"alg": "ES256",
		"kid": kid,
	});
	let exp = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_secs()
		+ 600;
	let payload = json!({
		"iss": "https://example.com",
		"aud": "wrong-aud",
		"exp": exp,
	});
	let header_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
	let payload_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
	let sig_enc = URL_SAFE_NO_PAD.encode(b"sig");
	let token = format!("{header_enc}.{payload_enc}.{sig_enc}");

	let res = jwt.validate_claims(&token);
	assert!(matches!(res, Err(TokenError::Invalid(_))));
}

#[test]
pub fn test_jwt_issuer_rejected() {
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
	use std::time::{SystemTime, UNIX_EPOCH};

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

	// Provider expects issuer https://example.com and audience allowed-aud
	let mut provider = Provider::from_jwks(
		jwks,
		"https://example.com".to_string(),
		vec!["allowed-aud".to_string()],
	)
	.unwrap();

	// Disable signature validation so we can use a synthetic token
	let kid = "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI";
	let key = provider.keys.get_mut(kid).unwrap();
	key.validation.insecure_disable_signature_validation();

	let jwt = Jwt {
		mode: Mode::Strict,
		providers: vec![provider],
	};

	// Build a token with wrong issuer
	let header = json!({
		"alg": "ES256",
		"kid": kid,
	});
	let exp = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_secs()
		+ 600;
	let payload = json!({
		"iss": "https://wrong.example.com",
		"aud": "allowed-aud",
		"exp": exp,
	});
	let header_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
	let payload_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
	let sig_enc = URL_SAFE_NO_PAD.encode(b"sig");
	let token = format!("{header_enc}.{payload_enc}.{sig_enc}");

	let res = jwt.validate_claims(&token);
	match res {
		Err(TokenError::Invalid(e)) => {
			assert!(matches!(e.kind(), jsonwebtoken::errors::ErrorKind::InvalidIssuer));
		},
		other => panic!("expected Invalid(InvalidIssuer), got {:?}", other),
	}
}

#[test]
pub fn test_jwt_expired_rejected() {
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
	use std::time::{SystemTime, UNIX_EPOCH};

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

	let mut provider = Provider::from_jwks(
		jwks,
		"https://example.com".to_string(),
		vec!["allowed-aud".to_string()],
	)
	.unwrap();

	// Disable signature validation so we can use a synthetic token
	let kid = "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI";
	let key = provider.keys.get_mut(kid).unwrap();
	key.validation.insecure_disable_signature_validation();

	let jwt = Jwt {
		mode: Mode::Strict,
		providers: vec![provider],
	};

	// Build a token with an expired exp
	let header = json!({
		"alg": "ES256",
		"kid": kid,
	});
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
	let payload = json!({
		"iss": "https://example.com",
		"aud": "allowed-aud",
		"exp": now - 100000,
	});
	let header_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
	let payload_enc = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
	let sig_enc = URL_SAFE_NO_PAD.encode(b"sig");
	let token = format!("{header_enc}.{payload_enc}.{sig_enc}");

	let res = jwt.validate_claims(&token);
	match res {
		Err(TokenError::Invalid(e)) => {
			assert!(matches!(e.kind(), jsonwebtoken::errors::ErrorKind::ExpiredSignature));
		},
		other => panic!("expected Invalid(ExpiredSignature), got {:?}", other),
	}
}
