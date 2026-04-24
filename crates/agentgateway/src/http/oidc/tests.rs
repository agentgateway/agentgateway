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
	jwt::Jwt::from_providers(
		vec![provider],
		jwt::Mode::Strict,
		crate::http::auth::AuthorizationLocation::bearer_header(),
	)
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
		transaction_cookie_prefix: "agw_oidc_t_test".into(),
		same_site: SameSiteMode::Lax,
		secure: CookieSecureMode::Never,
		ttl: Duration::from_secs(3600),
		transaction_ttl: Duration::from_secs(300),
		encoder: test_oidc_cookie_encoder(),
	};

	OidcPolicy {
		policy_id: PolicyId::policy("policy"),
		provider: Arc::new(Provider {
			issuer: TEST_ISSUER.into(),
			authorization_endpoint: provider_endpoint("https://issuer.example.com/authorize"),
			token_endpoint: provider_endpoint("https://issuer.example.com/token"),
			id_token_validator: test_id_token_validator(),
		}),
		client: ClientConfig {
			client_id: TEST_CLIENT_ID.into(),
			credentials: ClientCredentials::ClientSecretBasic {
				client_secret: SecretString::new("client-secret".into()),
			},
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
	transaction_id: &str,
	csrf_state: &str,
	nonce: &str,
	original_uri: &str,
	expires_at_unix: u64,
) -> String {
	policy
		.session
		.encode_transaction(&TransactionState {
			policy_id: policy.policy_id.clone(),
			transaction_id: transaction_id.into(),
			csrf_state: csrf_state.into(),
			nonce: nonce.into(),
			pkce_verifier: SecretString::new("pkce-verifier".into()),
			original_uri: original_uri.into(),
			expires_at_unix,
		})
		.expect("encode transaction")
}

fn encoded_callback_state(transaction_id: &str, csrf_state: &str) -> String {
	super::callback::CallbackTransactionState {
		transaction_id: transaction_id.into(),
		csrf_state: csrf_state.into(),
	}
	.encode()
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

fn redirect_location(response: &crate::http::Response) -> String {
	response
		.headers()
		.get(header::LOCATION)
		.expect("location header")
		.to_str()
		.expect("location utf8")
		.to_string()
}

fn query_param(url: &str, key: &str) -> String {
	url::Url::parse(url)
		.expect("absolute url")
		.query_pairs()
		.find_map(|(k, v)| (k == key).then(|| v.into_owned()))
		.expect("query param")
}

fn parse_set_cookie(set_cookie: &str) -> cookie::Cookie<'static> {
	cookie::Cookie::parse(set_cookie.to_string())
		.expect("set-cookie")
		.into_owned()
}

fn explicit_local_oidc_config() -> LocalOidcConfig {
	LocalOidcConfig {
		issuer: TEST_ISSUER.into(),
		discovery: None,
		authorization_endpoint: Some(provider_endpoint("https://issuer.example.com/authorize")),
		token_endpoint: Some(provider_endpoint("https://issuer.example.com/token")),
		token_endpoint_auth: None,
		jwks: Some(test_jwks_inline()),
		client_id: TEST_CLIENT_ID.into(),
		client_secret: Some(SecretString::new("client-secret".into())),
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
fn normalize_original_uri_preserves_only_safe_local_targets() {
	let over_limit = format!("/{}", "a".repeat(2050));
	let cases = [
		("missing path", None, "/"),
		("local path", Some("/protected?x=1"), "/protected?x=1"),
		("scheme-relative path", Some("//evil.example/path"), "/"),
		(
			"percent-decoded scheme-relative path",
			Some("/%2Fevil.example/path"),
			"/",
		),
		("over limit", Some(over_limit.as_str()), "/"),
	];

	for (name, raw, expected) in cases {
		let path_and_query =
			raw.map(|raw| http::uri::PathAndQuery::try_from(raw).expect("path and query"));
		assert_eq!(
			super::session::normalize_original_uri(path_and_query.as_ref()),
			expected,
			"{name}"
		);
	}
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
			"http redirect uri keeps cookie non-secure",
			"http://127.0.0.1/private",
			"text/html",
			"http://127.0.0.1/oauth/callback",
			Some("redirect_uri=http%3A%2F%2F127.0.0.1%2Foauth%2Fcallback"),
			false,
		),
		(
			"https redirect uri marks cookie secure behind plain http",
			"http://127.0.0.1/private",
			"text/html",
			"https://app.example.com/oauth/callback",
			Some("redirect_uri=https%3A%2F%2Fapp.example.com%2Foauth%2Fcallback"),
			true,
		),
		(
			"json request",
			"https://app.example.com/private",
			"application/json",
			"https://app.example.com/oauth/callback",
			None,
			true,
		),
		(
			"callback path without query",
			"https://app.example.com/oauth/callback",
			"text/html",
			"https://app.example.com/oauth/callback",
			None,
			true,
		),
	];

	for (name, request_uri, accept, redirect_uri, expected_fragment, expect_secure_cookie) in cases {
		let mut policy = test_policy();
		policy.session.secure = CookieSecureMode::Auto;
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
		let cookie = response
			.headers()
			.get(header::SET_COOKIE)
			.expect("set-cookie header")
			.to_str()
			.expect("set-cookie utf8");
		assert_eq!(cookie.contains("Secure"), expect_secure_cookie, "{name}");
	}
}

#[tokio::test]
async fn apply_bypasses_cors_preflight_requests() {
	let policy = test_policy();
	let mut req = request(Method::OPTIONS, "https://app.example.com/private", None);
	req.headers_mut().insert(
		header::ORIGIN,
		"https://frontend.example.com".parse().unwrap(),
	);
	req.headers_mut().insert(
		header::ACCESS_CONTROL_REQUEST_METHOD,
		"GET".parse().unwrap(),
	);

	let response = policy
		.apply(None, &mut req, policy_client())
		.await
		.expect("preflight should bypass oidc");

	assert!(response.direct_response.is_none());
	assert!(response.response_headers.is_none());
	assert!(req.extensions().get::<jwt::Claims>().is_none());
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
			credentials: ClientCredentials::from_parts(
				token_endpoint_auth,
				Some(SecretString::new(client_secret.into())),
			)
			.expect("confidential credentials"),
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
async fn public_client_token_exchange_sends_no_client_secret() {
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
		client_id: TEST_CLIENT_ID.into(),
		credentials: ClientCredentials::None,
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
	.expect("public-client token exchange");
	assert!(response.id_token.is_some());

	let request = &mock.received_requests().await.expect("requests")[0];
	let body = String::from_utf8(request.body.clone()).expect("utf8 body");

	// Public clients authenticate themselves only by presenting client_id and
	// a valid PKCE verifier; no Authorization header and no client_secret
	// parameter should ever leave the gateway.
	assert!(!request.headers.contains_key("authorization"));
	assert!(
		body.contains(&format!("client_id={TEST_CLIENT_ID}")),
		"token request must still identify the client: {body}"
	);
	assert!(
		!body.contains("client_secret"),
		"public-client request leaked a client_secret: {body}"
	);
	assert!(
		body.contains("code_verifier=verifier"),
		"PKCE verifier must still be present: {body}"
	);
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
			credentials: ClientCredentials::ClientSecretBasic {
				client_secret: SecretString::new("client-secret".into()),
			},
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
			"tx-1",
			"test-state",
			Error::MissingTransaction,
		),
		(
			"csrf mismatch",
			Some(("expected-state", TEST_NONCE, "/protected")),
			"tx-1",
			"wrong-state",
			Error::CsrfMismatch,
		),
	];

	for (name, transaction, transaction_id, callback_csrf_state, expected_error) in cases {
		let policy = test_policy();
		let callback_state = encoded_callback_state(transaction_id, callback_csrf_state);
		let uri =
			format!("https://app.example.com/oauth/callback?code=auth-code&state={callback_state}");
		let mut req = request(Method::GET, &uri, Some("text/html"));
		if let Some((cookie_csrf_state, nonce, original_uri)) = transaction {
			let encoded = encoded_transaction(
				&policy,
				transaction_id,
				cookie_csrf_state,
				nonce,
				original_uri,
				now_unix() + 300,
			);
			add_cookie(
				&mut req,
				format!(
					"{}={encoded}",
					policy.session.transaction_cookie_name(transaction_id)
				),
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
	let mut policy = policy;
	policy.session.secure = CookieSecureMode::Auto;
	let transaction_id = "tx-1";
	let callback_state = encoded_callback_state(transaction_id, "test-state");
	let encoded = encoded_transaction(
		&policy,
		transaction_id,
		"test-state",
		TEST_NONCE,
		"/protected",
		now_unix() + 300,
	);
	let uri = format!("http://127.0.0.1/oauth/callback?code=auth-code&state={callback_state}");
	let mut req = request(Method::GET, &uri, Some("text/html"));
	add_cookie(
		&mut req,
		format!(
			"{}={encoded}",
			policy.session.transaction_cookie_name(transaction_id)
		),
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
	assert!(cookies.iter().all(|cookie| cookie.contains("Secure")));
	assert!(cookies.iter().any(|cookie| {
		cookie.starts_with(&policy.session.transaction_cookie_name(transaction_id))
			&& cookie.contains("Max-Age=0")
	}));
}

#[tokio::test]
async fn concurrent_login_attempts_use_distinct_transaction_cookies() {
	let mock = MockServer::start().await;
	let policy = test_callback_policy(provider_endpoint(format!("{}/token", mock.uri())));

	let mut first_req = request(
		Method::GET,
		"https://app.example.com/protected",
		Some("text/html"),
	);
	let first_response = policy
		.apply(None, &mut first_req, policy_client())
		.await
		.expect("first login start")
		.direct_response
		.expect("first redirect");
	let first_location = redirect_location(&first_response);
	let first_state = query_param(&first_location, "state");
	let first_cookie = parse_set_cookie(
		first_response
			.headers()
			.get(header::SET_COOKIE)
			.expect("first set-cookie")
			.to_str()
			.expect("first set-cookie utf8"),
	);
	let first_transaction = policy
		.session
		.decode_transaction(first_cookie.value())
		.expect("decode first transaction");

	let mut second_req = request(
		Method::GET,
		"https://app.example.com/protected",
		Some("text/html"),
	);
	let second_response = policy
		.apply(None, &mut second_req, policy_client())
		.await
		.expect("second login start")
		.direct_response
		.expect("second redirect");
	let second_cookie = parse_set_cookie(
		second_response
			.headers()
			.get(header::SET_COOKIE)
			.expect("second set-cookie")
			.to_str()
			.expect("second set-cookie utf8"),
	);

	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": signed_id_token(&first_transaction.nonce)
		})))
		.mount(&mock)
		.await;

	assert_ne!(first_cookie.name(), second_cookie.name());
	assert!(
		first_cookie
			.name()
			.starts_with(&policy.session.transaction_cookie_prefix)
	);
	assert!(
		second_cookie
			.name()
			.starts_with(&policy.session.transaction_cookie_prefix)
	);

	let mut callback_req = request(
		Method::GET,
		&format!("https://app.example.com/oauth/callback?code=auth-code&state={first_state}"),
		Some("text/html"),
	);
	add_cookie(
		&mut callback_req,
		format!("{}={}", first_cookie.name(), first_cookie.value()),
	);
	add_cookie(
		&mut callback_req,
		format!("{}={}", second_cookie.name(), second_cookie.value()),
	);

	let callback_response = policy
		.apply(None, &mut callback_req, policy_client())
		.await
		.expect("first callback succeeds")
		.direct_response
		.expect("callback redirect");
	let set_cookies: Vec<_> = callback_response
		.headers()
		.get_all(header::SET_COOKIE)
		.iter()
		.map(|value| value.to_str().expect("set-cookie utf8").to_string())
		.collect();
	assert!(
		set_cookies
			.iter()
			.any(|cookie| cookie.starts_with(first_cookie.name()) && cookie.contains("Max-Age=0"))
	);
	assert!(
		!set_cookies
			.iter()
			.any(|cookie| cookie.starts_with(second_cookie.name()) && cookie.contains("Max-Age=0"))
	);
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
	let transaction_id = "tx-1";
	let callback_state = encoded_callback_state(transaction_id, "test-state");
	let encoded = encoded_transaction(
		&policy,
		transaction_id,
		"test-state",
		TEST_NONCE,
		"/protected",
		now_unix() + 300,
	);
	let uri =
		format!("https://edge.example.net:8443/oauth/callback?code=auth-code&state={callback_state}");
	let mut req = request(Method::GET, &uri, Some("text/html"));
	add_cookie(
		&mut req,
		format!(
			"{}={encoded}",
			policy.session.transaction_cookie_name(transaction_id)
		),
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
				token_endpoint_auth: None,
				jwks: None,
				client_id: TEST_CLIENT_ID.into(),
				client_secret: Some(SecretString::new("client-secret".into())),
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
		(
			"explicit post",
			LocalOidcConfig {
				token_endpoint_auth: Some(TokenEndpointAuth::ClientSecretPost),
				..explicit_local_oidc_config()
			},
			provider_endpoint("https://issuer.example.com/authorize"),
			provider_endpoint("https://issuer.example.com/token"),
			TokenEndpointAuth::ClientSecretPost,
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
			policy.client.credentials.method(),
			expected_token_endpoint_auth,
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
		token_endpoint_auth: None,
		jwks: None,
		client_id: TEST_CLIENT_ID.into(),
		client_secret: Some(SecretString::new("client-secret".into())),
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
				token_endpoint_auth: None,
				jwks: Some(test_jwks_inline()),
				client_id: TEST_CLIENT_ID.into(),
				client_secret: Some(SecretString::new("client-secret".into())),
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
		(
			"token endpoint auth without explicit provider",
			LocalOidcConfig {
				issuer: TEST_ISSUER.into(),
				discovery: None,
				authorization_endpoint: None,
				token_endpoint: None,
				token_endpoint_auth: Some(TokenEndpointAuth::ClientSecretPost),
				jwks: None,
				client_id: TEST_CLIENT_ID.into(),
				client_secret: Some(SecretString::new("client-secret".into())),
				redirect_uri: "http://localhost:3000/oauth/callback".into(),
				scopes: vec![],
			},
			"tokenEndpointAuth must be omitted unless authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly",
		),
	];

	for (name, config, expected_error_fragment) in cases {
		let err = compile_local_policy(config, translated_policy_id(name))
			.await
			.expect_err(name);
		assert!(err.to_string().contains(expected_error_fragment), "{name}");
	}
}

#[test]
fn policy_id_from_str_round_trips_canonical_forms() {
	use std::str::FromStr;

	for input in [
		"route/default/my-route/rule-0",
		"policy/default/my-policy",
		"route/ns/r",
		"policy/ns/p",
	] {
		let parsed = PolicyId::from_str(input).expect(input);
		assert_eq!(parsed.as_str(), input);
	}
}

#[test]
fn policy_id_from_str_rejects_malformed_input() {
	use std::str::FromStr;

	for bad in [
		"",              // missing prefix
		"nope",          // missing '/'
		"route/",        // empty identifier
		"policy/",       // empty identifier
		"gateway/ns/g",  // unsupported prefix
		"/default/name", // empty prefix
	] {
		assert!(
			PolicyId::from_str(bad).is_err(),
			"expected PolicyId::from_str({bad:?}) to fail",
		);
	}
}

#[test]
fn policy_id_constructors_match_from_str() {
	use std::str::FromStr;

	let route = PolicyId::route("default/my-route/rule-0");
	assert_eq!(
		PolicyId::from_str(route.as_str()).unwrap(),
		route,
		"route constructor output must round-trip",
	);

	let policy = PolicyId::policy("default/my-policy");
	assert_eq!(
		PolicyId::from_str(policy.as_str()).unwrap(),
		policy,
		"policy constructor output must round-trip",
	);
}

fn compile_test_oidc_from_xds(
	policy_id: PolicyId,
	jwks_inline: &str,
	encoder: &crate::http::sessionpersistence::Encoder,
) -> Result<OidcPolicy, super::Error> {
	super::compile_oidc_policy_from_xds(
		policy_id,
		TEST_ISSUER.into(),
		provider_endpoint("https://issuer.example.com/authorize"),
		provider_endpoint("https://issuer.example.com/token"),
		TokenEndpointAuth::ClientSecretBasic,
		jwks_inline,
		TEST_CLIENT_ID.into(),
		Some(SecretString::new("client-secret".into())),
		test_redirect_uri(),
		vec!["profile".into()],
		encoder,
	)
}

#[test]
fn xds_oidc_compiles_to_policy() {
	let policy_id = PolicyId::policy("default/my-policy");
	let encoder = test_oidc_cookie_encoder();
	let jwks = serde_json::to_string(&test_jwks()).expect("jwks json");

	let policy =
		compile_test_oidc_from_xds(policy_id.clone(), &jwks, &encoder).expect("compile xds oidc");

	assert_eq!(policy.policy_id, policy_id);
	assert_eq!(policy.provider.issuer, TEST_ISSUER);
	assert_eq!(policy.client.client_id, TEST_CLIENT_ID);
	match &policy.client.credentials {
		ClientCredentials::ClientSecretBasic { client_secret } => {
			assert_eq!(client_secret.expose_secret(), "client-secret");
		},
		other => panic!("expected ClientSecretBasic credentials, got {other:?}"),
	}

	// Scopes always include "openid", even when not explicitly requested.
	assert_eq!(policy.scopes, vec!["openid".to_string(), "profile".into()]);

	// Cookie names are deterministic from the policy id; the xDS path must
	// derive the same cookie as the file-backed path.
	let (expected_cookie, expected_transaction_prefix) =
		super::session::derive_cookie_names(&policy_id);
	assert_eq!(policy.session.cookie_name, expected_cookie);
	assert_eq!(
		policy.session.transaction_cookie_prefix,
		expected_transaction_prefix
	);
}

#[test]
fn xds_oidc_rejects_invalid_jwks_inline() {
	let err = compile_test_oidc_from_xds(
		PolicyId::policy("default/p"),
		"not valid json",
		&test_oidc_cookie_encoder(),
	)
	.expect_err("bad jwks must be rejected");
	let msg = err.to_string();
	assert!(
		msg.contains("failed to parse inline oidc jwks"),
		"expected jwks parse error, got: {msg}",
	);
}
