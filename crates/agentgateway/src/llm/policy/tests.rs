use ::http::{HeaderName, HeaderValue};

use super::*;

#[test]
fn test_get_webhook_forward_headers() {
	let mut headers = HeaderMap::new();
	headers.insert("x-test-header", HeaderValue::from_static("test-value"));
	headers.insert(
		"x-another-header",
		HeaderValue::from_static("another-value"),
	);
	headers.insert(
		"x-regex-header",
		HeaderValue::from_static("regex-match-123"),
	);

	let header_matches = vec![
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-test-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("test-value")),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-another-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("wrong-value")),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-regex-header")),
			value: HeaderValueMatch::Regex(regex::Regex::new(r"regex-match-\d+").unwrap()),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-missing-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("some-value")),
		},
	];

	let result = Policy::get_webhook_forward_headers(&headers, &header_matches);

	assert_eq!(result.len(), 2);
	assert_eq!(
		result.get("x-test-header").unwrap(),
		&HeaderValue::from_static("test-value")
	);
	assert_eq!(
		result.get("x-regex-header").unwrap(),
		&HeaderValue::from_static("regex-match-123")
	);
}

#[test]
fn test_rejection_with_json_headers() {
	let rejection = RequestRejection {
		body: Bytes::from(r#"{"error": {"message": "test", "type": "invalid_request_error"}}"#),
		status: StatusCode::BAD_REQUEST,
		headers: Some(HeaderModifier {
			set: vec![
				(strng::new("content-type"), strng::new("application/json")),
				(strng::new("x-custom-header"), strng::new("custom-value")),
			],
			add: vec![],
			remove: vec![],
		}),
	};

	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	assert_eq!(
		response.headers().get("content-type").unwrap(),
		"application/json"
	);
	assert_eq!(
		response.headers().get("x-custom-header").unwrap(),
		"custom-value"
	);
}

#[test]
fn test_rejection_add_multiple_header_values() {
	let rejection = RequestRejection {
		body: Bytes::from("blocked"),
		status: StatusCode::FORBIDDEN,
		headers: Some(HeaderModifier {
			set: vec![],
			add: vec![
				(strng::new("x-blocked-category"), strng::new("violence")),
				(strng::new("x-blocked-category"), strng::new("hate")),
			],
			remove: vec![],
		}),
	};

	let response = rejection.as_response();
	let values: Vec<_> = response
		.headers()
		.get_all("x-blocked-category")
		.iter()
		.map(|v| v.to_str().unwrap())
		.collect();
	assert_eq!(values, vec!["violence", "hate"]);
}

#[test]
fn test_rejection_backwards_compatibility() {
	// Simulate old config without headers field
	let rejection = RequestRejection {
		body: Bytes::from("error message"),
		status: StatusCode::FORBIDDEN,
		headers: None,
	};

	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	// Should have no extra headers
	assert!(response.headers().is_empty());
}

#[test]
fn test_rejection_default() {
	let rejection = RequestRejection::default();
	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	assert!(response.headers().is_empty());
}

#[test]
fn test_rejection_set_and_remove_headers() {
	let rejection = RequestRejection {
		body: Bytes::from("test"),
		status: StatusCode::BAD_REQUEST,
		headers: Some(HeaderModifier {
			set: vec![(strng::new("content-type"), strng::new("application/json"))],
			add: vec![],
			remove: vec![strng::new("server")],
		}),
	};

	let response = rejection.as_response();
	assert_eq!(
		response.headers().get("content-type").unwrap(),
		"application/json"
	);
	assert!(response.headers().get("server").is_none());
}

#[test]
fn test_prompt_caching_policy_deserialization() {
	use serde_json::json;

	let json = json!({
		"promptCaching": {
			"cacheSystem": true,
			"cacheMessages": true,
			"cacheTools": false,
			"minTokens": 1024
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system);
	assert!(caching.cache_messages);
	assert!(!caching.cache_tools);
	assert_eq!(caching.min_tokens, Some(1024));
}

#[test]
fn test_prompt_caching_policy_defaults() {
	use serde_json::json;

	// Empty config should have system and messages enabled by default
	let json = json!({
		"promptCaching": {}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system); // Default: true
	assert!(caching.cache_messages); // Default: true
	assert!(!caching.cache_tools); // Default: false
	assert_eq!(caching.min_tokens, Some(1024)); // Default: 1024
}

#[test]
fn test_policy_without_prompt_caching_field() {
	use serde_json::json;

	let json = json!({
		"modelAliases": {
			"gpt-4": "anthropic.claude-3-sonnet-20240229-v1:0"
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// prompt_caching should be None when not specified
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_prompt_caching_explicit_disable() {
	use serde_json::json;

	// Explicitly disable caching
	let json = json!({
		"promptCaching": null
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// Should be None when explicitly set to null
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_resolve_route() {
	let mut routes = IndexMap::new();
	routes.insert(
		strng::literal!("/completions"),
		crate::llm::RouteType::Completions,
	);
	routes.insert(
		strng::literal!("/v1/messages"),
		crate::llm::RouteType::Messages,
	);
	routes.insert(
		strng::literal!("/v1/embeddings"),
		crate::llm::RouteType::Embeddings,
	);
	routes.insert(strng::literal!("*"), crate::llm::RouteType::Passthrough);

	let policy = Policy {
		routes,
		..Default::default()
	};

	// Suffix matching
	assert_eq!(
		policy.resolve_route("/v1/chat/completions"),
		crate::llm::RouteType::Completions
	);
	assert_eq!(
		policy.resolve_route("/api/completions"),
		crate::llm::RouteType::Completions
	);
	// Exact suffix match
	assert_eq!(
		policy.resolve_route("/v1/messages"),
		crate::llm::RouteType::Messages
	);
	// Embeddings route
	assert_eq!(
		policy.resolve_route("/v1/embeddings"),
		crate::llm::RouteType::Embeddings
	);
	// Wildcard fallback
	assert_eq!(
		policy.resolve_route("/v1/models"),
		crate::llm::RouteType::Passthrough
	);
	// Empty routes defaults to Completions
	assert_eq!(
		Policy::default().resolve_route("/any/path"),
		crate::llm::RouteType::Completions
	);
}

#[test]
fn test_model_alias_wildcard_resolution() {
	let mut policy = Policy {
		model_aliases: HashMap::from([
			(strng::new("gpt-4"), strng::new("exact-target")),
			(
				strng::new("claude-haiku-3.5-*"),
				strng::new("haiku-3.5-target"),
			),
			(strng::new("claude-haiku-*"), strng::new("haiku-target")),
			(strng::new("*-sonnet-*"), strng::new("sonnet-target")),
		]),
		..Default::default()
	};

	policy.compile_model_alias_patterns();

	// Exact match takes precedence over wildcards
	assert_eq!(
		policy.resolve_model_alias("gpt-4"),
		Some(&strng::new("exact-target"))
	);

	// Longer patterns are more specific (checked first)
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-3.5-v1"),
		Some(&strng::new("haiku-3.5-target")) // Matches "claude-haiku-3.5-*" not "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-v1"),
		Some(&strng::new("haiku-target")) // Only matches "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("other-sonnet-model"),
		Some(&strng::new("sonnet-target")) // Matches "*-sonnet-*"
	);

	// No match returns None
	assert_eq!(policy.resolve_model_alias("unmatched-model"), None);
}

#[test]
fn test_model_alias_pattern_validation() {
	// Pattern must contain wildcard
	assert!(ModelAliasPattern::from_wildcard("no-wildcards").is_err());

	// Special characters are escaped (dot is literal, not regex wildcard)
	let pattern = ModelAliasPattern::from_wildcard("test.*").unwrap();
	assert!(pattern.matches("test.v1"));
	assert!(!pattern.matches("testXv1")); // X doesn't match literal dot
}
