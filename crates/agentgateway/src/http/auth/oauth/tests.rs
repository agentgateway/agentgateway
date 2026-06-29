use std::collections::HashMap;

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::json;
use url::form_urlencoded;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::http::Body;
use crate::http::oauth::{
	GRANT_TYPE_JWT_BEARER, GRANT_TYPE_TOKEN_EXCHANGE, TOKEN_TYPE_ID, TOKEN_TYPE_JWT,
};
use crate::types::agent::Target;

fn policy_client() -> PolicyClient {
	PolicyClient::new(
		crate::test_helpers::proxymock::setup_proxy_test("{}")
			.unwrap()
			.inputs(),
	)
}

fn token_body() -> serde_json::Value {
	json!({
		"access_token": "upstream-token",
		"token_type": "Bearer",
		"issued_token_type": TOKEN_TYPE_ACCESS,
		"expires_in": 3600,
	})
}

fn token_body_without_expiry() -> serde_json::Value {
	json!({
		"access_token": "upstream-token",
		"token_type": "Bearer",
		"issued_token_type": TOKEN_TYPE_ACCESS,
	})
}

async fn mock_token_endpoint(body: ResponseTemplate) -> MockServer {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(body)
		.mount(&mock)
		.await;
	mock
}

fn endpoint(mock: &MockServer) -> Arc<SimpleBackendReference> {
	Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
		*mock.address(),
	)))
}

fn base_auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
	OAuthTokenExchangeAuth {
		target: endpoint,
		policies: vec![],
		token_endpoint_path: "/token".into(),
		grant_type: OAuthGrantType::TokenExchange,
		subject_token: TokenSpec::default(),
		actor_token: None,
		audiences: vec![],
		scopes: vec![],
		resources: vec![],
		requested_token_type: None,
		client_auth: None,
		additional_params: BTreeMap::new(),
		authorization_location: AuthorizationLocation::default(),
		cache: TokenExchangeCache::default(),
	}
}

fn auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
	OAuthTokenExchangeAuth {
		audiences: vec!["https://upstream.example".into()],
		..base_auth(endpoint)
	}
}

fn exchange_req(subject: &str, token_type: &str) -> ExchangeRequest {
	ExchangeRequest {
		subject_token: subject.to_string().into(),
		subject_token_type: token_type.to_string(),
		..Default::default()
	}
}

fn jwt_with_exp(exp: u64) -> String {
	let header = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
	let body =
		base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#).as_bytes());
	format!("{header}.{body}.sig")
}

fn backend_info() -> crate::http::auth::BackendInfo {
	crate::http::auth::BackendInfo {
		target: crate::types::agent::BackendTarget::Invalid,
		call_target: Target::Hostname(crate::strng::new("unused"), 0),
		inputs: crate::test_helpers::proxymock::setup_proxy_test("{}")
			.unwrap()
			.inputs(),
	}
}

fn incoming_request() -> crate::http::Request {
	::http::Request::builder()
		.method(::http::Method::GET)
		.uri("http://upstream/")
		.header(::http::header::AUTHORIZATION, "Bearer subj")
		.body(Body::empty())
		.unwrap()
}

async fn sent_form_params(mock: &MockServer) -> HashMap<String, String> {
	let req = &mock.received_requests().await.unwrap()[0];
	form_urlencoded::parse(&req.body).into_owned().collect()
}

fn assert_proto_err_contains(proto: proto::OAuthTokenExchange, expected: &str) {
	let err = OAuthTokenExchangeAuth::try_from(proto).unwrap_err();
	assert!(
		matches!(err, ProtoError::Generic(ref m) if m.contains(expected)),
		"expected error containing {expected:?}, got {err:?}"
	);
}

#[test]
fn deserializes_minimal_config() {
	let a: OAuthTokenExchangeAuth = serde_json::from_str(r#"{"host": "localhost:8089"}"#).unwrap();
	assert!(matches!(
		a.target.as_ref(),
		SimpleBackendReference::InlineBackend(_)
	));
	assert!(a.token_endpoint_path.is_empty());
}

#[tokio::test]
async fn fails_closed_on_slow_endpoint() {
	let mock = mock_token_endpoint(
		ResponseTemplate::new(200)
			.set_body_json(token_body())
			.set_delay(Duration::from_secs(2)),
	)
	.await;
	let a = OAuthTokenExchangeAuth {
		policies: vec![BackendTrafficPolicy::HTTP(crate::types::backend::HTTP {
			request_timeout: Some(Duration::from_millis(50)),
			..Default::default()
		})],
		..base_auth(endpoint(&mock))
	};

	let err = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap_err();
	assert!(err.to_string().contains("timeout"), "got: {err}");
}

#[tokio::test]
async fn sends_form_params() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));

	let tok = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj-jwt", TOKEN_TYPE_ACCESS),
	)
	.await
	.expect("exchange succeeds");
	assert_eq!(tok.expose_secret(), "upstream-token");

	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["grant_type"], GRANT_TYPE_TOKEN_EXCHANGE);
	assert_eq!(pairs["subject_token"], "subj-jwt");
	assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_ACCESS);
	assert_eq!(pairs["audience"], "https://upstream.example");
	for k in ["scope", "resource", "requested_token_type", "client_id"] {
		assert!(!pairs.contains_key(k), "unset param {k} must not be sent");
	}
}

#[tokio::test]
async fn sends_optional_params() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		scopes: vec!["read".into(), "write".into()],
		resources: vec!["https://upstream.example/api".into()],
		requested_token_type: Some(TOKEN_TYPE_ACCESS.into()),
		client_auth: Some(OAuthClientAuth::new(
			"gateway-client".into(),
			None,
			OAuthClientAuthMethod::ClientSecretPost,
		)),
		..base_auth(endpoint(&mock))
	};

	fetch_token(&policy_client(), &a, exchange_req("subj", TOKEN_TYPE_JWT))
		.await
		.unwrap();
	let pairs = sent_form_params(&mock).await;
	assert!(!pairs.contains_key("audience"));
	assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_JWT);
	assert_eq!(pairs["scope"], "read write");
	assert_eq!(pairs["resource"], "https://upstream.example/api");
	assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
	assert_eq!(pairs["client_id"], "gateway-client");
	assert!(
		!pairs.contains_key("client_secret"),
		"public client sends no secret"
	);
}

#[tokio::test]
async fn sends_google_sts_workload_identity_form_without_authorization_header() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let audience = "//iam.googleapis.com/projects/123456789012/locations/global/workloadIdentityPools/pool/providers/provider";
	let a = OAuthTokenExchangeAuth {
		audiences: vec![audience.into()],
		scopes: vec!["https://www.googleapis.com/auth/cloud-platform".into()],
		requested_token_type: Some(TOKEN_TYPE_ACCESS.into()),
		..base_auth(endpoint(&mock))
	};

	fetch_token(
		&policy_client(),
		&a,
		exchange_req("external-id-token", TOKEN_TYPE_ID),
	)
	.await
	.unwrap();

	let req = &mock.received_requests().await.unwrap()[0];
	assert!(
		req.headers.get("authorization").is_none(),
		"Google STS requests should not send client auth when client_auth is unset"
	);
	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["grant_type"], GRANT_TYPE_TOKEN_EXCHANGE);
	assert_eq!(pairs["audience"], audience);
	assert_eq!(
		pairs["scope"],
		"https://www.googleapis.com/auth/cloud-platform"
	);
	assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
	assert_eq!(pairs["subject_token"], "external-id-token");
	assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_ID);
}

#[rstest]
#[case(TOKEN_TYPE_JWT, "upstream-jwt")]
#[case(TOKEN_TYPE_ID, "upstream-id-token")]
#[tokio::test]
async fn accepts_requested_response_type(
	#[case] requested_token_type: &str,
	#[case] access_token: &str,
) {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": access_token,
		"token_type": "Bearer",
		"issued_token_type": requested_token_type,
	})))
	.await;
	let a = OAuthTokenExchangeAuth {
		requested_token_type: Some(requested_token_type.into()),
		..base_auth(endpoint(&mock))
	};

	let tok = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.expect("requested response type should be accepted");
	assert_eq!(tok.expose_secret(), access_token);
}

#[tokio::test]
async fn client_secret_basic_uses_authorization_header() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth::new(
			"gw client".into(),
			Some("s3cr3t".into()),
			OAuthClientAuthMethod::ClientSecretBasic,
		)),
		..base_auth(endpoint(&mock))
	};

	fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap();

	let req = &mock.received_requests().await.unwrap()[0];
	let header = req.headers["authorization"].to_str().unwrap();
	assert_eq!(
		header,
		format!("Basic {}", BASE64_STANDARD.encode("gw+client:s3cr3t"))
	);
	let pairs = sent_form_params(&mock).await;
	assert!(
		!pairs.contains_key("client_id"),
		"basic auth keeps creds out of the body"
	);
	assert!(!pairs.contains_key("client_secret"));
}

#[tokio::test]
async fn client_secret_post_uses_form_body() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth::new(
			"gateway-client".into(),
			Some("s3cr3t".into()),
			OAuthClientAuthMethod::ClientSecretPost,
		)),
		..base_auth(endpoint(&mock))
	};

	fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap();

	let req = &mock.received_requests().await.unwrap()[0];
	assert!(req.headers.get("authorization").is_none());
	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["client_id"], "gateway-client");
	assert_eq!(pairs["client_secret"], "s3cr3t");
}

#[tokio::test]
async fn jwt_bearer_sends_assertion() {
	// RFC 7523 response: a plain RFC 6749 body with no issued_token_type.
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "upstream-token",
		"token_type": "Bearer",
	})))
	.await;
	let a = OAuthTokenExchangeAuth {
		grant_type: OAuthGrantType::JwtBearer,
		..base_auth(endpoint(&mock))
	};

	let tok = fetch_token(
		&policy_client(),
		&a,
		exchange_req("the-jwt", TOKEN_TYPE_ACCESS),
	)
	.await
	.expect("jwt-bearer exchange succeeds");
	assert_eq!(tok.expose_secret(), "upstream-token");

	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["grant_type"], GRANT_TYPE_JWT_BEARER);
	assert_eq!(pairs["assertion"], "the-jwt");
	for k in [
		"subject_token",
		"subject_token_type",
		"requested_token_type",
	] {
		assert!(!pairs.contains_key(k), "jwt-bearer must not send {k}");
	}
}

#[rstest]
#[case::missing_token_type(
	json!({
		"access_token": "upstream-token",
		"issued_token_type": TOKEN_TYPE_ACCESS,
		"expires_in": 3600,
	}),
	"missing token_type"
)]
#[case::empty_access_token(
	json!({
		"access_token": "",
		"token_type": "Bearer",
		"issued_token_type": TOKEN_TYPE_ACCESS,
		"expires_in": 3600,
	}),
	"empty access_token"
)]
#[tokio::test]
async fn rejects_invalid_token_response(
	#[case] response_body: serde_json::Value,
	#[case] expected: &str,
) {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(response_body)).await;
	let a = auth(endpoint(&mock));

	let err = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap_err();
	assert!(err.to_string().contains(expected), "got: {err}");
}

#[rstest]
#[case::unusable_issued_type(
	OAuthGrantType::JwtBearer,
	None,
	"urn:ietf:params:oauth:token-type:saml2",
	"unusable issued_token_type"
)]
#[case::issued_type_mismatch(
	OAuthGrantType::TokenExchange,
	Some(TOKEN_TYPE_JWT),
	TOKEN_TYPE_ACCESS,
	"expected"
)]
#[case::missing_requested_type_defaults_to_access(
	OAuthGrantType::TokenExchange,
	None,
	TOKEN_TYPE_JWT,
	TOKEN_TYPE_ACCESS
)]
#[tokio::test]
async fn rejects_mismatched_issued_token_type(
	#[case] grant_type: OAuthGrantType,
	#[case] requested_token_type: Option<&str>,
	#[case] issued_token_type: &str,
	#[case] expected_err: &str,
) {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "t",
		"token_type": "Bearer",
		"issued_token_type": issued_token_type,
	})))
	.await;
	let a = OAuthTokenExchangeAuth {
		grant_type,
		requested_token_type: requested_token_type.map(Into::into),
		..base_auth(endpoint(&mock))
	};

	let err = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap_err();
	assert!(err.to_string().contains(expected_err), "got: {err}");
}

#[rstest]
#[case(400, true)]
#[case(401, false)]
#[case(403, false)]
#[case(503, false)]
#[tokio::test]
async fn maps_error_status_by_class(#[case] status: u16, #[case] expect_client_error: bool) {
	let response = if expect_client_error {
		ResponseTemplate::new(status).set_body_string(r#"{"error":"invalid_grant"}"#)
	} else {
		ResponseTemplate::new(status)
	};
	let mock = mock_token_endpoint(response).await;
	let a = auth(endpoint(&mock));

	let err = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap_err();
	if expect_client_error {
		assert!(
			matches!(err, FetchError::Client { status: actual, .. } if actual == ::http::StatusCode::from_u16(status).unwrap()),
			"got: {err:?}"
		);
	} else {
		assert!(matches!(err, FetchError::Upstream(_)), "got: {err:?}");
	}
}

#[rstest]
#[case::same_subject_hits_cache(TokenExchangeCache::default(), token_body(), "subj".to_string(), 1)]
#[case::missing_expires_in_falls_back_to_default_ttl(
	TokenExchangeCache::new(&TokenCacheConfig {
		default_ttl: Duration::from_secs(120),
		..Default::default()
	}),
	token_body_without_expiry(),
	"subj".to_string(),
	1
)]
#[case::disabled_cache_always_misses(
	TokenExchangeCache::new(&TokenCacheConfig {
		max_entries: 0,
		..Default::default()
	}),
	token_body(),
	"subj".to_string(),
	2
)]
#[case::expired_subject_not_cached(
	TokenExchangeCache::default(),
	token_body(),
	jwt_with_exp(
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs()
			.saturating_sub(10),
	),
	2
)]
#[tokio::test]
async fn caches_token_per_request(
	#[case] cache: TokenExchangeCache,
	#[case] response_body: serde_json::Value,
	#[case] subject: String,
	#[case] expected_requests: usize,
) {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(response_body)).await;
	let a = OAuthTokenExchangeAuth {
		cache,
		..base_auth(endpoint(&mock))
	};
	let client = policy_client();

	let t1 = fetch_token(&client, &a, exchange_req(&subject, TOKEN_TYPE_ACCESS))
		.await
		.unwrap();
	let t2 = fetch_token(&client, &a, exchange_req(&subject, TOKEN_TYPE_ACCESS))
		.await
		.unwrap();

	assert_eq!(t1.expose_secret(), t2.expose_secret());
	assert_eq!(
		mock.received_requests().await.unwrap().len(),
		expected_requests
	);
}

#[tokio::test]
async fn caches_per_subject_token_type() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));
	let client = policy_client();

	fetch_token(&client, &a, exchange_req("subj", TOKEN_TYPE_ACCESS))
		.await
		.unwrap();
	fetch_token(&client, &a, exchange_req("subj", TOKEN_TYPE_JWT))
		.await
		.unwrap();
	fetch_token(&client, &a, exchange_req("subj", TOKEN_TYPE_ACCESS))
		.await
		.unwrap();

	assert_eq!(mock.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn appends_additional_params() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));
	let req = ExchangeRequest {
		subject_token: "subj".to_string().into(),
		subject_token_type: TOKEN_TYPE_ACCESS.to_string(),
		actor: None,
		extra_params: vec![
			("vendor_id".into(), "v1".into()),
			("org".into(), "o2".into()),
		],
	};

	fetch_token(&policy_client(), &a, req).await.unwrap();

	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["vendor_id"], "v1");
	assert_eq!(pairs["org"], "o2");
}

#[test]
fn evaluates_additional_params() {
	let (expr, err) = cel::Expression::new_permissive("\"static-value\"".to_string());
	assert!(err.is_none(), "{err:?}");
	let a = OAuthTokenExchangeAuth {
		additional_params: BTreeMap::from([("p".to_string(), Arc::new(expr))]),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	};
	let req = ::http::Request::builder()
		.method(::http::Method::GET)
		.uri("http://example/")
		.body(Body::empty())
		.unwrap();

	let params = a.evaluate_additional_params(&req).unwrap();
	assert_eq!(params, vec![("p".to_string(), "static-value".to_string())]);
}

#[test]
fn rejects_reserved_additional_param() {
	let proto = proto::OAuthTokenExchange {
		additional_params: std::collections::HashMap::from([(
			"client_assertion".to_string(),
			"x".to_string(),
		)]),
		..Default::default()
	};
	let err = OAuthTokenExchangeAuth::try_from(proto).unwrap_err();
	assert!(
		matches!(err, ProtoError::Generic(ref m) if m.contains("reserved")),
		"got: {err:?}"
	);
}

#[test]
fn invalid_cel_additional_param_parses_permissively() {
	let proto = proto::OAuthTokenExchange {
		additional_params: HashMap::from([("p".to_string(), "((".to_string())]),
		..Default::default()
	};
	// Like the rest of the xDS path, a bad CEL expression is parsed permissively:
	// conversion succeeds and the expression fails when evaluated at request time
	// instead of rejecting the whole config push.
	let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
	assert!(
		auth
			.evaluate_additional_params(&incoming_request())
			.is_err()
	);
}

fn assert_load_err(auth: OAuthTokenExchangeAuth, expected: &str) {
	let err = auth
		.validate_load()
		.expect_err("invalid local config should fail validation");
	assert!(
		err.contains(expected),
		"expected error containing {expected:?}, got {err:?}"
	);
}

#[rstest]
#[case::token_endpoint_path(
	OAuthTokenExchangeAuth {
		token_endpoint_path: "token".into(),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"must start with /"
)]
#[case::jwt_bearer_actor_token(
	OAuthTokenExchangeAuth {
		grant_type: OAuthGrantType::JwtBearer,
		actor_token: Some(ActorTokenSpec {
			source: AuthorizationLocation::default(),
			token_type: default_token_type(),
		}),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"actor_token"
)]
#[case::basic_without_secret(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth::new(
			"gateway-client".into(),
			None,
			OAuthClientAuthMethod::ClientSecretBasic,
		)),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"client_secret"
)]
#[case::empty_client_id(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth::new(
			String::new(),
			Some("secret".into()),
			OAuthClientAuthMethod::ClientSecretPost,
		)),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"client_id"
)]
#[case::empty_client_secret(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth::new(
			"gateway-client".into(),
			Some("".into()),
			OAuthClientAuthMethod::ClientSecretPost,
		)),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"client_secret"
)]
#[case::reserved_additional_param(
	OAuthTokenExchangeAuth {
		additional_params: BTreeMap::from([(
			"scope".into(),
			Arc::new(cel::Expression::new_strict(r#""read""#).unwrap()),
		)]),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"reserved"
)]
#[case::expression_output_location(
	OAuthTokenExchangeAuth {
		authorization_location: AuthorizationLocation::Expression {
			expression: Arc::new(cel::Expression::new_strict(r#""token""#).unwrap()),
		},
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"credential extraction"
)]
#[test]
fn validate_load_rejects_invalid_local_config(
	#[case] auth: OAuthTokenExchangeAuth,
	#[case] expected: &str,
) {
	assert_load_err(auth, expected);
}

#[test]
fn accepts_supported_requested_token_types_from_proto() {
	for token_type in [TOKEN_TYPE_ACCESS, TOKEN_TYPE_JWT, TOKEN_TYPE_ID] {
		let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
			requested_token_type: Some(token_type.to_string()),
			..Default::default()
		})
		.unwrap();
		assert_eq!(auth.requested_token_type.as_deref(), Some(token_type));
	}
}

#[rstest]
#[case::unsupported_requested_token_type(
	proto::OAuthTokenExchange {
		requested_token_type: Some("urn:ietf:params:oauth:token-type:saml2".to_string()),
		..Default::default()
	},
	"unsupported requested_token_type"
)]
#[case::non_slash_token_endpoint_path(
	proto::OAuthTokenExchange {
		token_endpoint_path: Some("noslash".to_string()),
		..Default::default()
	},
	"must start with /"
)]
#[case::empty_client_id(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::o_auth_token_exchange::ClientAuth {
			client_id: String::new(),
			client_secret: Some("s".to_string()),
			method: proto::o_auth_token_exchange::client_auth::Method::ClientSecretPost as i32,
		}),
		..Default::default()
	},
	"client_id"
)]
#[case::empty_client_secret(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::o_auth_token_exchange::ClientAuth {
			client_id: "gateway-client".to_string(),
			client_secret: Some(String::new()),
			method: proto::o_auth_token_exchange::client_auth::Method::ClientSecretPost as i32,
		}),
		..Default::default()
	},
	"client_secret"
)]
#[case::jwt_bearer_actor_token(
	proto::OAuthTokenExchange {
		grant_type: proto::o_auth_token_exchange::GrantType::JwtBearer as i32,
		actor_token: Some(proto::o_auth_token_exchange::ActorToken::default()),
		..Default::default()
	},
	"actor_token"
)]
#[case::actor_token_without_source(
	proto::OAuthTokenExchange {
		actor_token: Some(proto::o_auth_token_exchange::ActorToken::default()),
		..Default::default()
	},
	"actor_token.source"
)]
#[case::expression_output_location(
	proto::OAuthTokenExchange {
		authorization_location: Some(proto::AuthorizationLocation {
			kind: Some(proto::authorization_location::Kind::Expression(
				"foo".to_string(),
			)),
		}),
		..Default::default()
	},
	"credential extraction"
)]
#[test]
fn rejects_invalid_proto_config(#[case] proto: proto::OAuthTokenExchange, #[case] expected: &str) {
	assert_proto_err_contains(proto, expected);
}

#[test]
fn disabled_cache_from_proto_disables_storage() {
	let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
		cache: Some(proto::o_auth_token_exchange::TokenCache {
			in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
				max_entries: Some(0),
				default_ttl: None,
			}),
		}),
		..Default::default()
	})
	.unwrap();

	assert!(!auth.cache.enabled());
}

#[test]
fn in_memory_cache_from_proto_uses_default_ttl_and_capacity_defaults() {
	let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
		cache: Some(proto::o_auth_token_exchange::TokenCache {
			in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
				max_entries: None,
				default_ttl: Some(prost_types::Duration {
					seconds: 42,
					nanos: 0,
				}),
			}),
		}),
		..Default::default()
	})
	.unwrap();

	assert!(auth.cache.enabled());
	assert_eq!(auth.cache.default_ttl(), Duration::from_secs(42));
}

#[tokio::test]
async fn sends_actor_token() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));
	let req = ExchangeRequest {
		subject_token: "subj".to_string().into(),
		subject_token_type: TOKEN_TYPE_ACCESS.to_string(),
		actor: Some(("actor-tok".to_string().into(), TOKEN_TYPE_JWT.to_string())),
		extra_params: vec![],
	};

	fetch_token(&policy_client(), &a, req).await.unwrap();

	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["actor_token"], "actor-tok");
	assert_eq!(pairs["actor_token_type"], TOKEN_TYPE_JWT);
}

#[test]
fn subject_token_source_and_type_from_proto() {
	let proto = proto::OAuthTokenExchange {
		subject_token: Some(proto::o_auth_token_exchange::TokenSpec {
			source: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::Header(
					proto::authorization_location::Header {
						name: "x-subject".to_string(),
						prefix: None,
					},
				)),
			}),
			token_type: String::new(),
		}),
		..Default::default()
	};
	let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
	assert!(
		matches!(&auth.subject_token.source, AuthorizationLocation::Header { name, .. } if name.as_str() == "x-subject")
	);
	// Empty proto token_type defaults to access_token.
	assert_eq!(auth.subject_token.token_type, TOKEN_TYPE_ACCESS);
}

#[test]
fn authorization_location_from_proto() {
	let proto = proto::OAuthTokenExchange {
		authorization_location: Some(proto::AuthorizationLocation {
			kind: Some(proto::authorization_location::Kind::Header(
				proto::authorization_location::Header {
					name: "x-upstream-auth".to_string(),
					prefix: None,
				},
			)),
		}),
		..Default::default()
	};
	let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
	assert!(matches!(
		auth.authorization_location,
		AuthorizationLocation::Header { ref name, .. } if name.as_str() == "x-upstream-auth"
	));
}

#[test]
fn query_parameter_authorization_location_from_proto() {
	let proto = proto::OAuthTokenExchange {
		authorization_location: Some(proto::AuthorizationLocation {
			kind: Some(proto::authorization_location::Kind::QueryParameter(
				proto::authorization_location::QueryParameter {
					name: "access_token".to_string(),
				},
			)),
		}),
		..Default::default()
	};
	let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
	assert!(matches!(
		auth.authorization_location,
		AuthorizationLocation::QueryParameter { ref name } if name.as_str() == "access_token"
	));
}

#[tokio::test]
async fn dispatch_inserts_default_bearer_and_marks_explicit() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let backend_auth =
		crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(auth(endpoint(&mock))));
	let mut req = incoming_request();

	crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap();

	let hv = req
		.headers()
		.get(::http::header::AUTHORIZATION)
		.unwrap()
		.to_str()
		.unwrap();
	assert_eq!(hv, "Bearer upstream-token");
	let applied = req
		.extensions()
		.get::<crate::http::auth::AppliedBackendAuthLocation>()
		.unwrap();
	assert!(applied.explicit, "oauth output must be marked explicit");
}

#[tokio::test]
async fn dispatch_uses_configured_output_location_and_marks_explicit() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		authorization_location: AuthorizationLocation::Header {
			name: ::http::HeaderName::from_static("x-upstream-auth"),
			prefix: None,
		},
		..auth(endpoint(&mock))
	};
	let backend_auth = crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a));
	let mut req = incoming_request();

	crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap();

	let hv = req
		.headers()
		.get("x-upstream-auth")
		.unwrap()
		.to_str()
		.unwrap();
	assert_eq!(hv, "upstream-token");
	let applied = req
		.extensions()
		.get::<crate::http::auth::AppliedBackendAuthLocation>()
		.unwrap();
	assert!(
		applied.explicit,
		"configured location must be marked explicit"
	);
}

#[tokio::test]
async fn dispatch_supports_query_parameter_output_location() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		authorization_location: AuthorizationLocation::QueryParameter {
			name: "access_token".into(),
		},
		..auth(endpoint(&mock))
	};
	let backend_auth = crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a));
	let mut req = incoming_request();

	crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap();

	assert!(req.headers().get(::http::header::AUTHORIZATION).is_none());
	assert_eq!(req.uri().query(), Some("access_token=upstream-token"));
	let applied = req
		.extensions()
		.get::<crate::http::auth::AppliedBackendAuthLocation>()
		.unwrap();
	assert!(applied.explicit, "query output must be marked explicit");
}

#[tokio::test]
async fn dispatch_removes_input_token_locations_before_inserting_output() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		actor_token: Some(ActorTokenSpec {
			source: AuthorizationLocation::Header {
				name: ::http::HeaderName::from_static("x-actor-token"),
				prefix: None,
			},
			token_type: TOKEN_TYPE_JWT.to_string(),
		}),
		authorization_location: AuthorizationLocation::Header {
			name: ::http::HeaderName::from_static("x-upstream-auth"),
			prefix: None,
		},
		..auth(endpoint(&mock))
	};
	let backend_auth = crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a));
	let mut req = ::http::Request::builder()
		.method(::http::Method::GET)
		.uri("http://upstream/")
		.header(::http::header::AUTHORIZATION, "Bearer subj")
		.header("x-actor-token", "actor")
		.body(Body::empty())
		.unwrap();

	crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap();

	assert!(req.headers().get(::http::header::AUTHORIZATION).is_none());
	assert!(req.headers().get("x-actor-token").is_none());
	assert_eq!(
		req
			.headers()
			.get("x-upstream-auth")
			.unwrap()
			.to_str()
			.unwrap(),
		"upstream-token"
	);
}
