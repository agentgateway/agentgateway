use secrecy::SecretString;
use serde_json::Map;

use super::*;
use crate::http::jwt::Claims;
use crate::llm::bedrock::AwsRegion;
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
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
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

#[tokio::test]
async fn test_backend_auth_passthrough_prefers_explicit_bearer_token_extension() {
	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let inputs = t.inputs();

	let mut req = crate::http::Request::new(crate::http::Body::empty());
	req
		.extensions_mut()
		.insert(PassthroughBearerToken(SecretString::new(
			"oauth2-access-token".into(),
		)));
	req.extensions_mut().insert(Claims {
		inner: Map::new(),
		jwt: SecretString::new("id-token.jwt".into()),
	});

	let backend_info = BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs,
	};
	apply_backend_auth(&backend_info, &BackendAuth::Passthrough {}, &mut req)
		.await
		.expect("apply backend auth");

	let auth = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header must be set");
	assert_eq!(auth.to_str().unwrap(), "Bearer oauth2-access-token");
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

	let key_auth = BackendAuth::Key(SecretString::new("my-secret-key".into()));
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

	let aws_auth = AwsAuth::Implicit {};

	// Should use region from request extensions
	let result = aws::sign_request(&mut req, &aws_auth).await;

	// Clean up environment variables
	unsafe {
		std::env::remove_var("AWS_ACCESS_KEY_ID");
		std::env::remove_var("AWS_SECRET_ACCESS_KEY");
	}

	result.expect("signing failed");
}
