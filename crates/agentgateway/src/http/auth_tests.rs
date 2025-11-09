use secrecy::SecretString;
use serde_json::Map;

use super::*;
use crate::http::jwt::Claims;
use crate::test_helpers::proxymock::setup_proxy_test;

#[tokio::test]
async fn test_backend_auth_passthrough_happy_path() {
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let mut req = crate::http::Request::new(crate::http::Body::empty());
	// Insert claims with a JWT that Passthrough should forward as Authorization
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::new("header.payload.signature".into()),
	});
	// Ensure there is no pre-existing Authorization
	assert!(req.headers().get(http::header::AUTHORIZATION).is_none());

	let backend_info = BackendInfo {
		name: "test".into(),
		inputs,
	};
	apply_backend_auth(&backend_info, &BackendAuth::Passthrough {}, &mut req)
		.await
		.expect("apply backend auth");

	// Assert Authorization header added with Bearer <jwt>
	let auth = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header must be set");
	assert_eq!(auth.to_str().unwrap(), "Bearer header.payload.signature");
	assert!(auth.is_sensitive());
	// Claims remain
	assert!(req.extensions().get::<Claims>().is_some());
}
