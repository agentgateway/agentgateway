use super::Provider;
use itertools::Itertools;
use serde_json::json;

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
