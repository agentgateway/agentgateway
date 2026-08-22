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

#[test]
fn test_allowed_models_matching() {
	let cases: &[(&str, Option<&[&str]>, &str, bool)] = &[
		("omitted allows all", None, "gpt-4o", true),
		("empty denies all", Some(&[]), "gpt-4o", false),
		("exact match", Some(&["gpt-4o"]), "gpt-4o", true),
		("exact mismatch", Some(&["gpt-4o"]), "gpt-4.1", false),
		("prefix match", Some(&["openai/*"]), "openai/gpt-4o", true),
		(
			"prefix mismatch",
			Some(&["openai/*"]),
			"anthropic/claude",
			false,
		),
		("suffix match", Some(&["*-latest"]), "gpt-4-latest", true),
		(
			"suffix mismatch",
			Some(&["*-latest"]),
			"gpt-4-preview",
			false,
		),
		("all wildcard", Some(&["*"]), "any/model", true),
		(
			"any pattern may match",
			Some(&["gpt-4o", "anthropic/*"]),
			"anthropic/claude",
			true,
		),
		(
			"matching is case sensitive",
			Some(&["GPT-*"]),
			"gpt-4o",
			false,
		),
	];

	for (name, patterns, model, want) in cases {
		let allowed = AllowedModels::compile(
			patterns.map(|patterns| patterns.iter().map(|pattern| pattern.to_string()).collect()),
		)
		.unwrap();
		assert_eq!(allowed.allows(model), *want, "{name}");
	}
}

#[test]
fn test_allowed_models_discovery_intersections() {
	#[allow(clippy::type_complexity)]
	let cases: &[(&str, Option<&[&str]>, &str, &[&str])] = &[
		(
			"omitted preserves configured pattern",
			None,
			"provider/*",
			&["provider/*"],
		),
		("empty discovers nothing", Some(&[]), "provider/*", &[]),
		(
			"exact model is advertised through configured pattern",
			Some(&["provider/model"]),
			"provider/*",
			&["provider/model"],
		),
		(
			"nonmatching exact is hidden",
			Some(&["other/model"]),
			"provider/*",
			&[],
		),
		(
			"disjoint prefixes are hidden",
			Some(&["other/*"]),
			"provider/*",
			&[],
		),
		(
			"disjoint suffixes are hidden",
			Some(&["*-preview"]),
			"*-latest",
			&[],
		),
		(
			"narrower allowed pattern includes configured pattern",
			Some(&["provider/gpt-*"]),
			"provider/*",
			&["provider/*"],
		),
		(
			"narrower configured pattern is preserved",
			Some(&["provider/*"]),
			"provider/gpt-*",
			&["provider/gpt-*"],
		),
		(
			"overlapping prefix and suffix include configured pattern",
			Some(&["provider/*"]),
			"*-latest",
			&["*-latest"],
		),
		(
			"multiple exact models are advertised and deduplicated",
			Some(&["provider/one", "provider/two", "provider/one"]),
			"provider/*",
			&["provider/one", "provider/two"],
		),
		(
			"concrete configured models are filtered",
			Some(&["provider/*"]),
			"other/model",
			&[],
		),
	];

	for (name, patterns, configured, want) in cases {
		let allowed = AllowedModels::compile(
			patterns.map(|patterns| patterns.iter().map(|pattern| pattern.to_string()).collect()),
		)
		.unwrap();
		let policy = ModelAccessPolicy {
			allowed_models: allowed,
		};
		assert_eq!(
			discoverable_models(Some(&policy), configured).collect::<Vec<_>>(),
			*want,
			"{name}"
		);
	}
}

#[test]
fn test_allowed_models_rejects_invalid_patterns() {
	let cases = [
		(vec![""], "empty model name"),
		(vec!["provider/*/model"], "at the beginning or end"),
		(vec!["**"], "at most one wildcard"),
		(vec!["*", "provider/model"], "cannot combine '*'"),
	];

	for (patterns, want) in cases {
		let err =
			AllowedModels::compile(Some(patterns.into_iter().map(str::to_string).collect())).unwrap_err();
		assert!(
			err.to_string().contains(want),
			"error {err:?} did not contain {want:?}"
		);
	}
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
async fn test_apikey_sha256_extracts_and_strips() {
	let local: LocalAPIKeys = serde_json::from_value(serde_json::json!({
		"keys": [
			{
				"keyHash": "sha256:4C806362B613F7496ABF284146EFD31DA90E4B16169FE001841CA17290F427C4",
				"metadata": {"group": "eng"}
			},
			{
				"key": "plaintext-api-key",
				"metadata": {"group": "sales"}
			}
		],
		"mode": "strict"
	}))
	.expect("mixed API key config should deserialize");
	let auth = local.into();

	let mut req = ::http::Request::builder()
		.header(crate::http::header::AUTHORIZATION, "Bearer test-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("hashed API key should validate");

	let claims = req
		.extensions()
		.get::<Claims>()
		.expect("claims should be inserted");
	assert_eq!(claims.metadata["group"], "eng");
	let expr = crate::cel::Expression::new_strict("apiKey.key.unredacted()").unwrap();
	assert_eq!(
		crate::cel::Executor::new_request(&req)
			.eval(&expr)
			.unwrap()
			.json()
			.unwrap(),
		serde_json::json!("test-api-key")
	);
	assert!(
		req
			.headers()
			.get(crate::http::header::AUTHORIZATION)
			.is_none()
	);

	let mut req = ::http::Request::builder()
		.header(
			crate::http::header::AUTHORIZATION,
			"Bearer plaintext-api-key",
		)
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("plaintext API key should validate");

	let claims = req
		.extensions()
		.get::<Claims>()
		.expect("claims should be inserted");
	assert_eq!(claims.metadata["group"], "sales");
	let expr = crate::cel::Expression::new_strict("apiKey.key.unredacted()").unwrap();
	assert_eq!(
		crate::cel::Executor::new_request(&req)
			.eval(&expr)
			.unwrap()
			.json()
			.unwrap(),
		serde_json::json!("plaintext-api-key")
	);
	assert!(
		req
			.headers()
			.get(crate::http::header::AUTHORIZATION)
			.is_none()
	);
}

#[test]
fn test_apikey_sha256_serializes_with_prefix() {
	let hash: APIKeyHash = serde_json::from_value(serde_json::json!(
		"sha256:4C806362B613F7496ABF284146EFD31DA90E4B16169FE001841CA17290F427C4"
	))
	.expect("sha256 API key hash should deserialize");

	assert_eq!(
		serde_json::to_value(hash).expect("sha256 API key hash should serialize"),
		serde_json::json!("sha256:4c806362b613f7496abf284146efd31da90e4b16169fe001841ca17290f427c4")
	);
}

#[tokio::test]
async fn test_apikey_sha256_rejects_invalid_key() {
	let local: LocalAPIKeys = serde_json::from_value(serde_json::json!({
		"keys": [{
			"keyHash": "sha256:4c806362b613f7496abf284146efd31da90e4b16169fe001841ca17290f427c4"
		}],
		"mode": "strict"
	}))
	.expect("sha256 API key config should deserialize");
	let auth = local.into();

	let mut req = ::http::Request::builder()
		.header(crate::http::header::AUTHORIZATION, "Bearer invalid-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	let err = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect_err("invalid API key should be rejected");

	assert!(matches!(
		err,
		ProxyResponse::Error(ProxyError::APIKeyAuthenticationFailure(
			Error::InvalidCredentials
		))
	));
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
