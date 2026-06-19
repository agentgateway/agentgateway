use super::*;

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

fn bearer_request(token: &str) -> crate::http::Request {
	::http::Request::builder()
		.header(
			crate::http::header::AUTHORIZATION,
			format!("Bearer {token}"),
		)
		.body(axum::body::Body::empty())
		.unwrap()
}

#[test]
fn test_parse_stored_key_classifies_by_prefix() {
	assert!(matches!(parse_stored_key("plain-key"), StoredKey::Plain(_)));
	assert!(matches!(
		parse_stored_key(&format!("sha256:{}", sha256_hex("k"))),
		StoredKey::Sha256Hex(_)
	));
	let bhash = bcrypt::hash("k", 4).unwrap();
	assert!(matches!(parse_stored_key(&bhash), StoredKey::Bcrypt(_)));
}

#[test]
fn test_parse_stored_key_sha256_uppercase_normalized() {
	let upper = format!("sha256:{}", sha256_hex("k").to_ascii_uppercase());
	match parse_stored_key(&upper) {
		StoredKey::Sha256Hex(h) => assert_eq!(h, sha256_hex("k")),
		_ => panic!("expected sha256 digest"),
	}
}

#[test]
fn test_parse_stored_key_malformed_sha256_falls_back_to_plain() {
	assert!(matches!(
		parse_stored_key("sha256:not-hex"),
		StoredKey::Plain(_)
	));
	assert!(matches!(
		parse_stored_key("sha256:abcd"),
		StoredKey::Plain(_)
	));
}

#[tokio::test]
async fn test_apikey_sha256_valid_authenticates() {
	let stored = format!("sha256:{}", sha256_hex("raw-secret"));
	let auth = APIKeyAuthentication::new(
		[(APIKey::new(stored), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::bearer_header(),
	);
	let mut req = bearer_request("raw-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("sha256 hashed key should validate with raw key");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_sha256_wrong_key_rejected() {
	let stored = format!("sha256:{}", sha256_hex("raw-secret"));
	let auth = APIKeyAuthentication::new(
		[(APIKey::new(stored), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::bearer_header(),
	);
	let mut req = bearer_request("wrong-secret");
	assert!(
		crate::test_helpers::test_policy(&auth, &mut req)
			.await
			.is_err(),
		"wrong key must be rejected in strict mode"
	);
}

#[tokio::test]
async fn test_apikey_bcrypt_valid_authenticates() {
	let stored = bcrypt::hash("raw-secret", 4).unwrap();
	let auth = APIKeyAuthentication::new(
		[(APIKey::new(stored), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::bearer_header(),
	);
	let mut req = bearer_request("raw-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("bcrypt hashed key should validate with raw key");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_bcrypt_wrong_key_rejected() {
	let stored = bcrypt::hash("raw-secret", 4).unwrap();
	let auth = APIKeyAuthentication::new(
		[(APIKey::new(stored), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::bearer_header(),
	);
	let mut req = bearer_request("wrong-secret");
	assert!(
		crate::test_helpers::test_policy(&auth, &mut req)
			.await
			.is_err(),
		"wrong key must be rejected in strict mode"
	);
}

#[tokio::test]
async fn test_apikey_mixed_secret_each_kind_authenticates() {
	let plain = (
		"plain-key".to_string(),
		serde_json::json!({"kind": "plain"}),
	);
	let sha = (
		format!("sha256:{}", sha256_hex("sha-key")),
		serde_json::json!({"kind": "sha"}),
	);
	let bc = (
		bcrypt::hash("bcrypt-key", 4).unwrap(),
		serde_json::json!({"kind": "bcrypt"}),
	);
	let auth = APIKeyAuthentication::new(
		[
			(APIKey::new(plain.0), plain.1),
			(APIKey::new(sha.0), sha.1),
			(APIKey::new(bc.0), bc.1),
		],
		Mode::Strict,
		AuthorizationLocation::bearer_header(),
	);

	for (raw, expected_kind) in [
		("plain-key", "plain"),
		("sha-key", "sha"),
		("bcrypt-key", "bcrypt"),
	] {
		let mut req = bearer_request(raw);
		let _ = crate::test_helpers::test_policy(&auth, &mut req)
			.await
			.unwrap_or_else(|_| panic!("{raw} should authenticate"));
		let claims = req.extensions().get::<Claims>().expect("claims present");
		assert_eq!(claims.metadata["kind"], expected_kind);
	}

	let mut req = bearer_request("not-a-key");
	assert!(
		crate::test_helpers::test_policy(&auth, &mut req)
			.await
			.is_err(),
		"unknown key must be rejected"
	);
}

#[tokio::test]
async fn test_apikey_sha256_permissive_invalid_key_no_claims() {
	let stored = format!("sha256:{}", sha256_hex("raw-secret"));
	let auth = APIKeyAuthentication::new(
		[(APIKey::new(stored), serde_json::Value::Null)],
		Mode::Permissive,
		AuthorizationLocation::bearer_header(),
	);
	let mut req = bearer_request("wrong-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("permissive mode allows invalid key");
	assert!(req.extensions().get::<Claims>().is_none());
}
