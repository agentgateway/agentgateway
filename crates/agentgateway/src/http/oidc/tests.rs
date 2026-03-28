use std::sync::Arc;
use std::time::Duration;

use ::http::{Method, Request as HttpRequest, Uri, header};
use base64::Engine as _;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::json;
use wiremock::matchers::{header as match_header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::client;
use crate::http::jwt;
use crate::proxy::ProxyError;
use crate::serdes::FileInlineOrRemote;
use crate::test_helpers::proxymock::setup_proxy_test;

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgltxBTVDLg7C6vE1T
7OtwJIZ/dpm8ygE2MBTjPCY3hgahRANCAARYzu50EeBrT0rELmTGroaGtn0zdjxL
1lOGr9fGw5wOGcXO0+Gn5F5sIxGyTM0FwnUHFNz2SoixZR5dtxhNc+Lo
-----END PRIVATE KEY-----
";
const TEST_KEY_ID: &str = "kid-1";
const TEST_ISSUER: &str = "https://issuer.example.com";
const TEST_CLIENT_ID: &str = "client-id";
const TEST_NONCE: &str = "nonce";

#[derive(Serialize)]
struct TestIdTokenClaims<'a> {
	iss: &'a str,
	aud: &'a str,
	exp: u64,
	nonce: &'a str,
	sub: &'a str,
}

fn test_client() -> client::Client {
	client::Client::new(
		&client::Config {
			resolver_cfg: ResolverConfig::default(),
			resolver_opts: ResolverOpts::default(),
		},
		None,
		Default::default(),
		None,
	)
}

fn policy_client() -> crate::proxy::httpproxy::PolicyClient {
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	crate::proxy::httpproxy::PolicyClient {
		inputs: proxy.inputs(),
	}
}

fn test_jwks() -> JwkSet {
	serde_json::from_value(json!({
		"keys": [{
			"use": "sig",
			"kty": "EC",
			"kid": TEST_KEY_ID,
			"crv": "P-256",
			"alg": "ES256",
			"x": "WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk",
			"y": "xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"
		}]
	}))
	.expect("jwks json")
}

fn test_jwks_inline() -> FileInlineOrRemote {
	FileInlineOrRemote::Inline(serde_json::to_string(&test_jwks()).expect("jwks"))
}

fn test_id_token_validator() -> jwt::Jwt {
	let provider = jwt::Provider::from_jwks(
		test_jwks(),
		TEST_ISSUER.to_string(),
		Some(vec![TEST_CLIENT_ID.to_string()]),
		jwt::JWTValidationOptions::default(),
	)
	.expect("validator provider");
	jwt::Jwt::from_providers(vec![provider], jwt::Mode::Strict)
}

fn test_redirect_uri() -> RedirectUri {
	RedirectUri::parse("https://app.example.com/oauth/callback".into()).expect("redirect uri")
}

fn test_oidc_cookie_encoder() -> crate::http::sessionpersistence::Encoder {
	crate::http::sessionpersistence::Encoder::aes(
		"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
	)
	.expect("aes encoder")
}

fn test_policy() -> OidcPolicy {
	let session = SessionConfig {
		cookie_name: "agw_oidc_s_test".into(),
		transaction_cookie_name: "agw_oidc_t_test".into(),
		same_site: SameSiteMode::Lax,
		secure: CookieSecureMode::Never,
		ttl: Duration::from_secs(3600),
		transaction_ttl: Duration::from_secs(300),
		encoder: test_oidc_cookie_encoder(),
	};

	OidcPolicy {
		policy_id: "policy".into(),
		provider: Arc::new(Provider {
			issuer: TEST_ISSUER.into(),
			authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
			token_endpoint: Uri::from_static("https://issuer.example.com/token"),
			id_token_validator: test_id_token_validator(),
		}),
		client: ClientConfig {
			client_id: TEST_CLIENT_ID.into(),
			client_secret: SecretString::new("client-secret".into()),
			token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
		},
		redirect_uri: test_redirect_uri(),
		unauthenticated_action: UnauthenticatedAction::Auto,
		session,
		scopes: vec!["openid".into(), "profile".into()],
	}
}

fn test_callback_policy(token_endpoint: Uri) -> OidcPolicy {
	let mut policy = test_policy();
	policy.provider = Arc::new(Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint,
		id_token_validator: test_id_token_validator(),
	});
	policy
}

fn encoded_transaction(
	policy: &OidcPolicy,
	state: &str,
	nonce: &str,
	original_uri: &str,
	expires_at_unix: u64,
) -> String {
	policy
		.session
		.encode_transaction(&TransactionState {
			policy_id: policy.policy_id.clone(),
			csrf_state: state.into(),
			nonce: nonce.into(),
			pkce_verifier: SecretString::new("pkce-verifier".into()),
			original_uri: original_uri.into(),
			expires_at_unix,
		})
		.expect("encode transaction")
}

fn signed_id_token(nonce: &str) -> String {
	let mut header = Header::new(Algorithm::ES256);
	header.kid = Some(TEST_KEY_ID.into());
	jsonwebtoken::encode(
		&header,
		&TestIdTokenClaims {
			iss: TEST_ISSUER,
			aud: TEST_CLIENT_ID,
			exp: now_unix() + 600,
			nonce,
			sub: "user-1",
		},
		&EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("encoding key"),
	)
	.expect("signed id token")
}

fn request(method: Method, uri: &str, accept: Option<&str>) -> crate::http::Request {
	let mut builder = HttpRequest::builder().method(method).uri(uri);
	if let Some(accept) = accept {
		builder = builder.header(header::ACCEPT, accept);
	}
	builder.body(crate::http::Body::empty()).expect("request")
}

fn add_cookie(req: &mut crate::http::Request, cookie: String) {
	req
		.headers_mut()
		.append(header::COOKIE, cookie.parse().expect("cookie header"));
}

fn explicit_local_oidc_config() -> LocalOidcConfig {
	LocalOidcConfig {
		issuer: TEST_ISSUER.into(),
		discovery: None,
		authorization_endpoint: Some(Uri::from_static("https://issuer.example.com/authorize")),
		token_endpoint: Some(Uri::from_static("https://issuer.example.com/token")),
		jwks: Some(test_jwks_inline()),
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: test_redirect_uri().redirect_uri,
		scopes: vec!["profile".into(), "email".into()],
		unauthenticated_action: UnauthenticatedAction::Auto,
	}
}

fn translated_policy_id(name: &str) -> PolicyId {
	PolicyId::policy(name)
}

#[test]
fn redirect_uri_rejects_ambiguous_values() {
	for raw in [
		"https://app.example.com",
		"https://app.example.com/",
		"https://app.example.com/oauth/../callback",
		"https://app.example.com/oauth/%2fcallback",
		"https://app.example.com/oauth/callback?x=1",
	] {
		assert!(RedirectUri::parse(raw.to_string()).is_err(), "{raw}");
	}
}

#[test]
fn redirect_uri_accepts_valid_absolute_http_callbacks() {
	let redirect =
		RedirectUri::parse("http://app.example.com/oauth/callback".to_string()).expect("redirect uri");

	assert_eq!(redirect.host, "app.example.com");
	assert_eq!(redirect.port, 80);
	assert!(!redirect.https);
	assert_eq!(redirect.callback_path.path(), "/oauth/callback");
}

#[tokio::test]
async fn apply_derives_claims_from_stored_id_token() {
	let policy = test_policy();
	let id_token = signed_id_token(TEST_NONCE);
	let encoded = policy
		.session
		.encode_browser_session(&BrowserSession {
			policy_id: policy.policy_id.clone(),
			raw_id_token: SecretString::new(id_token.clone().into()),
			expires_at_unix: Some(now_unix() + 300),
		})
		.expect("encode session");
	let mut req = request(
		Method::GET,
		"https://app.example.com/protected",
		Some("text/html"),
	);
	add_cookie(
		&mut req,
		format!("{}={encoded}", policy.session.cookie_name),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("browser policy apply");
	assert!(response.direct_response.is_none());
	let claims = req
		.extensions()
		.get::<jwt::Claims>()
		.expect("claims extension");
	assert_eq!(claims.inner.get("sub"), Some(&json!("user-1")));
	assert_eq!(claims.jwt.expose_secret(), id_token);
}

#[tokio::test]
async fn auto_mode_redirects_html_navigation() {
	let mut policy = test_policy();
	policy.redirect_uri = RedirectUri::parse("http://127.0.0.1/oauth/callback".into()).unwrap();
	let mut req = request(Method::GET, "http://127.0.0.1/private", Some("text/html"));

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("apply");
	let response = response.direct_response.expect("redirect response");
	assert_eq!(response.status(), ::http::StatusCode::FOUND);
	let location = response
		.headers()
		.get(header::LOCATION)
		.unwrap()
		.to_str()
		.unwrap();
	assert!(location.starts_with("https://issuer.example.com/authorize?"));
	assert!(location.contains("redirect_uri=http%3A%2F%2F127.0.0.1%2Foauth%2Fcallback"));
}

#[tokio::test]
async fn auto_mode_returns_unauthorized_for_json_requests() {
	let policy = test_policy();
	let mut req = request(
		Method::GET,
		"https://app.example.com/private",
		Some("application/json"),
	);

	let err = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("oidc should reject API requests without redirect");
	assert!(matches!(err, Error::AuthenticationRequired));
	assert_eq!(err.status_code(), ::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn redirect_mode_redirects_non_html_requests() {
	let mut policy = test_policy();
	policy.unauthenticated_action = UnauthenticatedAction::Redirect;
	let mut req = request(
		Method::GET,
		"https://app.example.com/private",
		Some("application/json"),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("redirect mode should redirect");
	let response = response.direct_response.expect("redirect response");
	assert_eq!(response.status(), ::http::StatusCode::FOUND);
}

#[tokio::test]
async fn deny_mode_rejects_html_navigation() {
	let mut policy = test_policy();
	policy.unauthenticated_action = UnauthenticatedAction::Deny;
	let mut req = request(
		Method::GET,
		"https://app.example.com/private",
		Some("text/html"),
	);

	let err = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("deny mode should reject without redirect");
	assert!(matches!(err, Error::AuthenticationRequired));
}

#[tokio::test]
async fn missing_accept_header_does_not_redirect() {
	let mut req = request(Method::GET, "https://app.example.com/private", None);

	let err = test_policy()
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("missing accept should not redirect");
	assert!(matches!(err, Error::AuthenticationRequired));
}

#[tokio::test]
async fn html_must_be_preferred_over_json_to_redirect() {
	let mut req = request(
		Method::GET,
		"https://app.example.com/private",
		Some("application/json, text/html"),
	);

	let err = test_policy()
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("json-preferred accept should not redirect");
	assert!(matches!(err, Error::AuthenticationRequired));
}

#[tokio::test]
async fn client_secret_basic_uses_form_encoded_credentials() {
	let mock = MockServer::start().await;
	let expected_id_token = signed_id_token(TEST_NONCE);
	let encoded_client_id = url::form_urlencoded::Serializer::new(String::new())
		.append_pair("", "client:id")
		.finish();
	let encoded_client_secret = url::form_urlencoded::Serializer::new(String::new())
		.append_pair("", "s e:c")
		.finish();
	let expected_auth = format!(
		"Basic {}",
		base64::engine::general_purpose::STANDARD.encode(format!(
			"{}:{}",
			encoded_client_id.trim_start_matches('='),
			encoded_client_secret.trim_start_matches('=')
		))
	);
	Mock::given(method("POST"))
		.and(path("/token"))
		.and(match_header("authorization", expected_auth.as_str()))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": expected_id_token
		})))
		.mount(&mock)
		.await;

	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: "client:id".into(),
		client_secret: SecretString::new("s e:c".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let response = provider::exchange_code(
		policy_client(),
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
	)
	.await
	.expect("token exchange");
	assert!(response.id_token.is_some());
}

#[tokio::test]
async fn client_secret_post_sends_credentials_in_form_body() {
	let mock = MockServer::start().await;
	let expected_id_token = signed_id_token(TEST_NONCE);
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": expected_id_token
		})))
		.mount(&mock)
		.await;

	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: "client-id".into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretPost,
	};

	let response = provider::exchange_code(
		policy_client(),
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
	)
	.await
	.expect("token exchange");
	assert!(response.id_token.is_some());

	let request = &mock.received_requests().await.expect("requests")[0];
	let body = String::from_utf8(request.body.clone()).expect("utf8 body");
	assert!(body.contains("client_id=client-id"));
	assert!(body.contains("client_secret=client-secret"));
	assert!(!request.headers.contains_key("authorization"));
}

#[tokio::test]
async fn token_exchange_times_out() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
		.mount(&mock)
		.await;

	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let err = provider::exchange_code_with_timeout(
		policy_client(),
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
		Duration::from_millis(50),
	)
	.await
	.expect_err("timeout expected");
	assert!(matches!(err, Error::TokenExchangeFailed(_)));
}

#[tokio::test]
async fn token_exchange_rejects_oversized_response_body() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(70 * 1024)))
		.mount(&mock)
		.await;

	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let err = provider::exchange_code(
		policy_client(),
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
	)
	.await
	.expect_err("oversized body expected");
	assert!(matches!(err, Error::TokenExchangeFailed(_)));
}

#[tokio::test]
async fn callback_requires_transaction_cookie() {
	let policy = test_policy();
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=auth-code&state=test-state",
		Some("text/html"),
	);

	let err = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("missing transaction cookie");
	assert!(matches!(err, Error::MissingTransaction));
}

#[tokio::test]
async fn callback_rejects_state_mismatch() {
	let policy = test_policy();
	let encoded = encoded_transaction(
		&policy,
		"expected-state",
		TEST_NONCE,
		"/protected",
		now_unix() + 300,
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=auth-code&state=wrong-state",
		Some("text/html"),
	);
	add_cookie(
		&mut req,
		format!("{}={encoded}", policy.session.transaction_cookie_name),
	);

	let err = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect_err("state mismatch");
	assert!(matches!(err, Error::CsrfMismatch));
}

#[tokio::test]
async fn callback_success_sets_session_cookie_and_clears_transaction_cookie() {
	let mock = MockServer::start().await;
	let id_token = signed_id_token(TEST_NONCE);
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": id_token
		})))
		.mount(&mock)
		.await;

	let policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	let encoded = encoded_transaction(
		&policy,
		"test-state",
		TEST_NONCE,
		"/protected",
		now_unix() + 300,
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=auth-code&state=test-state",
		Some("text/html"),
	);
	add_cookie(
		&mut req,
		format!("{}={encoded}", policy.session.transaction_cookie_name),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("callback apply");
	let response = response.direct_response.expect("redirect response");
	assert_eq!(response.status(), ::http::StatusCode::FOUND);
	assert_eq!(
		response.headers().get(header::LOCATION).unwrap(),
		"/protected"
	);
	let cookies: Vec<_> = response
		.headers()
		.get_all(header::SET_COOKIE)
		.iter()
		.map(|h| h.to_str().unwrap().to_string())
		.collect();
	assert!(
		cookies
			.iter()
			.any(|cookie| cookie.starts_with(&policy.session.cookie_name))
	);
	assert!(cookies.iter().any(
		|cookie| cookie.starts_with(&policy.session.transaction_cookie_name)
			&& cookie.contains("Max-Age=0")
	));
}

#[tokio::test]
async fn callback_matching_uses_path_not_redirect_host_or_port() {
	let mock = MockServer::start().await;
	let id_token = signed_id_token(TEST_NONCE);
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": id_token
		})))
		.mount(&mock)
		.await;

	let policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	let encoded = encoded_transaction(
		&policy,
		"test-state",
		TEST_NONCE,
		"/protected",
		now_unix() + 300,
	);
	let mut req = request(
		Method::GET,
		"https://edge.example.net:8443/oauth/callback?code=auth-code&state=test-state",
		Some("text/html"),
	);
	add_cookie(
		&mut req,
		format!("{}={encoded}", policy.session.transaction_cookie_name),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("callback apply");
	assert_eq!(
		response
			.direct_response
			.unwrap()
			.headers()
			.get(header::LOCATION)
			.unwrap(),
		"/protected"
	);
}

#[tokio::test]
async fn callback_path_without_callback_query_falls_back_to_login_redirect() {
	let policy = test_policy();
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback",
		Some("text/html"),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("apply");
	let response = response.direct_response.expect("redirect response");
	assert_eq!(response.status(), ::http::StatusCode::FOUND);
	assert!(
		response
			.headers()
			.get(header::LOCATION)
			.unwrap()
			.to_str()
			.unwrap()
			.starts_with("https://issuer.example.com/authorize?")
	);
}

#[test]
fn oidc_errors_use_error_specific_status_codes() {
	assert_eq!(
		Error::AuthenticationRequired.status_code(),
		::http::StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		Error::MissingTransaction.status_code(),
		::http::StatusCode::BAD_REQUEST
	);
	assert_eq!(
		Error::NonceMismatch.status_code(),
		::http::StatusCode::BAD_REQUEST
	);
	assert_eq!(
		Error::TokenExchangeFailed(anyhow::anyhow!("boom")).status_code(),
		::http::StatusCode::INTERNAL_SERVER_ERROR
	);
	assert!(matches!(
		ProxyError::OidcFailure(Error::MissingTransaction),
		ProxyError::OidcFailure(_)
	));
}

#[tokio::test]
async fn compiled_policy_uses_supplied_identity_for_cookie_derivation() {
	let local = explicit_local_oidc_config();
	let encoder = test_oidc_cookie_encoder();
	let policy_a = local
		.translate(test_client(), &encoder, translated_policy_id("shared"))
		.await
		.expect("first translate");
	let policy_b = explicit_local_oidc_config()
		.translate(test_client(), &encoder, translated_policy_id("shared"))
		.await
		.expect("second translate");
	let policy_c = explicit_local_oidc_config()
		.translate(test_client(), &encoder, translated_policy_id("other"))
		.await
		.expect("third translate");

	assert_eq!(policy_a.policy_id, translated_policy_id("shared"));
	assert_eq!(policy_a.policy_id, policy_b.policy_id);
	assert_ne!(policy_a.policy_id, policy_c.policy_id);
	assert_eq!(policy_a.session.cookie_name, policy_b.session.cookie_name);
	assert_ne!(policy_a.session.cookie_name, policy_c.session.cookie_name);
	assert_eq!(
		policy_a.session.transaction_cookie_name,
		policy_b.session.transaction_cookie_name
	);
	assert_ne!(
		policy_a.session.transaction_cookie_name,
		policy_c.session.transaction_cookie_name
	);
	assert_eq!(policy_a.session.same_site, SameSiteMode::Lax);
	assert_eq!(policy_a.session.secure, CookieSecureMode::Auto);
	assert_eq!(policy_a.session.ttl, Duration::from_secs(3600));
	assert_eq!(policy_a.session.transaction_ttl, Duration::from_secs(300));
	assert_eq!(policy_a.scopes, vec!["openid", "profile", "email"]);
}

#[tokio::test]
async fn issuer_only_oidc_config_uses_discovery() {
	let mock = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": mock.uri(),
			"authorization_endpoint": format!("{}/authorize", mock.uri()),
			"token_endpoint": format!("{}/token", mock.uri()),
			"jwks_uri": format!("{}/jwks", mock.uri()),
			"token_endpoint_auth_methods_supported": ["client_secret_post"]
		})))
		.mount(&mock)
		.await;
	Mock::given(method("GET"))
		.and(path("/jwks"))
		.respond_with(ResponseTemplate::new(200).set_body_json(test_jwks()))
		.mount(&mock)
		.await;

	let policy = LocalOidcConfig {
		issuer: mock.uri(),
		discovery: None,
		authorization_endpoint: None,
		token_endpoint: None,
		jwks: None,
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "http://localhost:3000/oauth/callback".into(),
		scopes: vec![],
		unauthenticated_action: UnauthenticatedAction::Auto,
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("discovery"),
	)
	.await
	.expect("translate");

	assert_eq!(
		policy.provider.authorization_endpoint,
		format!("{}/authorize", mock.uri()).parse::<Uri>().unwrap()
	);
	assert_eq!(
		policy.provider.token_endpoint,
		format!("{}/token", mock.uri()).parse::<Uri>().unwrap()
	);
	assert_eq!(
		policy.client.token_endpoint_auth,
		TokenEndpointAuth::ClientSecretPost
	);
}

#[tokio::test]
async fn fully_explicit_oidc_config_skips_discovery() {
	let policy = explicit_local_oidc_config()
		.translate(
			test_client(),
			&test_oidc_cookie_encoder(),
			translated_policy_id("explicit"),
		)
		.await
		.expect("translate");

	assert_eq!(
		policy.provider.authorization_endpoint,
		Uri::from_static("https://issuer.example.com/authorize")
	);
	assert_eq!(
		policy.provider.token_endpoint,
		Uri::from_static("https://issuer.example.com/token")
	);
	assert_eq!(
		policy.client.token_endpoint_auth,
		TokenEndpointAuth::ClientSecretBasic
	);
}

#[tokio::test]
async fn partial_explicit_provider_config_is_rejected() {
	let err = LocalOidcConfig {
		issuer: TEST_ISSUER.into(),
		discovery: None,
		authorization_endpoint: None,
		token_endpoint: Some(Uri::from_static("https://issuer.example.com/token")),
		jwks: Some(test_jwks_inline()),
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "http://localhost:3000/oauth/callback".into(),
		scopes: vec![],
		unauthenticated_action: UnauthenticatedAction::Auto,
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("partial-explicit"),
	)
	.await
	.expect_err("partial explicit config should fail");

	assert!(err.to_string().contains(
		"authorizationEndpoint, tokenEndpoint, and jwks must either all be set or all be omitted"
	));
}

#[tokio::test]
async fn explicit_provider_config_rejects_discovery_override() {
	let err = LocalOidcConfig {
		discovery: Some(FileInlineOrRemote::Remote {
			url: "https://example.invalid/should-not-be-called"
				.parse()
				.unwrap(),
		}),
		..explicit_local_oidc_config()
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("reject-discovery-override"),
	)
	.await
	.expect_err("fully explicit config should reject discovery override");

	assert!(
		err
			.to_string()
			.contains("oidc discovery must be omitted when authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly")
	);
}

#[tokio::test]
async fn discovery_load_failures_identify_discovery_document_source() {
	let err = LocalOidcConfig {
		issuer: TEST_ISSUER.into(),
		discovery: Some(FileInlineOrRemote::Inline("{".into())),
		authorization_endpoint: None,
		token_endpoint: None,
		jwks: None,
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "http://localhost:3000/oauth/callback".into(),
		scopes: vec![],
		unauthenticated_action: UnauthenticatedAction::Auto,
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("invalid-discovery"),
	)
	.await
	.expect_err("invalid discovery document should fail");

	assert!(
		err
			.to_string()
			.contains("failed to decode oidc discovery response from inline configuration")
	);
}

#[tokio::test]
async fn explicit_remote_jwks_failures_identify_explicit_source() {
	let mock = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/jwks"))
		.respond_with(ResponseTemplate::new(200).set_body_string("{"))
		.mount(&mock)
		.await;

	let err = LocalOidcConfig {
		jwks: Some(FileInlineOrRemote::Remote {
			url: format!("{}/jwks", mock.uri()).parse().unwrap(),
		}),
		..explicit_local_oidc_config()
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("invalid-explicit-jwks"),
	)
	.await
	.expect_err("invalid explicit jwks should fail");

	assert!(
		err
			.to_string()
			.contains("failed to load oidc jwks from explicit jwks source uri")
	);
}

#[tokio::test]
async fn discovered_remote_jwks_failures_identify_discovered_source() {
	let mock = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": mock.uri(),
			"authorization_endpoint": format!("{}/authorize", mock.uri()),
			"token_endpoint": format!("{}/token", mock.uri()),
			"jwks_uri": format!("{}/jwks", mock.uri()),
			"token_endpoint_auth_methods_supported": ["client_secret_basic"]
		})))
		.mount(&mock)
		.await;
	Mock::given(method("GET"))
		.and(path("/jwks"))
		.respond_with(ResponseTemplate::new(200).set_body_string("{"))
		.mount(&mock)
		.await;

	let err = LocalOidcConfig {
		issuer: mock.uri(),
		discovery: None,
		authorization_endpoint: None,
		token_endpoint: None,
		jwks: None,
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "http://localhost:3000/oauth/callback".into(),
		scopes: vec![],
		unauthenticated_action: UnauthenticatedAction::Auto,
	}
	.translate(
		test_client(),
		&test_oidc_cookie_encoder(),
		translated_policy_id("invalid-discovered-jwks"),
	)
	.await
	.expect_err("invalid discovered jwks should fail");

	assert!(
		err
			.to_string()
			.contains("failed to load oidc jwks from discovered jwks source uri")
	);
}
