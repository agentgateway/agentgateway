use std::collections::HashMap;

use base64::Engine;
use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::json;
use url::form_urlencoded;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::client_auth::RawPrivateKeyJwt;
use super::cross_app_access::CrossAppAccessEndpoint;
use super::*;
use crate::http::Body;
use crate::http::oauth::{
	CLIENT_ASSERTION_TYPE_JWT_BEARER, GRANT_TYPE_JWT_BEARER, GRANT_TYPE_TOKEN_EXCHANGE,
	TOKEN_TYPE_ID, TOKEN_TYPE_ID_JAG, TOKEN_TYPE_JWT,
};
use crate::serdes::FileOrInline;
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference, Target};

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
		target: SimpleBackendReferenceWithPolicies {
			target: endpoint,
			policies: vec![],
		},
		path: "/token".into(),
		grant_type: OAuthGrantType::TokenExchange,
		subject_token: TokenSpec::default(),
		actor_token: None,
		audiences: vec![],
		scopes: vec![],
		resources: vec![],
		requested_token_type: None,
		client_auth: None,
		additional_params: BTreeMap::new(),
		chained_exchange: None,
		authorization_location: AuthorizationLocation::default(),
		cache: Some(InMemoryTokenCache::default()),
	}
}

fn auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
	OAuthTokenExchangeAuth {
		audiences: vec!["https://upstream.example".into()],
		..base_auth(endpoint)
	}
}

fn cross_app_access_endpoint(endpoint: Arc<SimpleBackendReference>) -> CrossAppAccessEndpoint {
	CrossAppAccessEndpoint {
		target: SimpleBackendReferenceWithPolicies {
			target: endpoint,
			policies: vec![],
		},
		path: "/token".into(),
		client_auth: OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::ClientSecretPost {
				client_secret: None,
			},
		},
	}
}

fn cross_app_access_raw(
	idp: Arc<SimpleBackendReference>,
	resource_as: Arc<SimpleBackendReference>,
) -> CrossAppAccessAuth {
	CrossAppAccessAuth {
		identity_provider: cross_app_access_endpoint(idp),
		resource_authorization_server: cross_app_access_endpoint(resource_as),
		audience: "https://resource-as.example".into(),
		resources: vec![],
		scopes: vec!["read".into()],
		cache: Some(InMemoryTokenCache::default()),
		oauth: None,
	}
}

fn cross_app_access(
	idp: Arc<SimpleBackendReference>,
	resource_as: Arc<SimpleBackendReference>,
) -> CrossAppAccessAuth {
	let mut auth = cross_app_access_raw(idp, resource_as);
	auth.apply_local_defaults().unwrap();
	auth
}

fn cross_app_access_with_resources(
	idp: Arc<SimpleBackendReference>,
	resource_as: Arc<SimpleBackendReference>,
	resources: Vec<String>,
) -> CrossAppAccessAuth {
	let mut auth = cross_app_access_raw(idp, resource_as);
	auth.resources = resources;
	auth.apply_local_defaults().unwrap();
	auth
}

fn exchange_req(subject: &str, token_type: &str) -> ExchangeRequest {
	ExchangeRequest {
		subject_token: subject.to_string().into(),
		subject_token_type: token_type_from_urn(token_type),
		..Default::default()
	}
}

fn token_type_from_urn(token_type: &str) -> OAuthTokenType {
	OAuthTokenType::from_urn(token_type).unwrap()
}

fn jwt_with_claims(claims: &serde_json::Value) -> String {
	let header = BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
	let body = BASE64_URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
	format!("{header}.{body}.sig")
}

const TEST_EC_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgltxBTVDLg7C6vE1T
7OtwJIZ/dpm8ygE2MBTjPCY3hgahRANCAARYzu50EeBrT0rELmTGroaGtn0zdjxL
1lOGr9fGw5wOGcXO0+Gn5F5sIxGyTM0FwnUHFNz2SoixZR5dtxhNc+Lo
-----END PRIVATE KEY-----
";

fn claims_with_may_act(
	subject_token: &str,
	may_act: serde_json::Value,
) -> crate::http::jwt::Claims {
	let serde_json::Value::Object(inner) = json!({"may_act": may_act}) else {
		unreachable!()
	};
	crate::http::jwt::Claims {
		inner,
		jwt: subject_token.to_string().into(),
	}
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
	let err = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap_err();
	assert!(
		matches!(err, ProtoError::Generic(ref m) if m.contains(expected)),
		"expected error containing {expected:?}, got {err:?}"
	);
}

#[test]
fn deserializes_minimal_config() {
	let a: OAuthTokenExchangeAuth =
		serde_json::from_str(r#"{"host": "localhost:8089", "path": "/oauth2/token"}"#).unwrap();
	assert!(matches!(
		a.target.target.as_ref(),
		SimpleBackendReference::InlineBackend(_)
	));
	assert_eq!(a.path, "/oauth2/token");
	assert!(a.cache.is_some());
}

#[test]
fn deserializes_local_cache_config() {
	let a: OAuthTokenExchangeAuth = serde_json::from_str(
		r#"{
			"host": "localhost:8089",
			"cache": {
				"defaultTtl": "42s"
			}
		}"#,
	)
	.unwrap();

	assert!(a.cache.is_some());

	let cfg: TokenCacheConfig = serde_json::from_value(json!({"defaultTtl": "42s"})).unwrap();
	assert_eq!(cfg.default_ttl, Some(Duration::from_secs(42)));
}

#[test]
fn local_cache_config_can_disable_storage() {
	let a: OAuthTokenExchangeAuth = serde_json::from_str(
		r#"{
			"host": "localhost:8089",
			"cache": {
				"maxEntries": 0
			}
		}"#,
	)
	.unwrap();

	assert!(a.cache.is_none());
}

#[test]
fn deserializes_custom_subject_token_type_uri() {
	let auth = serde_json::from_str::<OAuthTokenExchangeAuth>(
		r#"{"host": "localhost:8089", "subjectToken": {"tokenType": "urn:company:domain:human"}}"#,
	)
	.expect("custom absolute URI token type should deserialize");
	assert_eq!(
		auth.subject_token.token_type.as_str(),
		"urn:company:domain:human"
	);
}

#[tokio::test]
async fn fails_closed_on_slow_endpoint() {
	let mock = mock_token_endpoint(
		ResponseTemplate::new(200)
			.set_body_json(token_body())
			.set_delay(Duration::from_secs(2)),
	)
	.await;
	let mut a = base_auth(endpoint(&mock));
	a.target.policies = vec![BackendTrafficPolicy::HTTP(crate::types::backend::HTTP {
		request_timeout: Some(Duration::from_millis(50)),
		..Default::default()
	})];

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
	assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
	for k in ["scope", "resource", "client_id"] {
		assert!(!pairs.contains_key(k), "unset param {k} must not be sent");
	}
}

#[tokio::test]
async fn sends_optional_params() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		scopes: vec!["read".into(), "write".into()],
		resources: vec!["https://upstream.example/api".into()],
		requested_token_type: Some(OAuthTokenType::AccessToken),
		client_auth: Some(OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::ClientSecretPost {
				client_secret: None,
			},
		}),
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
async fn sends_custom_subject_token_type() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = OAuthTokenExchangeAuth {
		subject_token: TokenSpec {
			source: AuthorizationLocation::default(),
			token_type: token_type_from_urn("urn:company:domain:human"),
		},
		..base_auth(endpoint(&mock))
	};

	fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", "urn:company:domain:human"),
	)
	.await
	.unwrap();
	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["subject_token_type"], "urn:company:domain:human");
	assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
}

#[tokio::test]
async fn sends_google_sts_workload_identity_form_without_authorization_header() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let audience = "//iam.googleapis.com/projects/123456789012/locations/global/workloadIdentityPools/pool/providers/provider";
	let a = OAuthTokenExchangeAuth {
		audiences: vec![audience.into()],
		scopes: vec!["https://www.googleapis.com/auth/cloud-platform".into()],
		requested_token_type: Some(OAuthTokenType::AccessToken),
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
		requested_token_type: Some(token_type_from_urn(requested_token_type)),
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
		client_auth: Some(OAuthClientAuth {
			client_id: "gw client".into(),
			method: OAuthClientAuthMethod::ClientSecretBasic {
				client_secret: "s3cr3t".into(),
			},
		}),
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
		client_auth: Some(OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::ClientSecretPost {
				client_secret: Some("s3cr3t".into()),
			},
		}),
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

#[tokio::test]
async fn id_jag_chain_exchanges_two_legs_and_caches_final_token() {
	let idp = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "id-jag-assertion",
		"token_type": "N_A",
		"issued_token_type": TOKEN_TYPE_ID_JAG,
		"expires_in": 120,
	})))
	.await;
	let resource_as = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "resource-access-token",
		"token_type": "Bearer",
		"expires_in": 3600,
	})))
	.await;
	let identity = cross_app_access_with_resources(
		endpoint(&idp),
		endpoint(&resource_as),
		vec!["https://api.resource-as.example/chat".into()],
	);
	let a = identity.oauth_token_exchange();

	for _ in 0..2 {
		let tok = fetch_token(&policy_client(), a, exchange_req("id-token", TOKEN_TYPE_ID))
			.await
			.expect("id-jag chain succeeds");
		assert_eq!(tok.expose_secret(), "resource-access-token");
	}

	let idp_requests = idp.received_requests().await.unwrap();
	assert_eq!(idp_requests.len(), 1, "root cache stores the final bearer");
	let idp_pairs: HashMap<String, String> = form_urlencoded::parse(&idp_requests[0].body)
		.into_owned()
		.collect();
	assert_eq!(idp_pairs["requested_token_type"], TOKEN_TYPE_ID_JAG);
	assert_eq!(idp_pairs["audience"], "https://resource-as.example");
	assert_eq!(idp_pairs["subject_token_type"], TOKEN_TYPE_ID);
	assert_eq!(idp_pairs["scope"], "read");
	assert_eq!(
		idp_pairs["resource"], "https://api.resource-as.example/chat",
		"idp ID-JAG request must carry the target resource"
	);

	let resource_requests = resource_as.received_requests().await.unwrap();
	assert_eq!(resource_requests.len(), 1);
	let resource_pairs: HashMap<String, String> = form_urlencoded::parse(&resource_requests[0].body)
		.into_owned()
		.collect();
	assert_eq!(resource_pairs["grant_type"], GRANT_TYPE_JWT_BEARER);
	assert_eq!(resource_pairs["assertion"], "id-jag-assertion");
	// The jwt-bearer leg sends `scope` to select the access-token scopes, but omits `resource`
	// (bound via the ID-JAG claims).
	assert_eq!(resource_pairs["scope"], "read");
	assert!(!resource_pairs.contains_key("resource"));
}

#[tokio::test]
async fn id_jag_intermediate_rejects_bearer_token_type() {
	let idp = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "id-jag-assertion",
		"token_type": "Bearer",
		"issued_token_type": TOKEN_TYPE_ID_JAG,
	})))
	.await;
	let resource_as =
		mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let identity = cross_app_access(endpoint(&idp), endpoint(&resource_as));
	let a = identity.oauth_token_exchange();

	let err = fetch_token(&policy_client(), a, exchange_req("subj", TOKEN_TYPE_ID))
		.await
		.unwrap_err();
	assert!(
		err
			.to_string()
			.contains("unsupported token_type for id-jag: Bearer"),
		"got: {err}"
	);
	assert!(resource_as.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn id_jag_chained_exchange_client_error_is_upstream_failure() {
	let idp = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "id-jag-assertion",
		"token_type": "N_A",
		"issued_token_type": TOKEN_TYPE_ID_JAG,
	})))
	.await;
	let resource_as = mock_token_endpoint(
		ResponseTemplate::new(400)
			.set_body_string(r#"{"error":"invalid_grant","error_description":"issuer not trusted"}"#),
	)
	.await;
	let identity = cross_app_access(endpoint(&idp), endpoint(&resource_as));
	let a = identity.oauth_token_exchange();

	let err = fetch_token(&policy_client(), a, exchange_req("subj", TOKEN_TYPE_ID))
		.await
		.unwrap_err();
	assert!(matches!(err, FetchError::Upstream(_)), "got: {err:?}");
	let msg = err.to_string();
	assert!(msg.contains("chained token exchange returned status 400"));
	assert!(!msg.contains("invalid_grant"), "got: {msg}");
	assert!(!msg.contains("issuer not trusted"), "got: {msg}");
}

#[tokio::test]
async fn private_key_jwt_sends_client_assertion_form_fields() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let private_key = PrivateKeyJwt::try_from(RawPrivateKeyJwt {
		signing_key: FileOrInline::Inline(TEST_EC_PRIVATE_KEY_PEM.to_string()),
		alg: SigningAlg::Es256,
		kid: Some("kid-1".into()),
		assertion_audience: "https://issuer.example/token".into(),
	})
	.unwrap();
	let a = OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::PrivateKeyJwt(private_key),
		}),
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
	assert_eq!(
		pairs["client_assertion_type"],
		CLIENT_ASSERTION_TYPE_JWT_BEARER
	);
	#[derive(serde::Deserialize)]
	struct AssertionClaims {
		iss: String,
		sub: String,
		aud: String,
		jti: String,
		iat: u64,
		exp: u64,
	}
	let claims: AssertionClaims = decode_unverified_jwt_claims(&pairs["client_assertion"]).unwrap();
	assert_eq!(claims.iss, "gateway-client");
	assert_eq!(claims.sub, "gateway-client");
	assert_eq!(claims.aud, "https://issuer.example/token");
	assert!(!claims.jti.is_empty());
	assert!(claims.exp > claims.iat);
}

#[test]
fn private_key_jwt_rejects_bad_key_at_deserialize_time() {
	let err = serde_json::from_str::<OAuthTokenExchangeAuth>(
		r#"{
			"host": "localhost:8089",
			"clientAuth": {
				"clientId": "gateway-client",
				"method": "privateKeyJwt",
				"signingKey": "not a key",
				"alg": "ES256",
				"assertionAudience": "https://issuer.example/token"
			}
		}"#,
	)
	.expect_err("bad key must fail during config load");
	assert!(err.to_string().contains("signing_key"), "got: {err}");
}

#[test]
fn client_auth_rejects_unknown_fields() {
	let err = serde_json::from_str::<OAuthTokenExchangeAuth>(
		r#"{
			"host": "localhost:8089",
			"clientAuth": {
				"clientId": "gateway-client",
				"method": "clientSecretPost",
				"clientSecret": "secret",
				"clientSecrett": "typo"
			}
		}"#,
	)
	.expect_err("unknown clientAuth fields must fail during config load");
	assert!(
		err.to_string().contains("did not match any variant"),
		"got: {err}"
	);
}

#[test]
fn client_auth_defaults_to_basic_when_method_is_omitted() {
	let auth = serde_json::from_str::<OAuthTokenExchangeAuth>(
		r#"{
			"host": "localhost:8089",
			"clientAuth": {
				"clientId": "gateway-client",
				"clientSecret": "secret"
			}
		}"#,
	)
	.unwrap();
	let client_auth = auth.client_auth.expect("client auth");
	assert_eq!(client_auth.client_id, "gateway-client");
	assert!(matches!(
		client_auth.method,
		OAuthClientAuthMethod::ClientSecretBasic { .. }
	));
}

#[test]
fn cross_app_access_endpoint_rejects_unknown_fields() {
	let err = serde_json::from_str::<CrossAppAccessAuth>(
		r#"{
				"identityProvider": {
					"host": "idp.example.com:443",
					"clientAuth": {
						"clientId": "gateway-at-idp",
						"method": "clientSecretPost"
					}
				},
				"resourceAuthorizationServer": {
					"host": "chat.example.com:443",
					"tokenEndpointPat": "/oauth2/token",
					"clientAuth": {
						"clientId": "gateway-at-chat",
						"method": "clientSecretPost"
					}
				},
				"audience": "https://chat.example.com/"
			}"#,
	)
	.expect_err("unknown endpoint fields must fail during config load");
	assert!(err.to_string().contains("unknown field"), "got: {err}");
}

fn cross_app_access_local_config() -> CrossAppAccessAuth {
	let mut auth: CrossAppAccessAuth = serde_json::from_str(
		r#"{
				"identityProvider": {
					"host": "idp.example.com:443",
					"path": "/oauth2/token",
					"clientAuth": {
						"clientId": "gateway-at-idp",
						"method": "clientSecretBasic",
						"clientSecret": "mock-idp-client-secret"
					}
				},
				"resourceAuthorizationServer": {
					"host": "chat.example.com:443",
					"path": "/oauth2/token",
					"clientAuth": {
						"clientId": "gateway-at-chat",
						"method": "clientSecretBasic",
						"clientSecret": "mock-resource-authorization-server-client-secret"
					}
				},
				"audience": "https://chat.example.com/",
				"resources": ["https://api.chat.example.com/"],
				"scopes": ["chat.read", "chat.history"],
				"cache": {
					"defaultTtl": "1h"
				}
			}"#,
	)
	.unwrap();
	auth.apply_local_defaults().unwrap();
	auth.validate_load().unwrap();
	auth
}

#[test]
fn deserializes_cross_app_access_local_config_shape() {
	let auth = cross_app_access_local_config();
	let oauth = auth.oauth_token_exchange();
	assert_eq!(oauth.requested_token_type, Some(OAuthTokenType::IdJag));
	// The IdP token-exchange leg carries the configured resource (draft requires it there).
	assert_eq!(oauth.resources, ["https://api.chat.example.com/"]);
	// The jwt-bearer leg carries `scope` (selects access-token scopes) but not `resource`.
	let chained_exchange = oauth.chained_exchange.as_ref().expect("chained exchange");
	assert!(chained_exchange.resources.is_empty());
	assert_eq!(chained_exchange.scopes, ["chat.read", "chat.history"]);
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
		requested_token_type: requested_token_type.map(token_type_from_urn),
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

#[tokio::test]
async fn jwt_bearer_ignores_unexpected_issued_token_type() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "t",
		"token_type": "Bearer",
		"issued_token_type": "urn:ietf:params:oauth:token-type:saml2",
	})))
	.await;
	let a = OAuthTokenExchangeAuth {
		grant_type: OAuthGrantType::JwtBearer,
		..base_auth(endpoint(&mock))
	};

	let token = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.expect("jwt-bearer response should not validate unneeded issued_token_type");
	assert_eq!(token.expose_secret(), "t");
}

#[rstest]
#[case(400, true)]
#[case(401, false)]
#[case(403, false)]
#[case(503, false)]
#[tokio::test]
async fn maps_error_status_by_class(#[case] status: u16, #[case] expect_client_error: bool) {
	let response = ResponseTemplate::new(status)
		.set_body_string(r#"{"error":"invalid_grant","error_description":"provider diagnostic"}"#);
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
		let msg = err.to_string();
		assert!(msg.contains(&format!("token exchange returned status {status}")));
		assert!(!msg.contains("invalid_grant"), "got: {msg}");
		assert!(!msg.contains("provider diagnostic"), "got: {msg}");
	}
}

#[tokio::test]
async fn appends_additional_params() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));
	let req = ExchangeRequest {
		subject_token: "subj".to_string().into(),
		subject_token_type: OAuthTokenType::AccessToken,
		actor: None,
		extra_params: vec![
			("vendor_id".into(), "v1".into()),
			("org".into(), "o2".into()),
		],
		chained_extra_params: vec![],
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
	let err = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap_err();
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
	let auth = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap();
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
		path: "token".into(),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"must start with /"
)]
#[case::jwt_bearer_actor_token(
	OAuthTokenExchangeAuth {
		grant_type: OAuthGrantType::JwtBearer,
		actor_token: Some(ActorTokenSpec {
			source: AuthorizationLocation::default(),
			token_type: OAuthTokenType::default(),
			enforce_may_act: false,
		}),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"actor_token"
)]
#[case::enforce_may_act_non_jwt_actor_token(
	OAuthTokenExchangeAuth {
		actor_token: Some(ActorTokenSpec {
			source: AuthorizationLocation::Header {
				name: ::http::HeaderName::from_static("x-actor-token"),
				prefix: None,
			},
			token_type: OAuthTokenType::AccessToken,
			enforce_may_act: true,
		}),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"requires actor_token.token_type"
)]
#[case::basic_without_secret(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::ClientSecretBasic {
				client_secret: "".into(),
			},
		}),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"client_secret"
)]
#[case::empty_client_id(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth {
			client_id: String::new(),
			method: OAuthClientAuthMethod::ClientSecretPost {
				client_secret: Some("secret".into()),
			},
		}),
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"client_id"
)]
#[case::empty_client_secret(
	OAuthTokenExchangeAuth {
		client_auth: Some(OAuthClientAuth {
			client_id: "gateway-client".into(),
			method: OAuthClientAuthMethod::ClientSecretPost {
				client_secret: Some("".into()),
			},
		}),
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
#[case::plain_oauth_id_jag(
	OAuthTokenExchangeAuth {
		requested_token_type: Some(OAuthTokenType::IdJag),
		audiences: vec!["https://resource-as.example".into()],
		..base_auth(Arc::new(SimpleBackendReference::Invalid))
	},
	"backendAuth.crossAppAccess"
)]
#[case::expression_output_location(
	OAuthTokenExchangeAuth {
		authorization_location: AuthorizationLocation::Expression(Arc::new(cel::Expression::new_strict(r#""token""#).unwrap())),
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
		let auth = OAuthTokenExchangeAuth::from_proto(
			proto::OAuthTokenExchange {
				requested_token_type: Some(token_type.to_string()),
				..Default::default()
			},
			&mut Diagnostics::default(),
		)
		.unwrap();
		assert_eq!(
			auth.requested_token_type,
			Some(token_type_from_urn(token_type))
		);
	}
}

#[test]
fn private_key_jwt_client_auth_from_proto() {
	let auth = OAuthClientAuth::try_from(proto::OAuthClientAuth {
		client_id: "gateway-client".to_string(),
		method: proto::o_auth_client_auth::Method::PrivateKeyJwt as i32,
		private_key_jwt: Some(proto::o_auth_client_auth::PrivateKeyJwt {
			signing_key: TEST_EC_PRIVATE_KEY_PEM.to_string(),
			alg: proto::o_auth_client_auth::private_key_jwt::SigningAlg::Es256 as i32,
			kid: Some("kid-1".to_string()),
			assertion_audience: "https://issuer.example/token".to_string(),
		}),
		..Default::default()
	})
	.unwrap();

	assert_eq!(auth.client_id, "gateway-client");
	match auth.method {
		OAuthClientAuthMethod::PrivateKeyJwt(private_key) => {
			let serialized = serde_json::to_value(private_key).unwrap();
			assert_eq!(serialized["alg"].as_str(), Some("ES256"));
			assert_eq!(serialized["kid"].as_str(), Some("kid-1"));
			assert_eq!(
				serialized["assertionAudience"].as_str(),
				Some("https://issuer.example/token")
			);
		},
		other => panic!("expected privateKeyJwt client auth, got {other:?}"),
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
#[case::id_jag_unsupported_over_xds(
	proto::OAuthTokenExchange {
		requested_token_type: Some(TOKEN_TYPE_ID_JAG.to_string()),
		..Default::default()
	},
	"only supported by local backendAuth.crossAppAccess"
)]
#[case::invalid_subject_token_type(
	proto::OAuthTokenExchange {
		subject_token: Some(proto::o_auth_token_exchange::TokenSpec {
			token_type: "not a uri".to_string(),
			..Default::default()
		}),
		..Default::default()
	},
	"unsupported subject_token.token_type"
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
		client_auth: Some(proto::OAuthClientAuth {
			client_id: String::new(),
			client_secret: Some("s".to_string()),
			method: proto::o_auth_client_auth::Method::ClientSecretPost as i32,
			..Default::default()
		}),
		..Default::default()
	},
	"client_id"
)]
#[case::empty_client_secret(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::OAuthClientAuth {
			client_id: "gateway-client".to_string(),
			client_secret: Some(String::new()),
			method: proto::o_auth_client_auth::Method::ClientSecretPost as i32,
			..Default::default()
		}),
		..Default::default()
	},
	"client_secret"
)]
#[case::private_key_jwt_missing_settings(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::OAuthClientAuth {
			client_id: "gateway-client".to_string(),
			method: proto::o_auth_client_auth::Method::PrivateKeyJwt as i32,
			..Default::default()
		}),
		..Default::default()
	},
	"private_key_jwt settings are required"
)]
#[case::private_key_jwt_with_client_secret(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::OAuthClientAuth {
			client_id: "gateway-client".to_string(),
			client_secret: Some("secret".to_string()),
			method: proto::o_auth_client_auth::Method::PrivateKeyJwt as i32,
			private_key_jwt: Some(proto::o_auth_client_auth::PrivateKeyJwt {
				signing_key: TEST_EC_PRIVATE_KEY_PEM.to_string(),
				alg: proto::o_auth_client_auth::private_key_jwt::SigningAlg::Es256 as i32,
				assertion_audience: "https://issuer.example/token".to_string(),
				..Default::default()
			}),
		}),
		..Default::default()
	},
	"must not set client_secret"
)]
#[case::private_key_jwt_settings_with_secret_method(
	proto::OAuthTokenExchange {
		client_auth: Some(proto::OAuthClientAuth {
			client_id: "gateway-client".to_string(),
			client_secret: Some("secret".to_string()),
			method: proto::o_auth_client_auth::Method::ClientSecretPost as i32,
			private_key_jwt: Some(proto::o_auth_client_auth::PrivateKeyJwt {
				signing_key: TEST_EC_PRIVATE_KEY_PEM.to_string(),
				alg: proto::o_auth_client_auth::private_key_jwt::SigningAlg::Es256 as i32,
				assertion_audience: "https://issuer.example/token".to_string(),
				..Default::default()
			}),
		}),
		..Default::default()
	},
	"requires the PRIVATE_KEY_JWT method"
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
#[case::enforce_may_act_non_jwt_actor_token(
	proto::OAuthTokenExchange {
		actor_token: Some(proto::o_auth_token_exchange::ActorToken {
			source: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::Header(
					proto::authorization_location::Header {
						name: "x-actor-token".to_string(),
						prefix: None,
					},
				)),
			}),
			token_type: TOKEN_TYPE_ACCESS.to_string(),
			enforce_may_act: true,
		}),
		..Default::default()
	},
	"requires actor_token.token_type"
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
	let cfg = token_cache_config_from_proto(Some(proto::o_auth_token_exchange::TokenCache {
		in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
			max_entries: Some(0),
			default_ttl: None,
		}),
	}))
	.unwrap();

	assert!(cfg.into_cache().is_none());

	let auth = OAuthTokenExchangeAuth::from_proto(
		proto::OAuthTokenExchange {
			cache: Some(proto::o_auth_token_exchange::TokenCache {
				in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
					max_entries: Some(0),
					default_ttl: None,
				}),
			}),
			..Default::default()
		},
		&mut Diagnostics::default(),
	)
	.unwrap();

	assert!(auth.cache.is_none());
}

#[test]
fn cache_from_proto_defaults_to_in_memory_cache() {
	let cfg = token_cache_config_from_proto(None).unwrap();

	assert_eq!(cfg.max_entries, None);
	assert_eq!(cfg.default_ttl, None);
}

#[test]
fn in_memory_cache_from_proto_uses_default_ttl_and_capacity_defaults() {
	let cfg = token_cache_config_from_proto(Some(proto::o_auth_token_exchange::TokenCache {
		in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
			max_entries: None,
			default_ttl: Some(prost_types::Duration {
				seconds: 42,
				nanos: 0,
			}),
		}),
	}))
	.unwrap();

	assert_eq!(cfg.max_entries, None);
	assert_eq!(cfg.default_ttl, Some(Duration::from_secs(42)));
}

#[test]
fn in_memory_cache_from_proto_uses_default_ttl_for_negative_default_ttl() {
	let cfg = token_cache_config_from_proto(Some(proto::o_auth_token_exchange::TokenCache {
		in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
			max_entries: None,
			default_ttl: Some(prost_types::Duration {
				seconds: -1,
				nanos: 0,
			}),
		}),
	}))
	.unwrap();

	assert_eq!(cfg.default_ttl, None);
}

#[test]
fn in_memory_cache_from_proto_accepts_large_default_ttl() {
	let cfg = token_cache_config_from_proto(Some(proto::o_auth_token_exchange::TokenCache {
		in_memory: Some(proto::o_auth_token_exchange::token_cache::InMemory {
			max_entries: None,
			default_ttl: Some(prost_types::Duration {
				seconds: i64::MAX,
				nanos: 999_999_999,
			}),
		}),
	}))
	.unwrap();

	assert_eq!(
		cfg.default_ttl,
		Some(Duration::from_secs(i64::MAX as u64) + Duration::from_nanos(999_999_999))
	);
}

#[rstest]
#[case(TOKEN_TYPE_ACCESS, true)]
#[case(TOKEN_TYPE_JWT, true)]
#[case(TOKEN_TYPE_ID, true)]
#[case("urn:ietf:params:oauth:token-type:saml2", true)]
#[case("urn:company:domain:human", true)]
#[case("not a uri", false)]
#[case("https://tokens.example/custom#fragment", false)]
fn oauth_token_type_from_urn_cases(#[case] token_type: &str, #[case] expected: bool) {
	assert_eq!(OAuthTokenType::from_urn(token_type).is_some(), expected);
}

#[tokio::test]
async fn sends_actor_token() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let a = auth(endpoint(&mock));
	let req = ExchangeRequest {
		subject_token: "subj".to_string().into(),
		subject_token_type: OAuthTokenType::AccessToken,
		actor: Some(("actor-tok".to_string().into(), OAuthTokenType::Jwt)),
		extra_params: vec![],
		chained_extra_params: vec![],
	};

	fetch_token(&policy_client(), &a, req).await.unwrap();

	let pairs = sent_form_params(&mock).await;
	assert_eq!(pairs["actor_token"], "actor-tok");
	assert_eq!(pairs["actor_token_type"], TOKEN_TYPE_JWT);
}

fn actor_token_with_header(enforce_may_act: bool) -> ActorTokenSpec {
	ActorTokenSpec {
		source: AuthorizationLocation::Header {
			name: ::http::HeaderName::from_static("x-actor-token"),
			prefix: None,
		},
		token_type: OAuthTokenType::Jwt,
		enforce_may_act,
	}
}

#[test]
fn actor_token_does_not_fallback_to_subject_claims() {
	let subject = "subject-token";
	let mut req = incoming_request();
	req
		.extensions_mut()
		.insert(claims_with_may_act(subject, json!({"sub": "actor-a"})));

	let err = actor_token_from_request(&actor_token_with_header(false), &req, subject).unwrap_err();
	assert!(matches!(err, ProxyError::InvalidRequest));
}

fn request_with_actor_header(subject: &str, actor: &str) -> crate::http::Request {
	::http::Request::builder()
		.method(::http::Method::GET)
		.uri("http://upstream/")
		.header(::http::header::AUTHORIZATION, format!("Bearer {subject}"))
		.header("x-actor-token", actor)
		.body(Body::empty())
		.unwrap()
}

fn backend_auth_requiring_may_act(mock: &MockServer) -> crate::http::auth::BackendAuth {
	let a = OAuthTokenExchangeAuth {
		actor_token: Some(actor_token_with_header(true)),
		..auth(endpoint(mock))
	};
	crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a))
}

#[test]
fn actor_token_authorization_from_proto() {
	let proto = proto::OAuthTokenExchange {
		actor_token: Some(proto::o_auth_token_exchange::ActorToken {
			source: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::Header(
					proto::authorization_location::Header {
						name: "x-actor-token".to_string(),
						prefix: None,
					},
				)),
			}),
			enforce_may_act: true,
			token_type: TOKEN_TYPE_JWT.to_string(),
		}),
		..Default::default()
	};
	let auth = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap();
	assert!(auth.actor_token.unwrap().enforce_may_act);
}

#[rstest]
#[case::exact_match(json!({"sub": "actor-a"}), "actor-a", true)]
#[case::actor_in_allowed_list(json!({"sub": ["actor-a", "actor-b"]}), "actor-b", true)]
#[case::actor_not_allowed(json!({"sub": "actor-a"}), "actor-b", false)]
#[case::non_object_may_act_claim(json!("actor-a"), "actor-a", false)]
#[tokio::test]
async fn enforce_may_act_checks_validated_subject_claims(
	#[case] may_act: serde_json::Value,
	#[case] actor_sub: &str,
	#[case] expect_authorized: bool,
) {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let backend_auth = backend_auth_requiring_may_act(&mock);

	let subject = jwt_with_claims(&json!({"sub": "subject-a"}));
	let actor = jwt_with_claims(&json!({"sub": actor_sub}));
	let mut req = request_with_actor_header(&subject, &actor);
	req
		.extensions_mut()
		.insert(claims_with_may_act(&subject, may_act));

	let result =
		crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req).await;
	if expect_authorized {
		result.unwrap();
	} else {
		assert!(matches!(
			result.unwrap_err(),
			ProxyError::AuthorizationFailed
		));
		assert!(mock.received_requests().await.unwrap().is_empty());
	}
}

#[tokio::test]
async fn enforce_may_act_ignores_validated_claims_for_a_different_subject_token() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let backend_auth = backend_auth_requiring_may_act(&mock);

	let subject = jwt_with_claims(&json!({"may_act": {"sub": "actor-a"}}));
	let actor = jwt_with_claims(&json!({"sub": "actor-b"}));
	let mut req = request_with_actor_header(&subject, &actor);
	req.extensions_mut().insert(claims_with_may_act(
		"some-other-subject",
		json!({"sub": "actor-b"}),
	));

	let err = crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap_err();
	assert!(matches!(err, ProxyError::AuthorizationFailed));
	assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn enforce_may_act_falls_back_to_unvalidated_subject_token_without_jwt_policy() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
	let backend_auth = backend_auth_requiring_may_act(&mock);

	let subject = jwt_with_claims(&json!({"may_act": {"sub": "actor-a"}}));
	let mut req = request_with_actor_header(&subject, &jwt_with_claims(&json!({"sub": "actor-a"})));

	crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
		.await
		.unwrap();
}

#[tokio::test]
async fn rejects_na_token_type_as_non_bearer() {
	let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
		"access_token": "delegated-token",
		"token_type": "N_A",
		"issued_token_type": TOKEN_TYPE_ACCESS,
	})))
	.await;
	let a = auth(endpoint(&mock));

	let err = fetch_token(
		&policy_client(),
		&a,
		exchange_req("subj", TOKEN_TYPE_ACCESS),
	)
	.await
	.unwrap_err();
	assert!(err.to_string().contains("unsupported token_type: N_A"));
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
	let auth = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap();
	assert!(
		matches!(&auth.subject_token.source, AuthorizationLocation::Header { name, .. } if name.as_str() == "x-subject")
	);
	// Empty proto token_type defaults to access_token.
	assert_eq!(auth.subject_token.token_type, OAuthTokenType::AccessToken);
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
	let auth = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap();
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
	let auth = OAuthTokenExchangeAuth::from_proto(proto, &mut Diagnostics::default()).unwrap();
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
			token_type: OAuthTokenType::Jwt,
			enforce_may_act: false,
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
