use super::*;

#[test]
fn test_apikey_equality() {
	// APIKey equality must use a constant-time comparison (subtle::ConstantTimeEq): the gateway
	// compares attacker-controlled keys against configured secrets, and a short-circuiting
	// comparison would let an attacker recover a key byte-by-byte via response timing.
	// These assertions verify the constant-time implementation is behaviorally correct.
	assert_eq!(APIKey::new("test-api-key"), APIKey::new("test-api-key"));
	// Same length, differing only in the last byte
	assert_ne!(APIKey::new("test-api-key"), APIKey::new("test-api-kez"));
	// Matching prefix but different length
	assert_ne!(APIKey::new("test-api-key"), APIKey::new("test-api"));
	assert_ne!(APIKey::new(""), APIKey::new("test-api-key"));

	// Hash must stay consistent with PartialEq to keep the HashMap<APIKey, _> invariant
	let mut map = HashMap::new();
	map.insert(APIKey::new("test-api-key"), ());
	assert!(map.contains_key(&APIKey::new("test-api-key")));
	assert!(!map.contains_key(&APIKey::new("other-key")));
}

#[tokio::test]
async fn test_apikey_query_parameter_extracts_and_strips() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::QueryParameter {
			name: "api_key".into(),
		},
	);

	let mut req = ::http::Request::builder()
		.uri("http://example.com/data?api_key=test-api-key&keep=yes")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("api key should validate");

	assert_eq!(req.uri().to_string(), "http://example.com/data?keep=yes");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_cookie_extracts_and_strips() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::Cookie {
			name: "api_key".into(),
		},
	);

	let mut req = ::http::Request::builder()
		.uri("http://example.com/data")
		.header("cookie", "keep=yes; api_key=test-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("api key should validate");

	assert_eq!(
		req.headers().get("cookie").unwrap().to_str().unwrap(),
		"keep=yes"
	);
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_permissive_no_key_ok() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Permissive,
		AuthorizationLocation::bearer_header(),
	);

	let mut req = crate::http::Request::new(crate::http::Body::empty());

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("missing API key should be allowed in permissive mode");

	assert!(req.extensions().get::<Claims>().is_none());
}

#[tokio::test]
async fn test_apikey_permissive_invalid_key_ok_and_keeps_header() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Permissive,
		AuthorizationLocation::bearer_header(),
	);

	let mut req = ::http::Request::builder()
		.header(crate::http::header::AUTHORIZATION, "Bearer invalid-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("invalid API key should be allowed in permissive mode");

	assert!(
		req
			.headers()
			.get(crate::http::header::AUTHORIZATION)
			.is_some()
	);
	assert!(req.extensions().get::<Claims>().is_none());
}

#[tokio::test]
async fn test_apikey_permissive_valid_key_inserts_claims_and_removes_header() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Permissive,
		AuthorizationLocation::bearer_header(),
	);

	let mut req = ::http::Request::builder()
		.header(crate::http::header::AUTHORIZATION, "Bearer test-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("valid API key should validate in permissive mode");

	assert!(
		req
			.headers()
			.get(crate::http::header::AUTHORIZATION)
			.is_none()
	);
	assert!(req.extensions().get::<Claims>().is_some());
}
