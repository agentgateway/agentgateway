use std::sync::Arc;
use std::time::Duration;

use ::http::{Method, Request as HttpRequest, header};
use base64::Engine as _;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::client;
use crate::http::jwt;
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

fn provider_endpoint(value: impl AsRef<str>) -> ProviderEndpoint {
	value.as_ref().parse().expect("provider endpoint")
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
			authorization_endpoint: provider_endpoint("https://issuer.example.com/authorize"),
			token_endpoint: provider_endpoint("https://issuer.example.com/token"),
			id_token_validator: test_id_token_validator(),
		}),
		client: ClientConfig {
			client_id: TEST_CLIENT_ID.into(),
			client_secret: SecretString::new("client-secret".into()),
			token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
		},
		redirect_uri: test_redirect_uri(),
		session,
		scopes: vec!["openid".into(), "profile".into()],
	}
}

fn test_callback_policy(token_endpoint: ProviderEndpoint) -> OidcPolicy {
	let mut policy = test_policy();
	policy.provider = Arc::new(Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: provider_endpoint("https://issuer.example.com/authorize"),
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
		authorization_endpoint: Some(provider_endpoint("https://issuer.example.com/authorize")),
		token_endpoint: Some(provider_endpoint("https://issuer.example.com/token")),
		jwks: Some(test_jwks_inline()),
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: test_redirect_uri().redirect_uri,
		scopes: vec!["profile".into(), "email".into()],
	}
}

fn translated_policy_id(name: &str) -> PolicyId {
	PolicyId::policy(name)
}

async fn compile_local_policy(
	config: LocalOidcConfig,
	policy_id: PolicyId,
) -> Result<OidcPolicy, Error> {
	config
		.compile(test_client(), policy_id, &test_oidc_cookie_encoder())
		.await
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

#[test]
fn explicit_provider_config_rejects_relative_endpoints_during_deserialization() {
	let err = serde_json::from_value::<LocalOidcConfig>(json!({
		"issuer": TEST_ISSUER,
		"authorizationEndpoint": "/authorize",
		"tokenEndpoint": "https://issuer.example.com/token",
		"jwks": serde_json::to_string(&test_jwks()).expect("jwks"),
		"clientId": TEST_CLIENT_ID,
		"clientSecret": "client-secret",
		"redirectURI": "http://localhost:3000/oauth/callback"
	}))
	.expect_err("relative authorization endpoint should be rejected");

	assert!(err.to_string().contains("must be an absolute http(s) URL"));
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
async fn apply_redirects_unauthenticated_requests_to_login() {
	let cases = [
		(
			"html request",
			"http://127.0.0.1/private",
			"text/html",
			"http://127.0.0.1/oauth/callback",
			Some("redirect_uri=http%3A%2F%2F127.0.0.1%2Foauth%2Fcallback"),
		),
		(
			"json request",
			"https://app.example.com/private",
			"application/json",
			"https://app.example.com/oauth/callback",
			None,
		),
		(
			"callback path without query",
			"https://app.example.com/oauth/callback",
			"text/html",
			"https://app.example.com/oauth/callback",
			None,
		),
	];

	for (name, request_uri, accept, redirect_uri, expected_fragment) in cases {
		let mut policy = test_policy();
		policy.redirect_uri = RedirectUri::parse(redirect_uri.to_string()).expect("redirect uri");
		let mut req = request(Method::GET, request_uri, Some(accept));

		let response = policy
			.apply(None, &mut req, policy_client())
			.await
			.expect(name);
		let response = response.direct_response.expect("redirect response");
		assert_eq!(response.status(), ::http::StatusCode::FOUND, "{name}");

		let location = response
			.headers()
			.get(header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location utf8");
		assert!(
			location.starts_with("https://issuer.example.com/authorize?"),
			"{name}"
		);
		if let Some(expected_fragment) = expected_fragment {
			assert!(location.contains(expected_fragment), "{name}");
		}
	}
}

#[tokio::test]
async fn token_endpoint_auth_modes_shape_exchange_requests() {
	#[derive(Copy, Clone)]
	enum Expectation {
		AuthorizationHeader,
		FormBodyCredentials,
	}

	let cases = [
		(
			"client_secret_basic",
			TokenEndpointAuth::ClientSecretBasic,
			"client:id",
			"s e:c",
			Expectation::AuthorizationHeader,
		),
		(
			"client_secret_post",
			TokenEndpointAuth::ClientSecretPost,
			"client-id",
			"client-secret",
			Expectation::FormBodyCredentials,
		),
	];

	for (name, token_endpoint_auth, client_id, client_secret, expectation) in cases {
		let mock = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"id_token": signed_id_token(TEST_NONCE)
			})))
			.mount(&mock)
			.await;

		let provider = Provider {
			issuer: TEST_ISSUER.into(),
			authorization_endpoint: provider_endpoint("https://issuer.example.com/authorize"),
			token_endpoint: provider_endpoint(format!("{}/token", mock.uri())),
			id_token_validator: test_id_token_validator(),
		};
		let client_config = ClientConfig {
			client_id: client_id.into(),
			client_secret: SecretString::new(client_secret.into()),
			token_endpoint_auth,
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
		.expect(name);
		assert!(response.id_token.is_some(), "{name}");

		let request = &mock.received_requests().await.expect("requests")[0];
		let body = String::from_utf8(request.body.clone()).expect("utf8 body");
		match expectation {
			Expectation::AuthorizationHeader => {
				let encoded_client_id = url::form_urlencoded::Serializer::new(String::new())
					.append_pair("", client_id)
					.finish();
				let encoded_client_secret = url::form_urlencoded::Serializer::new(String::new())
					.append_pair("", client_secret)
					.finish();
				let expected_auth = format!(
					"Basic {}",
					base64::engine::general_purpose::STANDARD.encode(format!(
						"{}:{}",
						encoded_client_id.trim_start_matches('='),
						encoded_client_secret.trim_start_matches('=')
					))
				);
				assert_eq!(
					request
						.headers
						.get("authorization")
						.expect("authorization header")
						.to_str()
						.expect("authorization header value"),
					expected_auth.as_str(),
					"{name}"
				);
				assert!(!body.contains("client_id="), "{name}");
				assert!(!body.contains("client_secret="), "{name}");
			},
			Expectation::FormBodyCredentials => {
				assert!(!request.headers.contains_key("authorization"), "{name}");
				assert!(body.contains("client_id=client-id"), "{name}");
				assert!(body.contains("client_secret=client-secret"), "{name}");
			},
		}
	}
}

#[tokio::test]
async fn token_exchange_bounds_transport_failures() {
	#[derive(Copy, Clone)]
	enum FailureMode {
		Timeout,
		OversizedBody,
	}

	let cases = [
		("timeout", FailureMode::Timeout),
		("oversized body", FailureMode::OversizedBody),
	];

	for (name, failure_mode) in cases {
		let mock = MockServer::start().await;
		let response = match failure_mode {
			FailureMode::Timeout => ResponseTemplate::new(200).set_delay(Duration::from_millis(200)),
			FailureMode::OversizedBody => {
				ResponseTemplate::new(200).set_body_string("x".repeat(70 * 1024))
			},
		};
		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(response)
			.mount(&mock)
			.await;

		let provider = Provider {
			issuer: TEST_ISSUER.into(),
			authorization_endpoint: provider_endpoint("https://issuer.example.com/authorize"),
			token_endpoint: provider_endpoint(format!("{}/token", mock.uri())),
			id_token_validator: test_id_token_validator(),
		};
		let client_config = ClientConfig {
			client_id: TEST_CLIENT_ID.into(),
			client_secret: SecretString::new("client-secret".into()),
			token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
		};

		let err = match failure_mode {
			FailureMode::Timeout => {
				provider::exchange_code_with_timeout(
					policy_client(),
					&provider,
					&client_config,
					"https://app.example.com/oauth/callback",
					"code",
					&SecretString::new("verifier".into()),
					Duration::from_millis(50),
				)
				.await
			},
			FailureMode::OversizedBody => {
				provider::exchange_code(
					policy_client(),
					&provider,
					&client_config,
					"https://app.example.com/oauth/callback",
					"code",
					&SecretString::new("verifier".into()),
				)
				.await
			},
		}
		.expect_err(name);
		assert!(matches!(err, Error::TokenExchangeFailed(_)), "{name}");
	}
}

#[tokio::test]
async fn callback_rejects_invalid_transaction_state() {
	let cases = [
		(
			"missing transaction",
			None,
			"https://app.example.com/oauth/callback?code=auth-code&state=test-state",
			Error::MissingTransaction,
		),
		(
			"csrf mismatch",
			Some(("expected-state", TEST_NONCE, "/protected")),
			"https://app.example.com/oauth/callback?code=auth-code&state=wrong-state",
			Error::CsrfMismatch,
		),
	];

	for (name, transaction, uri, expected_error) in cases {
		let policy = test_policy();
		let mut req = request(Method::GET, uri, Some("text/html"));
		if let Some((state, nonce, original_uri)) = transaction {
			let encoded = encoded_transaction(&policy, state, nonce, original_uri, now_unix() + 300);
			add_cookie(
				&mut req,
				format!("{}={encoded}", policy.session.transaction_cookie_name),
			);
		}

		let err = policy
			.apply(None, &mut req, policy_client())
			.await
			.expect_err(name);
		match expected_error {
			Error::MissingTransaction => assert!(matches!(err, Error::MissingTransaction), "{name}"),
			Error::CsrfMismatch => assert!(matches!(err, Error::CsrfMismatch), "{name}"),
			_ => unreachable!("unexpected test error"),
		}
	}
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

	let policy = test_callback_policy(provider_endpoint(format!("{}/token", mock.uri())));
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

	let policy = test_callback_policy(provider_endpoint(format!("{}/token", mock.uri())));
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

#[test]
fn oidc_errors_use_error_specific_status_codes() {
	let cases = [
		(
			"authentication required",
			Error::AuthenticationRequired,
			::http::StatusCode::UNAUTHORIZED,
		),
		(
			"missing transaction",
			Error::MissingTransaction,
			::http::StatusCode::BAD_REQUEST,
		),
		(
			"nonce mismatch",
			Error::NonceMismatch,
			::http::StatusCode::BAD_REQUEST,
		),
		(
			"token exchange failure",
			Error::TokenExchangeFailed(anyhow::anyhow!("boom")),
			::http::StatusCode::INTERNAL_SERVER_ERROR,
		),
	];

	for (name, error, expected_status) in cases {
		assert_eq!(
			crate::proxy::ProxyError::OidcFailure(error)
				.into_response()
				.status(),
			expected_status,
			"{name}"
		);
	}
}

#[tokio::test]
async fn local_oidc_config_compiles_supported_provider_sources() {
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

	let cases = [
		(
			"discovery",
			LocalOidcConfig {
				issuer: mock.uri(),
				discovery: None,
				authorization_endpoint: None,
				token_endpoint: None,
				jwks: None,
				client_id: TEST_CLIENT_ID.into(),
				client_secret: SecretString::new("client-secret".into()),
				redirect_uri: "http://localhost:3000/oauth/callback".into(),
				scopes: vec![],
			},
			provider_endpoint(format!("{}/authorize", mock.uri())),
			provider_endpoint(format!("{}/token", mock.uri())),
			TokenEndpointAuth::ClientSecretPost,
		),
		(
			"explicit",
			explicit_local_oidc_config(),
			provider_endpoint("https://issuer.example.com/authorize"),
			provider_endpoint("https://issuer.example.com/token"),
			TokenEndpointAuth::ClientSecretBasic,
		),
	];

	for (
		name,
		config,
		expected_authorization_endpoint,
		expected_token_endpoint,
		expected_token_endpoint_auth,
	) in cases
	{
		let policy = compile_local_policy(config, translated_policy_id(name))
			.await
			.expect(name);

		assert_eq!(
			policy.provider.authorization_endpoint, expected_authorization_endpoint,
			"{name}"
		);
		assert_eq!(
			policy.provider.token_endpoint, expected_token_endpoint,
			"{name}"
		);
		assert_eq!(
			policy.client.token_endpoint_auth, expected_token_endpoint_auth,
			"{name}"
		);
	}
}

#[tokio::test]
async fn discovery_rejects_relative_provider_endpoints() {
	let mock = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": mock.uri(),
			"authorization_endpoint": "/authorize",
			"token_endpoint": format!("{}/token", mock.uri()),
			"jwks_uri": format!("{}/jwks", mock.uri()),
			"token_endpoint_auth_methods_supported": ["client_secret_post"]
		})))
		.mount(&mock)
		.await;

	let policy = LocalOidcConfig {
		issuer: mock.uri(),
		discovery: None,
		authorization_endpoint: None,
		token_endpoint: None,
		jwks: None,
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "http://localhost:3000/oauth/callback".into(),
		scopes: vec![],
	};
	let err = compile_local_policy(policy, translated_policy_id("discovery-relative-endpoints"))
		.await
		.expect_err("relative discovery endpoint should fail");

	assert!(err.to_string().contains("invalid authorization endpoint"));
}

#[tokio::test]
async fn local_oidc_config_rejects_ambiguous_provider_source_configuration() {
	let cases = [
		(
			"partial explicit",
			LocalOidcConfig {
				issuer: TEST_ISSUER.into(),
				discovery: None,
				authorization_endpoint: None,
				token_endpoint: Some(provider_endpoint("https://issuer.example.com/token")),
				jwks: Some(test_jwks_inline()),
				client_id: TEST_CLIENT_ID.into(),
				client_secret: SecretString::new("client-secret".into()),
				redirect_uri: "http://localhost:3000/oauth/callback".into(),
				scopes: vec![],
			},
			"authorizationEndpoint, tokenEndpoint, and jwks must either all be set or all be omitted",
		),
		(
			"explicit with discovery override",
			LocalOidcConfig {
				discovery: Some(FileInlineOrRemote::Remote {
					url: "https://example.invalid/should-not-be-called"
						.parse()
						.expect("discovery override url"),
				}),
				..explicit_local_oidc_config()
			},
			"oidc discovery must be omitted when authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly",
		),
	];

	for (name, config, expected_error_fragment) in cases {
		let err = compile_local_policy(config, translated_policy_id(name))
			.await
			.expect_err(name);
		assert!(err.to_string().contains(expected_error_fragment), "{name}");
	}
}
