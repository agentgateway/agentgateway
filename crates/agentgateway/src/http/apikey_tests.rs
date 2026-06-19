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

fn sha256_hex(raw: &str) -> String {
	hex::encode(Sha256::digest(raw.as_bytes()))
}

fn auth_from_json(value: serde_json::Value) -> APIKeyAuthentication {
	let local: LocalAPIKeys =
		serde_json::from_value(value).expect("API key config should deserialize");
	local.into()
}

#[test]
fn test_apikeyhash_parse_accepts_lowercase_and_uppercase() {
	let lower = format!("sha256:{}", sha256_hex("k"));
	let upper = format!("sha256:{}", sha256_hex("k").to_ascii_uppercase());
	assert_eq!(
		APIKeyHash::parse(&lower).unwrap(),
		APIKeyHash::parse(&upper).unwrap()
	);
	assert_eq!(
		APIKeyHash::parse(&lower).unwrap(),
		APIKeyHash::from_raw_key("k")
	);
}

#[test]
fn test_apikeyhash_parse_rejects_malformed() {
	assert!(APIKeyHash::parse("no-prefix").is_err());
	assert!(APIKeyHash::parse("sha256:not-hex").is_err());
	assert!(APIKeyHash::parse("sha256:abcd").is_err());
}

#[test]
fn test_stored_key_parse_hash_routes_bcrypt_and_sha256() {
	assert!(matches!(
		StoredKey::parse_hash(&format!("sha256:{}", sha256_hex("k"))),
		Ok(StoredKey::Sha256(_))
	));
	let bhash = bcrypt::hash("k", 4).unwrap();
	assert!(matches!(
		StoredKey::parse_hash(&bhash),
		Ok(StoredKey::Bcrypt(_))
	));
}

#[test]
fn test_local_apikey_rejects_invalid_keyhash() {
	let err = serde_json::from_value::<LocalAPIKeys>(serde_json::json!({
		"keys": [{ "keyHash": "sha256:not-hex" }],
		"mode": "strict"
	}));
	assert!(err.is_err(), "invalid keyHash must fail to deserialize");
}

#[tokio::test]
async fn test_apikey_sha256_valid_authenticates() {
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": format!("sha256:{}", sha256_hex("raw-secret")) }],
		"mode": "strict"
	}));
	let mut req = bearer_request("raw-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("sha256 hashed key should validate with raw key");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_sha256_wrong_key_rejected() {
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": format!("sha256:{}", sha256_hex("raw-secret")) }],
		"mode": "strict"
	}));
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
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": bcrypt::hash("raw-secret", 4).unwrap() }],
		"mode": "strict"
	}));
	let mut req = bearer_request("raw-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("bcrypt hashed key should validate with raw key");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_bcrypt_wrong_key_rejected() {
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": bcrypt::hash("raw-secret", 4).unwrap() }],
		"mode": "strict"
	}));
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
	let auth = auth_from_json(serde_json::json!({
		"keys": [
			{ "key": "plain-key", "metadata": {"kind": "plain"} },
			{ "keyHash": format!("sha256:{}", sha256_hex("sha-key")), "metadata": {"kind": "sha"} },
			{ "keyHash": bcrypt::hash("bcrypt-key", 4).unwrap(), "metadata": {"kind": "bcrypt"} }
		],
		"mode": "strict"
	}));

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
async fn test_apikey_sha256_claims_expose_presented_key() {
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": format!("sha256:{}", sha256_hex("raw-secret")) }],
		"mode": "strict"
	}));
	let mut req = bearer_request("raw-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("sha256 hashed key should validate");
	let expr = crate::cel::Expression::new_strict("apiKey.key.unredacted()").unwrap();
	assert_eq!(
		crate::cel::Executor::new_request(&req)
			.eval(&expr)
			.unwrap()
			.json()
			.unwrap(),
		serde_json::json!("raw-secret")
	);
}

#[tokio::test]
async fn test_apikey_sha256_permissive_invalid_key_no_claims() {
	let auth = auth_from_json(serde_json::json!({
		"keys": [{ "keyHash": format!("sha256:{}", sha256_hex("raw-secret")) }],
		"mode": "permissive"
	}));
	let mut req = bearer_request("wrong-secret");
	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("permissive mode allows invalid key");
	assert!(req.extensions().get::<Claims>().is_none());
}
