use secrecy::SecretString;
use serde_json::Map;

use super::*;
use crate::http::jwt::Claims;
use crate::llm::bedrock::AwsRegion;
use crate::test_helpers::proxymock::setup_proxy_test;

#[test]
fn test_aws_auth_deserializes_assume_role() {
	let implicit: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/backend"
		}
	}))
	.expect("implicit AWS assume role auth should deserialize");
	assert!(
		matches!(
			implicit,
			AwsAuth::Implicit {
				assume_role: Some(_),
				..
			}
		),
		"expected implicit AWS auth with assume role"
	);
}

#[test]
fn test_aws_auth_deserializes_assume_role_with_session_name_and_tags() {
	let implicit: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"sessionName": "acme-payments-invoice-processor",
			"tags": [
				{"key": "Team", "value": "acme-payments"},
				{"key": "App", "value": "invoice-processor"}
			]
		}
	}))
	.expect("should deserialize assume role with session name and tags");
	match implicit {
		AwsAuth::Implicit {
			assume_role: Some(ar),
			..
		} => {
			assert_eq!(ar.role_arn, "arn:aws:iam::123456789012:role/my-role");
			// A plain string keeps its existing (static) meaning.
			assert_eq!(
				ar.session_name,
				Some(aws::AwsSessionName::Static(
					"acme-payments-invoice-processor".to_string()
				))
			);
			// Tags are stored sorted by key, regardless of configured order.
			assert_eq!(
				ar.tags.static_tags().as_ref(),
				&[
					("App".to_string(), "invoice-processor".to_string()),
					("Team".to_string(), "acme-payments".to_string()),
				]
			);
		},
		_ => panic!("expected implicit AWS auth with assume role"),
	}
}

#[test]
fn test_aws_auth_deserializes_dynamic_session_tags() {
	let implicit: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"tags": [
				{"key": "Team", "value": "acme-payments"},
				{"key": "App", "expression": "request.headers[\"x-app\"]"},
				{"key": "User", "expression": "jwt.sub"}
			]
		}
	}))
	.expect("should deserialize mixed static and dynamic tags");
	match implicit {
		AwsAuth::Implicit {
			assume_role: Some(ar),
			..
		} => {
			assert!(ar.tags.has_dynamic());
			assert_eq!(
				ar.tags.static_tags().as_ref(),
				&[("Team".to_string(), "acme-payments".to_string())]
			);
			let expressions: Vec<&str> = ar
				.tags
				.expressions()
				.map(|e| e.original_expression.as_str())
				.collect();
			// Dynamic tags are sorted by key: App before User.
			assert_eq!(expressions, vec![r#"request.headers["x-app"]"#, "jwt.sub"]);
		},
		_ => panic!("expected implicit AWS auth with assume role"),
	}
}

#[test]
fn test_aws_session_tags_round_trip_through_serialization() {
	let source = serde_json::json!({
		"roleArn": "arn:aws:iam::123456789012:role/my-role",
		"tags": [
			{"key": "Team", "value": "acme-payments"},
			{"key": "App", "expression": "request.headers[\"x-app\"]"}
		]
	});
	let ar: AwsAssumeRole = serde_json::from_value(source).expect("should deserialize");
	let serialized = serde_json::to_value(&ar).expect("should serialize");
	let round_tripped: AwsAssumeRole =
		serde_json::from_value(serialized).expect("serialized form should deserialize");
	assert_eq!(ar.tags, round_tripped.tags);
}

#[test]
fn test_aws_auth_deserializes_dynamic_session_name() {
	let implicit: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"sessionName": {"expression": "jwt.sub"}
		}
	}))
	.expect("should deserialize dynamic session name");
	match implicit {
		AwsAuth::Implicit {
			assume_role: Some(ar),
			..
		} => match ar.session_name {
			Some(aws::AwsSessionName::Dynamic { expression }) => {
				assert_eq!(expression.original_expression.as_str(), "jwt.sub");
			},
			other => panic!("expected dynamic session name, got {other:?}"),
		},
		_ => panic!("expected implicit AWS auth with assume role"),
	}
}

#[test]
fn test_aws_session_name_round_trips_through_serialization() {
	for source in [
		// Static form serializes back to a plain string.
		serde_json::json!({
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"sessionName": "acme-payments"
		}),
		// Dynamic form serializes back to {expression: ...}.
		serde_json::json!({
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"sessionName": {"expression": "jwt.sub"}
		}),
	] {
		let ar: AwsAssumeRole = serde_json::from_value(source.clone()).expect("should deserialize");
		let serialized = serde_json::to_value(&ar).expect("should serialize");
		assert_eq!(
			serialized.get("sessionName"),
			source.get("sessionName"),
			"sessionName should round-trip in its original form"
		);
	}
}

#[test]
fn test_aws_auth_cel_expressions_include_tags_and_session_name() {
	let auth: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"sessionName": {"expression": "jwt.sub"},
			"tags": [
				{"key": "App", "expression": "request.headers[\"x-app\"]"}
			]
		}
	}))
	.expect("should deserialize");
	let expressions: Vec<&str> = auth
		.cel_expressions()
		.map(|e| e.original_expression.as_str())
		.collect();
	assert_eq!(
		expressions,
		vec![r#"request.headers["x-app"]"#, "jwt.sub"],
		"both tag and session name expressions must register with the CEL context"
	);
}

#[test]
fn test_aws_session_name_rejects_invalid_expression() {
	let result = serde_json::from_value::<AwsAssumeRole>(serde_json::json!({
		"roleArn": "arn:aws:iam::123456789012:role/my-role",
		"sessionName": {"expression": "this is not cel ("}
	}));
	assert!(result.is_err(), "invalid CEL expression should be rejected");
}

#[test]
fn test_aws_session_tag_rejects_invalid_configs() {
	for (name, tags) in [
		(
			"both value and expression",
			serde_json::json!([{"key": "App", "value": "x", "expression": "jwt.sub"}]),
		),
		(
			"neither value nor expression",
			serde_json::json!([{"key": "App"}]),
		),
		(
			"duplicate keys",
			serde_json::json!([
				{"key": "App", "value": "x"},
				{"key": "App", "expression": "jwt.sub"}
			]),
		),
		(
			"invalid CEL expression",
			serde_json::json!([{"key": "App", "expression": "this is not cel ("}]),
		),
		(
			"static value with invalid characters",
			serde_json::json!([{"key": "App", "value": "a,b"}]),
		),
		("empty key", serde_json::json!([{"key": "", "value": "x"}])),
	] {
		let result = serde_json::from_value::<AwsAssumeRole>(serde_json::json!({
			"roleArn": "arn:aws:iam::123456789012:role/my-role",
			"tags": tags,
		}));
		assert!(result.is_err(), "{name} should be rejected");
	}
}

#[test]
fn test_aws_session_tags_rejects_more_than_sts_limit() {
	let tags: Vec<serde_json::Value> = (0..51)
		.map(|i| serde_json::json!({"key": format!("Key{i}"), "value": "x"}))
		.collect();
	let result = serde_json::from_value::<AwsAssumeRole>(serde_json::json!({
		"roleArn": "arn:aws:iam::123456789012:role/my-role",
		"tags": tags,
	}));
	assert!(result.is_err(), "more than 50 tags should be rejected");
}

#[test]
fn test_aws_auth_assume_role_defaults_session_name_and_tags() {
	let implicit: AwsAuth = serde_json::from_value(serde_json::json!({
		"assumeRole": {
			"roleArn": "arn:aws:iam::123456789012:role/backend"
		}
	}))
	.expect("assume role without session name or tags should deserialize");
	match implicit {
		AwsAuth::Implicit {
			assume_role: Some(ar),
			..
		} => {
			assert_eq!(ar.session_name, None);
			assert!(ar.tags.is_empty());
		},
		_ => panic!("expected implicit AWS auth with assume role"),
	}
}

#[test]
fn test_authorization_location_expression_extracts_from_cel() {
	let req = ::http::Request::builder()
		.uri("http://example.com/")
		.header("x-token", "from-cel")
		.body(crate::http::Body::empty())
		.unwrap();
	let location = AuthorizationLocation::Expression(std::sync::Arc::new(
		crate::cel::Expression::new_strict(r#"request.headers["x-token"]"#).unwrap(),
	));

	assert_eq!(location.extract(&req).as_deref(), Some("from-cel"));
}

#[test]
fn test_authorization_location_expression_deserializes_flat_expression() {
	let location: AuthorizationLocation =
		crate::serdes::yamlviajson::from_str(r#"expression: 'request.headers["authorization"]'"#)
			.expect("expression location should deserialize");

	let expression = location
		.expression()
		.expect("location should contain an expression");
	assert_eq!(
		expression.original_expression,
		r#"request.headers["authorization"]"#
	);
}

#[test]
fn test_authorization_location_expression_cannot_insert() {
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let location = AuthorizationLocation::Expression(std::sync::Arc::new(
		crate::cel::Expression::new_strict(r#""token""#).unwrap(),
	));

	let err = location.insert(&mut req, "token").unwrap_err();
	assert!(
		err
			.to_string()
			.contains("only supported for credential extraction")
	);
}

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
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};
	apply_backend_auth(
		&backend_info,
		&BackendAuth::Passthrough { location: None },
		&mut req,
	)
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

#[tokio::test]
async fn test_backend_auth_key() {
	// Test Key authentication
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let backend_info = BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};

	let key_auth = BackendAuth::Key {
		value: SecretString::new("my-secret-key".into()),
		location: None,
	};
	apply_backend_auth(&backend_info, &key_auth, &mut req)
		.await
		.expect("apply backend auth");

	let auth = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header must be set");
	assert_eq!(auth.to_str().unwrap(), "Bearer my-secret-key");
	assert!(auth.is_sensitive());
}

#[tokio::test]
async fn test_backend_auth_key_query_parameter() {
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "http://example.com/search?keep=yes&key=old"
		.parse()
		.unwrap();
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let backend_info = BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};

	let key_auth = BackendAuth::Key {
		value: SecretString::new("my-secret-key".into()),
		location: Some(AuthorizationLocation::QueryParameter { name: "key".into() }),
	};
	apply_backend_auth(&backend_info, &key_auth, &mut req)
		.await
		.expect("apply backend auth");

	assert_eq!(
		req.uri().to_string(),
		"http://example.com/search?keep=yes&key=my-secret-key"
	);
}

#[tokio::test]
async fn test_backend_auth_key_default_sets_non_explicit_extension() {
	// When location is None (defaulted), the extension must have explicit=false.
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let backend_info = BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};

	let key_auth = BackendAuth::Key {
		value: SecretString::new("my-secret-key".into()),
		location: None,
	};
	apply_backend_auth(&backend_info, &key_auth, &mut req)
		.await
		.expect("apply backend auth");

	let ext = req
		.extensions()
		.get::<AppliedBackendAuthLocation>()
		.expect("extension must be set");
	assert!(
		!ext.explicit,
		"default location must not be marked explicit"
	);
}

#[tokio::test]
async fn test_backend_auth_key_explicit_location_sets_explicit_extension() {
	// When location is Some(...), the extension must have explicit=true.
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let backend_info = BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};

	let key_auth = BackendAuth::Key {
		value: SecretString::new("my-secret-key".into()),
		location: Some(AuthorizationLocation::bearer_header()),
	};
	apply_backend_auth(&backend_info, &key_auth, &mut req)
		.await
		.expect("apply backend auth");

	let ext = req
		.extensions()
		.get::<AppliedBackendAuthLocation>()
		.expect("extension must be set");
	assert!(ext.explicit, "explicit location must be marked explicit");
}

#[tokio::test]
async fn test_aws_sign_request_explicit_region() {
	// Test AWS signing with explicit region in config
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-runtime.us-west-2.amazonaws.com/model/invoke"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;

	let aws_auth = AwsAuth::ExplicitConfig {
		access_key_id: SecretString::new("AKIAIOSFODNN7EXAMPLE".into()),
		secret_access_key: SecretString::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into()),
		region: Some("us-west-2".to_string()),
		session_token: None,
		service_name: None,
	};

	// No default region in request extensions.

	// Should use the explicit region and attempt signing
	// Will fail on credentials but should not fail on region
	aws::sign_request(&mut req, &aws_auth)
		.await
		.expect("signing failed");
	// get the signature header
	let auth = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header must be set");

	// Part 2
	// now, repeat with adefault region to make sure explicit region takes precedence
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-runtime.us-west-2.amazonaws.com/model/invoke"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;

	// Insert default AwsRegion into request extensions
	req.extensions_mut().insert(AwsRegion {
		region: "eu-central-1".to_string(),
	});

	// Should use the explicit region and attempt signing
	// Will fail on credentials but should not fail on region
	aws::sign_request(&mut req, &aws_auth)
		.await
		.expect("signing failed");
	// get the signature header
	let auth2 = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header must be set");

	assert_eq!(auth, auth2, "Signatures should match with explicit region");
}

#[tokio::test]
async fn test_aws_sign_requestallback() {
	// Test AWS signing falls back tohen not specified in config
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-runtime.eu-west-1.amazonaws.com/model/invoke"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;

	let aws_auth = AwsAuth::ExplicitConfig {
		access_key_id: SecretString::new("AKIAIOSFODNN7EXAMPLE".into()),
		secret_access_key: SecretString::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into()),
		region: None, // No region in config
		session_token: None,
		service_name: None,
	};

	// Insert default AwsRegion into request extensions
	req.extensions_mut().insert(AwsRegion {
		region: "eu-west-1".to_string(),
	});

	// Should use the default region in the extension
	aws::sign_request(&mut req, &aws_auth)
		.await
		.expect("signing failed");
}

#[tokio::test(start_paused = true)]
async fn test_aws_sign_request_no_region_error() {
	unsafe {
		// prevent loading from default profile on developer's laptops, so this test passes consistently.
		std::env::set_var("AWS_PROFILE", "/dev/null");
	}

	// Test AWS signing fails with clear error when no region available
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-runtime.amazonaws.com/model/invoke"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;

	let aws_auth = AwsAuth::ExplicitConfig {
		access_key_id: SecretString::new("AKIAIOSFODNN7EXAMPLE".into()),
		secret_access_key: SecretString::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into()),
		region: None, // No region in config
		session_token: None,
		service_name: None,
	};

	// No default region in request extensions.

	// Should fail with specific "Region must be specified" error
	let result = aws::sign_request(&mut req, &aws_auth).await;
	assert!(result.is_err(), "Should fail without region");

	let err = result.unwrap_err().to_string();
	assert!(
		err.contains("No region found in AWS config or request extensions"),
		"Error should mention missing region, got: {}",
		err
	);
}

#[tokio::test]
async fn test_aws_sign_request_implicit_with_extension() {
	// Test AWS signing with implicit auth uses region from request extensions
	// Set temporary AWS credentials in environment for test consistency
	let _env_guard = crate::config::lock_env_for_tests_async().await;
	unsafe {
		std::env::set_var("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE");
		std::env::set_var(
			"AWS_SECRET_ACCESS_KEY",
			"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
		);
	}

	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-runtime.ap-southeast-1.amazonaws.com/model/invoke"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;

	// Insert AwsRegion into request extensions
	req.extensions_mut().insert(AwsRegion {
		region: "ap-southeast-1".to_string(),
	});

	let aws_auth = AwsAuth::Implicit {
		service_name: None,
		region: None,
		assume_role: None,
		source_credentials_cache: Default::default(),
		assume_role_cache: Default::default(),
	};

	// Should use region from request extensions
	let result = aws::sign_request(&mut req, &aws_auth).await;

	// Clean up environment variables
	unsafe {
		std::env::remove_var("AWS_ACCESS_KEY_ID");
		std::env::remove_var("AWS_SECRET_ACCESS_KEY");
	}

	result.expect("signing failed");
}

#[tokio::test]
async fn test_aws_sign_request_implicit_configured_region_wins() {
	// A region configured on the auth policy beats the typed-backend region
	// extension (and thus also the ambient AWS region it falls back to).
	// AWS_REGION is deliberately not set here: the ambient SDK config is
	// process-global and cached, so setting it would leak into other tests.
	let _env_guard = crate::config::lock_env_for_tests_async().await;
	unsafe {
		std::env::set_var("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE");
		std::env::set_var(
			"AWS_SECRET_ACCESS_KEY",
			"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
		);
	}

	let mut req = crate::http::Request::new(crate::http::Body::empty());
	*req.uri_mut() = "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/x/invocations"
		.parse()
		.unwrap();
	*req.method_mut() = http::Method::POST;
	req.extensions_mut().insert(AwsRegion {
		region: "us-east-2".to_string(),
	});

	let aws_auth = AwsAuth::Implicit {
		service_name: Some("bedrock-agentcore".to_string()),
		region: Some("us-east-1".to_string()),
		assume_role: None,
		source_credentials_cache: Default::default(),
		assume_role_cache: Default::default(),
	};

	let result = aws::sign_request(&mut req, &aws_auth).await;

	unsafe {
		std::env::remove_var("AWS_ACCESS_KEY_ID");
		std::env::remove_var("AWS_SECRET_ACCESS_KEY");
	}

	result.expect("signing failed");
	let authz = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header")
		.to_str()
		.unwrap();
	assert!(
		authz.contains("/us-east-1/bedrock-agentcore/"),
		"credential scope must use the configured region: {authz}"
	);
}

#[test]
fn extract_subject_token_falls_back_to_claims_for_authorization_header() {
	// Default source is the Authorization Bearer header; a JWT policy stripped it,
	// leaving only the Claims extension.
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});

	let token = oauth::extract_subject_token(&AuthorizationLocation::default(), &req);
	assert_eq!(token.as_deref(), Some("claims-jwt"));
}

#[test]
fn extract_subject_token_uses_authorization_header_without_claims() {
	let req = ::http::Request::builder()
		.uri("http://example/")
		.header(::http::header::AUTHORIZATION, "Bearer header-tok")
		.body(crate::http::Body::empty())
		.unwrap();

	let token = oauth::extract_subject_token(&AuthorizationLocation::default(), &req);
	assert_eq!(token.as_deref(), Some("header-tok"));
}

#[test]
fn extract_subject_token_empty_authorization_header_falls_back_to_claims() {
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.header(::http::header::AUTHORIZATION, "Bearer ")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});

	let token = oauth::extract_subject_token(&AuthorizationLocation::default(), &req);
	assert_eq!(token.as_deref(), Some("claims-jwt"));
}

#[test]
fn extract_subject_token_empty_authorization_header_is_missing() {
	let req = ::http::Request::builder()
		.uri("http://example/")
		.header(::http::header::AUTHORIZATION, "Bearer ")
		.body(crate::http::Body::empty())
		.unwrap();

	let token = oauth::extract_subject_token(&AuthorizationLocation::default(), &req);
	assert_eq!(token, None);
}

#[test]
fn extract_subject_token_prefers_authorization_header_over_claims() {
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.header(::http::header::AUTHORIZATION, "Bearer header-tok")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});

	let token = oauth::extract_subject_token(&AuthorizationLocation::default(), &req);
	assert_eq!(token.as_deref(), Some("header-tok"));
}

#[test]
fn extract_subject_token_custom_source_prefers_configured_location_over_claims() {
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.header("x-subject", "custom-tok")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});
	let source = AuthorizationLocation::Header {
		name: ::http::HeaderName::from_static("x-subject"),
		prefix: None,
	};

	let token = oauth::extract_subject_token(&source, &req);
	assert_eq!(token.as_deref(), Some("custom-tok"));
}

#[test]
fn extract_subject_token_custom_header_falls_back_to_claims() {
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});
	let source = AuthorizationLocation::Header {
		name: ::http::HeaderName::from_static("x-subject"),
		prefix: None,
	};

	let token = oauth::extract_subject_token(&source, &req);
	assert_eq!(token.as_deref(), Some("claims-jwt"));
}

#[test]
fn extract_subject_token_expression_falls_back_to_claims() {
	let mut req = ::http::Request::builder()
		.uri("http://example/")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::from("claims-jwt"),
	});
	let source = AuthorizationLocation::Expression(std::sync::Arc::new(
		crate::cel::Expression::new_strict(r#"request.headers["x-subject"]"#).unwrap(),
	));

	let token = oauth::extract_subject_token(&source, &req);
	assert_eq!(token.as_deref(), Some("claims-jwt"));
}
