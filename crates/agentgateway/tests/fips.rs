//! FIPS JWT policy through the public `jsonwebtoken` API. Runs in its own
//! process so `crypto::jwt::init` installs the provider before any JWT use.
#![cfg(feature = "fips")]

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{Value, json};

const RSA_2048: &[u8] = include_bytes!("../src/crypto/testdata/rsa2048.pem");
const RSA_2048_PUB: &[u8] = include_bytes!("../src/crypto/testdata/rsa2048.pub.der");
const RSA_2049: &[u8] = include_bytes!("../src/crypto/testdata/rsa2049.pem");

#[test]
fn fips_policy_applies_to_public_api() {
	agentgateway::crypto::jwt::init();
	agentgateway::crypto::jwt::init();
	let claims = json!({"sub": "test-user", "exp": 4102444800u64});

	let short_secret = [7u8; 13];
	let err = jsonwebtoken::encode(
		&Header::new(Algorithm::HS256),
		&claims,
		&EncodingKey::from_secret(&short_secret),
	)
	.unwrap_err();
	assert!(matches!(err.kind(), ErrorKind::Provider(_)), "{err:?}");

	// header {"alg":"HS256"}, payload {}, arbitrary signature
	let err = jsonwebtoken::decode::<Value>(
		"eyJhbGciOiJIUzI1NiJ9.e30.c2ln",
		&DecodingKey::from_secret(&short_secret),
		&Validation::new(Algorithm::HS256),
	)
	.unwrap_err();
	assert!(matches!(err.kind(), ErrorKind::Provider(_)), "{err:?}");

	let err = jsonwebtoken::encode(
		&Header::new(Algorithm::RS256),
		&claims,
		&EncodingKey::from_rsa_pem(RSA_2049).unwrap(),
	)
	.unwrap_err();
	assert!(matches!(err.kind(), ErrorKind::InvalidRsaKey(_)), "{err:?}");

	let token = jsonwebtoken::encode(
		&Header::new(Algorithm::RS256),
		&claims,
		&EncodingKey::from_rsa_pem(RSA_2048).unwrap(),
	)
	.unwrap();
	let mut validation = Validation::new(Algorithm::RS256);
	validation.required_spec_claims.clear();
	jsonwebtoken::decode::<Value>(
		&token,
		&DecodingKey::from_rsa_der(RSA_2048_PUB),
		&validation,
	)
	.unwrap();
}
