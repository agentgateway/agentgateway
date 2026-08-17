//! Unit tests for RFC 7662 Token Introspection.

use super::*;

#[test]
fn hash_token_produces_deterministic_output() {
	let h1 = hash_token("test-token");
	let h2 = hash_token("test-token");
	assert_eq!(h1, h2);
	assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
}

#[test]
fn hash_token_differs_for_different_tokens() {
	let h1 = hash_token("token-a");
	let h2 = hash_token("token-b");
	assert_ne!(h1, h2);
}

#[test]
fn introspection_response_to_claims_active() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: Some("user123".to_string()),
		aud: Some(Value::String("my-api".to_string())),
		exp: Some(9999999999),
		iat: Some(1000000000),
		nbf: None,
		scope: Some("read write".to_string()),
		client_id: Some("my-client".to_string()),
		username: None,
		token_type: Some("bearer".to_string()),
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: Some("https://idp.example.com/introspect".to_string()),
		client_id: "gw-client".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec!["my-api".to_string()],
	};

	let claims = response_to_claims(resp, "opaque-token", &config).expect("should succeed");
	assert_eq!(
		claims.inner.get("iss").unwrap().as_str().unwrap(),
		"https://idp.example.com"
	);
	assert_eq!(
		claims.inner.get("sub").unwrap().as_str().unwrap(),
		"user123"
	);
	assert_eq!(
		claims.inner.get("scope").unwrap().as_str().unwrap(),
		"read write"
	);
	assert_eq!(
		claims.inner.get("client_id").unwrap().as_str().unwrap(),
		"my-client"
	);
}

#[test]
fn introspection_response_inactive_rejected() {
	let resp = IntrospectionResponse {
		active: false,
		iss: None,
		sub: None,
		aud: None,
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec![],
	};

	let err = response_to_claims(resp, "token", &config).unwrap_err();
	assert!(matches!(err, IntrospectionError::Inactive));
}

#[test]
fn introspection_response_issuer_mismatch() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://wrong-issuer.com".to_string()),
		sub: None,
		aud: None,
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://expected.com".to_string(),
		expected_audiences: vec![],
	};

	let err = response_to_claims(resp, "token", &config).unwrap_err();
	assert!(matches!(err, IntrospectionError::IssuerMismatch { .. }));
}

#[test]
fn introspection_response_missing_issuer_uses_configured() {
	let resp = IntrospectionResponse {
		active: true,
		iss: None, // IdP didn't return iss
		sub: Some("user1".to_string()),
		aud: None,
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://expected.com".to_string(),
		expected_audiences: vec![],
	};

	let claims = response_to_claims(resp, "token", &config).expect("should fill iss from config");
	assert_eq!(
		claims.inner.get("iss").unwrap().as_str().unwrap(),
		"https://expected.com"
	);
}

#[test]
fn introspection_response_audience_mismatch() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: None,
		aud: Some(Value::String("wrong-aud".to_string())),
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec!["expected-aud".to_string()],
	};

	let err = response_to_claims(resp, "token", &config).unwrap_err();
	assert!(matches!(err, IntrospectionError::AudienceMismatch));
}

#[test]
fn introspection_response_no_audiences_configured_skips_validation() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: None,
		aud: Some(Value::String("any-aud".to_string())),
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec![], // No audiences configured → skip
	};

	response_to_claims(resp, "token", &config).expect("should pass when no audiences configured");
}

#[test]
fn introspection_response_expired_token() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: None,
		aud: None,
		exp: Some(1), // Way in the past
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec![],
	};

	let err = response_to_claims(resp, "token", &config).unwrap_err();
	assert!(matches!(err, IntrospectionError::Expired));
}

#[test]
fn introspection_response_not_yet_valid() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: None,
		aud: None,
		exp: Some(9999999999),
		iat: None,
		nbf: Some(9999999998), // Way in the future
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec![],
	};

	let err = response_to_claims(resp, "token", &config).unwrap_err();
	assert!(matches!(err, IntrospectionError::NotYetValid));
}

#[test]
fn introspection_response_array_audience() {
	let resp = IntrospectionResponse {
		active: true,
		iss: Some("https://idp.example.com".to_string()),
		sub: None,
		aud: Some(Value::Array(vec![
			Value::String("aud1".to_string()),
			Value::String("aud2".to_string()),
		])),
		exp: None,
		iat: None,
		nbf: None,
		scope: None,
		client_id: None,
		username: None,
		token_type: None,
		jti: None,
		extra: Default::default(),
	};

	let config = IntrospectionConfig {
		endpoint: None,
		client_id: "gw".to_string(),
		client_secret: None,
		cache_duration: Duration::from_secs(30),
		timeout: Duration::from_secs(5),
		failure_mode: FailureMode::FailClosed,
		expected_issuer: "https://idp.example.com".to_string(),
		expected_audiences: vec!["aud2".to_string()],
	};

	response_to_claims(resp, "token", &config).expect("should match any audience in array");
}

#[tokio::test]
async fn cache_insert_and_get() {
	let cache = IntrospectionCache::new(std::time::Duration::from_secs(60));
	let claims = Claims {
		inner: Default::default(),
		jwt: secrecy::SecretString::new("test".into()),
	};
	cache.insert("my-token", claims.clone()).await;
	let cached = cache.get("my-token").await;
	assert!(cached.is_some());
}

#[tokio::test]
async fn cache_miss_for_unknown_token() {
	let cache = IntrospectionCache::new(std::time::Duration::from_secs(60));
	let cached = cache.get("unknown-token").await;
	assert!(cached.is_none());
}

#[tokio::test]
async fn cache_expired_returns_none() {
	let cache = IntrospectionCache::new(std::time::Duration::from_millis(1));
	let claims = Claims {
		inner: Default::default(),
		jwt: secrecy::SecretString::new("test".into()),
	};
	cache.insert("my-token", claims).await;
	tokio::time::sleep(std::time::Duration::from_millis(10)).await;
	let cached = cache.get("my-token").await;
	assert!(cached.is_none());
}
