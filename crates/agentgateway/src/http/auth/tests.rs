use std::collections::HashMap;

use base64::Engine;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::idjag::IdentityAssertion;
use super::*;
use crate::http::jwt::Claims;
use crate::llm::bedrock::AwsRegion;
use crate::test_helpers::proxymock::{TestBind, setup_proxy_test};

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
fn test_authorization_location_expression_extracts_from_cel() {
	let req = ::http::Request::builder()
		.uri("http://example.com/")
		.header("x-token", "from-cel")
		.body(crate::http::Body::empty())
		.unwrap();
	let location = AuthorizationLocation::Expression {
		expression: std::sync::Arc::new(
			crate::cel::Expression::new_strict(r#"request.headers["x-token"]"#).unwrap(),
		),
	};

	assert_eq!(location.extract(&req).as_deref(), Some("from-cel"));
}

#[test]
fn test_authorization_location_expression_cannot_insert() {
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let location = AuthorizationLocation::Expression {
		expression: std::sync::Arc::new(crate::cel::Expression::new_strict(r#""token""#).unwrap()),
	};

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

// ---------------------------------------------------------------------------
// Identity Assertion (ID-JAG / Cross App Access) — BackendAuth::IdentityAssertion
// ---------------------------------------------------------------------------

// EC P-256 (ES256) private key used only for the private_key_jwt signing test.
const TEST_EC_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgltxBTVDLg7C6vE1T
7OtwJIZ/dpm8ygE2MBTjPCY3hgahRANCAARYzu50EeBrT0rELmTGroaGtn0zdjxL
1lOGr9fGw5wOGcXO0+Gn5F5sIxGyTM0FwnUHFNz2SoixZR5dtxhNc+Lo
-----END PRIVATE KEY-----
";

/// Build a request carrying an authenticated identity (a JWT + `sub` claim), as the
/// `jwtAuthentication` policy would leave it.
fn request_with_identity(sub: &str, raw_jwt: &str) -> crate::http::Request {
	let mut inner = Map::new();
	inner.insert("sub".into(), json!(sub));
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	req.extensions_mut().insert(Claims {
		inner,
		jwt: SecretString::new(raw_jwt.into()),
	});
	req
}

fn idjag_backend_info(t: &TestBind) -> BackendInfo {
	BackendInfo {
		call_target: Target::Address("0.0.0.0:80".parse().unwrap()),
		target: BackendTarget::Backend {
			name: Default::default(),
			namespace: Default::default(),
			section: None,
		},
		inputs: t.inputs(),
	}
}

fn idjag_config(idp: &MockServer, resource_as: &MockServer, idp_auth: Value) -> Value {
	json!({
		"idp": {
			"tokenEndpoint": format!("{}/token", idp.uri()),
			"clientId": "gw-idp",
			"clientAuth": idp_auth,
		},
		"resourceAs": {
			"tokenEndpoint": format!("{}/token", resource_as.uri()),
			"clientId": "gw-ras",
			"clientAuth": { "clientSecretPost": { "clientSecret": "ras-secret" } },
		},
		"audience": "https://resource-as.example/",
		"scope": "chat.read chat.history",
	})
}

/// Parse the last form-urlencoded request body received by a mock server into a map.
async fn last_form(server: &MockServer) -> HashMap<String, String> {
	let reqs = server.received_requests().await.expect("recording enabled");
	let last = reqs.last().expect("at least one request");
	serde_urlencoded::from_bytes(&last.body).expect("form-encoded body")
}

async fn mount_idp(server: &MockServer, expect: u64) {
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issued_token_type": "urn:ietf:params:oauth:token-type:id-jag",
			"access_token": "the-id-jag",
			"token_type": "N_A",
			"scope": "chat.read chat.history",
			"expires_in": 300,
		})))
		.expect(expect)
		.mount(server)
		.await;
}

async fn mount_resource_as(server: &MockServer, expect: u64) {
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"token_type": "Bearer",
			"access_token": "backend-access-token",
			"expires_in": 3600,
			"scope": "chat.read chat.history",
		})))
		.expect(expect)
		.mount(server)
		.await;
}

#[tokio::test]
async fn test_identity_assertion_happy_path_sends_spec_params() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;
	mount_idp(&idp, 1).await;
	mount_resource_as(&resource_as, 1).await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({ "clientSecretBasic": { "clientSecret": "idp-secret" } }),
	))
	.expect("config deserializes");

	let mut req = request_with_identity("user-1", "header.payload.signature");
	apply_backend_auth(
		&idjag_backend_info(&t),
		&BackendAuth::IdentityAssertion(Box::new(cfg)),
		&mut req,
	)
	.await
	.expect("apply backend auth");

	let auth = req
		.headers()
		.get(http::header::AUTHORIZATION)
		.expect("authorization header set");
	assert_eq!(auth.to_str().unwrap(), "Bearer backend-access-token");
	assert!(auth.is_sensitive());

	// Step 1 (IdP) carried the exact RFC 8693 / ID-JAG parameters.
	let idp_form = last_form(&idp).await;
	assert_eq!(
		idp_form.get("grant_type").map(String::as_str),
		Some("urn:ietf:params:oauth:grant-type:token-exchange")
	);
	assert_eq!(
		idp_form.get("requested_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:id-jag")
	);
	assert_eq!(
		idp_form.get("subject_token").map(String::as_str),
		Some("header.payload.signature")
	);
	assert_eq!(
		idp_form.get("subject_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:id_token")
	);
	assert_eq!(
		idp_form.get("audience").map(String::as_str),
		Some("https://resource-as.example/")
	);

	// Step 2 (resource AS) presented the ID-JAG as a JWT bearer assertion.
	let ras_form = last_form(&resource_as).await;
	assert_eq!(
		ras_form.get("grant_type").map(String::as_str),
		Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
	);
	assert_eq!(
		ras_form.get("assertion").map(String::as_str),
		Some("the-id-jag")
	);
	// The configured scope must be forwarded on the jwt-bearer leg too; the resource AS does
	// not default to the ID-JAG's scopes.
	assert_eq!(
		ras_form.get("scope").map(String::as_str),
		Some("chat.read chat.history")
	);
	// client_secret_post credentials on the resource AS leg.
	assert_eq!(
		ras_form.get("client_id").map(String::as_str),
		Some("gw-ras")
	);
	assert_eq!(
		ras_form.get("client_secret").map(String::as_str),
		Some("ras-secret")
	);
}

#[tokio::test]
async fn test_identity_assertion_second_call_is_cached() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;
	// expect exactly one call to each endpoint despite two apply() invocations.
	mount_idp(&idp, 1).await;
	mount_resource_as(&resource_as, 1).await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({ "clientSecretBasic": { "clientSecret": "idp-secret" } }),
	))
	.expect("config deserializes");
	let auth = BackendAuth::IdentityAssertion(Box::new(cfg));

	for _ in 0..2 {
		let mut req = request_with_identity("user-1", "header.payload.signature");
		apply_backend_auth(&idjag_backend_info(&t), &auth, &mut req)
			.await
			.expect("apply backend auth");
		assert_eq!(
			req.headers().get(http::header::AUTHORIZATION).unwrap(),
			"Bearer backend-access-token"
		);
	}
	// .expect(1) on each mock is verified when the servers drop.
	assert_eq!(idp.received_requests().await.unwrap().len(), 1);
	assert_eq!(resource_as.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_identity_assertion_client_secret_basic() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;
	mount_idp(&idp, 1).await;
	mount_resource_as(&resource_as, 1).await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({ "clientSecretBasic": { "clientSecret": "idp-secret" } }),
	))
	.expect("config deserializes");

	let mut req = request_with_identity("user-1", "header.payload.signature");
	apply_backend_auth(
		&idjag_backend_info(&t),
		&BackendAuth::IdentityAssertion(Box::new(cfg)),
		&mut req,
	)
	.await
	.expect("apply backend auth");

	let reqs = idp.received_requests().await.unwrap();
	let auth = reqs[0]
		.headers
		.get(http::header::AUTHORIZATION)
		.expect("idp request has basic auth");
	let expected = format!(
		"Basic {}",
		base64::engine::general_purpose::STANDARD.encode("gw-idp:idp-secret")
	);
	assert_eq!(auth.to_str().unwrap(), expected);
}

#[tokio::test]
async fn test_identity_assertion_private_key_jwt() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;
	mount_idp(&idp, 1).await;
	mount_resource_as(&resource_as, 1).await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({
			"privateKeyJwt": {
				"signingKey": TEST_EC_PRIVATE_KEY_PEM,
				"alg": "ES256",
				"kid": "test-kid",
			}
		}),
	))
	.expect("config deserializes");

	let mut req = request_with_identity("user-1", "header.payload.signature");
	apply_backend_auth(
		&idjag_backend_info(&t),
		&BackendAuth::IdentityAssertion(Box::new(cfg)),
		&mut req,
	)
	.await
	.expect("apply backend auth");

	let idp_form = last_form(&idp).await;
	assert_eq!(
		idp_form.get("client_assertion_type").map(String::as_str),
		Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
	);
	let assertion = idp_form
		.get("client_assertion")
		.expect("client_assertion present");

	// Decode the JWT payload (signature not verified here) and check the claims.
	let payload_b64 = assertion.split('.').nth(1).expect("jwt has a payload");
	let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
		.decode(payload_b64)
		.expect("payload is base64url");
	let claims: Value = serde_json::from_slice(&payload).expect("payload is json");
	assert_eq!(claims["iss"], "gw-idp");
	assert_eq!(claims["sub"], "gw-idp");
	assert_eq!(claims["aud"], format!("{}/token", idp.uri()));
	assert!(claims.get("jti").and_then(Value::as_str).is_some());
}

#[tokio::test]
async fn test_identity_assertion_idp_error_is_surfaced() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "invalid_grant" })))
		.mount(&idp)
		.await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({ "clientSecretBasic": { "clientSecret": "idp-secret" } }),
	))
	.expect("config deserializes");

	let mut req = request_with_identity("user-1", "header.payload.signature");
	let err = apply_backend_auth(
		&idjag_backend_info(&t),
		&BackendAuth::IdentityAssertion(Box::new(cfg)),
		&mut req,
	)
	.await
	.expect_err("should fail when the IdP rejects the exchange");
	assert!(req.headers().get(http::header::AUTHORIZATION).is_none());
	assert!(err.to_string().contains("401") || err.to_string().contains("ID-JAG"));
}

#[tokio::test]
async fn test_identity_assertion_missing_identity_is_rejected() {
	let idp = MockServer::start().await;
	let resource_as = MockServer::start().await;

	let t = setup_proxy_test("{}").expect("setup proxy inputs");
	let cfg: IdentityAssertion = serde_json::from_value(idjag_config(
		&idp,
		&resource_as,
		json!({ "clientSecretBasic": { "clientSecret": "idp-secret" } }),
	))
	.expect("config deserializes");

	// No Claims inserted into the request.
	let mut req = crate::http::Request::new(crate::http::Body::empty());
	let err = apply_backend_auth(
		&idjag_backend_info(&t),
		&BackendAuth::IdentityAssertion(Box::new(cfg)),
		&mut req,
	)
	.await
	.expect_err("should fail without an authenticated identity");
	assert!(err.to_string().contains("authenticated request"));
}
