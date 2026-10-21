//! `crypto::jwt::init` must fail closed when `jsonwebtoken` already latched its
//! own default provider. Runs in its own process to control that ordering.
#![cfg(feature = "fips")]

use jsonwebtoken::{Algorithm, EncodingKey, Header};

#[test]
#[should_panic(
	expected = "a JWT crypto provider was installed before crypto::init(); refusing to run without the FIPS policy"
)]
fn init_after_jwt_use_panics() {
	let key =
		EncodingKey::from_rsa_pem(include_bytes!("../src/crypto/testdata/rsa2048.pem")).unwrap();
	jsonwebtoken::encode(&Header::new(Algorithm::RS256), &serde_json::json!({}), &key).unwrap();

	agentgateway::crypto::jwt::init();
}
