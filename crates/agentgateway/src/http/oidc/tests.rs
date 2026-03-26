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
use crate::http::oidc::callback;
use crate::proxy::ProxyError;
use crate::serdes::FileInlineOrRemote;
use crate::test_helpers::proxymock::{send_request_headers, setup_proxy_test};
use crate::types::agent::ServerTLSConfig;
use crate::types::agent::{
	Bind, BindProtocol, Listener, ListenerName, ListenerOidc, ListenerProtocol, ListenerSet,
	PathMatch, Route, RouteBackendReference, RouteMatch, RouteName, RouteSet, TunnelProtocol,
};

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

fn test_oidc_cookie_secret() -> SecretString {
	SecretString::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into())
}

fn test_policy() -> OidcPolicy {
	let session = SessionConfig {
		cookie_name: "agw_oidc_s_test".into(),
		transaction_cookie_name: "agw_oidc_t_test".into(),
		same_site: SameSiteMode::Lax,
		secure: CookieSecureMode::Never,
		ttl: Duration::from_secs(3600),
		transaction_ttl: Duration::from_secs(300),
		encoder: crate::http::sessionpersistence::Encoder::aes(
			"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		)
		.expect("aes encoder"),
	};

	OidcPolicy {
		policy_id: "policy".into(),
		provider: Arc::new(Provider {
			issuer: TEST_ISSUER.into(),
			authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
			token_endpoint: Uri::from_static("https://issuer.example.com/token"),
			token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
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

fn test_callback_policy(token_endpoint: Uri) -> OidcPolicy {
	let mut policy = test_policy();
	policy.provider = Arc::new(Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint,
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
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

fn callback_context(state: &str, transaction_cookie: String) -> callback::CallbackRequestContext {
	callback::CallbackRequestContext {
		is_https: true,
		code: "auth-code".into(),
		state: state.into(),
		transaction_cookie,
	}
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

fn test_provider(name: &str, policy: OidcPolicy) -> NamedOidcProvider {
	NamedOidcProvider {
		name: name.into(),
		policy,
	}
}

fn oidc_route(key: &str, hostnames: Vec<&str>, provider: &str) -> Route {
	Route {
		key: key.into(),
		name: RouteName {
			name: key.into(),
			namespace: "default".into(),
			rule_name: None,
			kind: None,
		},
		service_key: None,
		hostnames: hostnames.into_iter().map(Into::into).collect(),
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/".into()),
			method: None,
			query: vec![],
		}],
		backends: vec![RouteBackendReference {
			weight: 1,
			backend: crate::types::agent::BackendReference::Invalid,
			inline_policies: vec![],
		}],
		inline_policies: vec![crate::types::agent::TrafficPolicy::Oidc(OidcProviderRef {
			provider: provider.into(),
		})],
	}
}

fn oidc_bind(
	listener_name: ListenerName,
	port: u16,
	protocol: ListenerProtocol,
	hostname: &str,
	providers: Vec<NamedOidcProvider>,
	routes: Vec<Route>,
) -> Bind {
	Bind {
		key: "bind".into(),
		address: format!("127.0.0.1:{port}").parse().unwrap(),
		protocol: match protocol {
			ListenerProtocol::HTTP => BindProtocol::http,
			ListenerProtocol::HTTPS(_) => BindProtocol::tls,
			_ => BindProtocol::http,
		},
		tunnel_protocol: TunnelProtocol::Direct,
		listeners: ListenerSet::from_list([Listener {
			key: "listener".into(),
			name: listener_name,
			hostname: hostname.into(),
			protocol,
			oidc: Some(ListenerOidc::new(providers).unwrap()),
			routes: RouteSet::from_list(routes),
			tcp_routes: Default::default(),
		}]),
	}
}

fn oidc_listener(
	port: u16,
	protocol: ListenerProtocol,
	hostname: &str,
	providers: Vec<NamedOidcProvider>,
) -> Arc<Listener> {
	oidc_bind(
		ListenerName {
			gateway_name: "gw".into(),
			gateway_namespace: "ns".into(),
			listener_name: "listener".into(),
			listener_set: None,
		},
		port,
		protocol,
		hostname,
		providers,
		vec![oidc_route("route", vec![hostname], "corp")],
	)
	.listeners
	.get_exactly_one()
	.expect("single listener")
}

// Config parsing and local session invariants.
#[test]
fn redirect_uri_rejects_ambiguous_values() {
	assert!(RedirectUri::parse("https://app.example.com/".into()).is_err());
	assert!(RedirectUri::parse("https://user@app.example.com/oauth/callback".into()).is_err());
	assert!(RedirectUri::parse("https://app.example.com/oauth/callback?x=1".into()).is_err());
	assert!(RedirectUri::parse("https://app.example.com/oauth/../callback".into()).is_err());
	assert!(RedirectUri::parse("https://app.example.com/oauth/%2fcallback".into()).is_err());
	assert!(RedirectUri::parse("http://app.example.com/oauth/callback".into()).is_err());
}

// Route enforcement and token exchange behavior.
#[tokio::test]
async fn apply_derives_claims_from_stored_id_token() {
	let policy = test_policy();
	let id_token = signed_id_token(TEST_NONCE);
	let encoded = policy
		.session
		.encode_browser_session(&BrowserSession {
			policy_id: policy.policy_id.clone(),
			raw_id_token: SecretString::new(id_token.clone().into()),
			expires_at_unix: Some(now_unix() + 60),
		})
		.expect("encode session");
	let mut req = request(
		Method::GET,
		"https://app.example.com/protected",
		Some("text/html"),
	);
	req.headers_mut().insert(
		header::COOKIE,
		format!("{}={encoded}", policy.session.cookie_name)
			.parse()
			.expect("cookie header"),
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = policy
		.apply(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
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
	let policy = test_policy();
	let mut req = request(
		Method::GET,
		"https://app.example.com/protected?foo=bar",
		Some("text/html,application/xhtml+xml"),
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = policy
		.apply(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("browser policy apply");
	let direct = response.direct_response.expect("redirect response");
	assert_eq!(direct.status(), http::StatusCode::FOUND);
	let location = direct
		.headers()
		.get(header::LOCATION)
		.unwrap()
		.to_str()
		.unwrap();
	assert!(location.starts_with("https://issuer.example.com/authorize?"));
	assert!(location.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Foauth%2Fcallback"));
	assert!(location.contains("scope=openid+profile"));
}

#[tokio::test]
async fn auto_mode_returns_unauthorized_for_json_requests() {
	let policy = test_policy();
	let mut req = request(
		Method::GET,
		"https://app.example.com/api/data",
		Some("application/json"),
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = policy
		.apply(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("browser policy apply");
	let direct = response.direct_response.expect("unauthorized response");
	assert_eq!(direct.status(), http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn proxy_resolves_route_provider_against_listener_browser_provider() {
	let mut policy = test_policy();
	policy.redirect_uri =
		RedirectUri::parse("http://127.0.0.1/oauth/callback".into()).expect("redirect uri");

	let listener_name = ListenerName {
		gateway_name: "gw".into(),
		gateway_namespace: "ns".into(),
		listener_name: "listener".into(),
		listener_set: None,
	};
	let bind = oidc_bind(
		listener_name,
		80,
		ListenerProtocol::HTTP,
		"127.0.0.1",
		vec![test_provider("corp", policy)],
		vec![oidc_route("route", vec!["127.0.0.1"], "corp")],
	);

	let proxy = setup_proxy_test("{}")
		.expect("proxy test harness")
		.with_bind(bind);
	let io = proxy.serve_http("bind".into());
	let response = send_request_headers(
		io,
		Method::GET,
		"http://127.0.0.1/protected",
		&[(header::ACCEPT.as_str(), "text/html")],
	)
	.await;

	assert_eq!(response.status(), http::StatusCode::FOUND);
	let location = response
		.headers()
		.get(header::LOCATION)
		.expect("location header")
		.to_str()
		.expect("location value");
	assert!(location.starts_with("https://issuer.example.com/authorize?"));
	assert!(location.contains("redirect_uri=http%3A%2F%2F127.0.0.1%2Foauth%2Fcallback"));
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

	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: "client:id".into(),
		client_secret: SecretString::new("s e:c".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let response = provider::exchange_code(
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
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

	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretPost],
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: "client-id".into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretPost,
	};

	let response = provider::exchange_code(
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
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
		.respond_with(
			ResponseTemplate::new(200)
				.set_delay(Duration::from_millis(100))
				.set_body_json(json!({ "id_token": signed_id_token(TEST_NONCE) })),
		)
		.mount(&mock)
		.await;

	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let err = provider::exchange_code_with_timeout(
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
		Duration::from_millis(25),
	)
	.await
	.expect_err("token exchange timeout should fail");
	assert!(matches!(err, Error::TokenExchangeFailed(_)));
}

#[tokio::test]
async fn token_exchange_rejects_oversized_response_body() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(
			ResponseTemplate::new(200)
				.insert_header(header::CONTENT_TYPE.as_str(), "application/json")
				.set_body_string(format!("\"{}\"", "x".repeat(70_000))),
		)
		.mount(&mock)
		.await;

	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let provider = Provider {
		issuer: TEST_ISSUER.into(),
		authorization_endpoint: Uri::from_static("https://issuer.example.com/authorize"),
		token_endpoint: format!("{}/token", mock.uri()).parse().unwrap(),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
		id_token_validator: test_id_token_validator(),
	};
	let client_config = ClientConfig {
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
	};

	let err = provider::exchange_code(
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
		&provider,
		&client_config,
		"https://app.example.com/oauth/callback",
		"code",
		&SecretString::new("verifier".into()),
	)
	.await
	.expect_err("oversized token response should fail");
	assert!(matches!(err, Error::TokenExchangeFailed(_)));
}

// Callback handling and post-login redirect behavior.
#[tokio::test]
async fn callback_requires_transaction_cookie() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy.clone())],
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=abc&state=csrf",
		None,
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let err = listener
		.maybe_handle_oidc_callback(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect_err("missing transaction cookie should fail");
	assert!(matches!(err, Error::MissingTransaction));
}

#[tokio::test]
async fn callback_rejects_invalid_transaction_cookie() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy.clone())],
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=abc&state=csrf",
		None,
	);
	req.headers_mut().insert(
		header::COOKIE,
		http::HeaderValue::from_str(&format!(
			"{}=not-a-valid-cookie",
			policy.session.transaction_cookie_name
		))
		.expect("cookie header"),
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let err = listener
		.maybe_handle_oidc_callback(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect_err("invalid transaction cookie should fail");
	assert!(matches!(err, Error::InvalidTransaction));
}

#[tokio::test]
async fn callback_rejects_state_mismatch() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let err = callback::handle_callback(
		&policy,
		None,
		callback_context(
			"wrong-state",
			encoded_transaction(
				&policy,
				"expected-state",
				TEST_NONCE,
				"/app",
				now_unix() + 60,
			),
		),
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
	)
	.await
	.expect_err("state mismatch should fail");
	assert!(matches!(err, Error::CsrfMismatch));
}

#[tokio::test]
async fn callback_rejects_policy_mismatch() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let mut wrong_policy = policy.clone();
	wrong_policy.policy_id = PolicyId::from("other-policy");
	let err = callback::handle_callback(
		&wrong_policy,
		None,
		callback_context(
			"csrf",
			encoded_transaction(&policy, "csrf", TEST_NONCE, "/app", now_unix() + 60),
		),
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
	)
	.await
	.expect_err("policy mismatch should fail");
	assert!(matches!(err, Error::PolicyMismatch));
}

#[tokio::test]
async fn callback_rejects_nonce_mismatch() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": signed_id_token("wrong-nonce"),
			"token_type": "Bearer"
		})))
		.mount(&mock)
		.await;
	let policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let err = callback::handle_callback(
		&policy,
		None,
		callback_context(
			"csrf",
			encoded_transaction(&policy, "csrf", TEST_NONCE, "/app", now_unix() + 60),
		),
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
	)
	.await
	.expect_err("nonce mismatch should fail");
	assert!(matches!(err, Error::NonceMismatch));
}

#[tokio::test]
async fn callback_success_sets_session_cookie_and_clears_transaction_cookie() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": signed_id_token(TEST_NONCE),
			"access_token": "access-token",
			"refresh_token": "refresh-token",
			"token_type": "Bearer"
		})))
		.mount(&mock)
		.await;
	let policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = callback::handle_callback(
		&policy,
		None,
		callback_context(
			"csrf",
			encoded_transaction(&policy, "csrf", TEST_NONCE, "/app?x=1", now_unix() + 60),
		),
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
	)
	.await
	.expect("callback success");
	let direct = response.direct_response.expect("direct response");
	assert_eq!(direct.status(), http::StatusCode::FOUND);
	assert_eq!(direct.headers().get(header::LOCATION).unwrap(), "/app?x=1");
	let set_cookies: Vec<_> = direct
		.headers()
		.get_all(header::SET_COOKIE)
		.iter()
		.collect();
	assert_eq!(set_cookies.len(), 2);
	assert!(set_cookies.iter().any(|value| {
		value
			.to_str()
			.unwrap()
			.starts_with(&format!("{}=", policy.session.cookie_name))
	}));
	assert!(set_cookies.iter().any(|value| {
		let value = value.to_str().unwrap();
		value.starts_with(&format!("{}=", policy.session.transaction_cookie_name))
			&& value.contains("Max-Age=0")
	}));
}

#[tokio::test]
async fn callback_success_falls_back_to_root_for_invalid_original_uri() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": signed_id_token(TEST_NONCE),
			"token_type": "Bearer"
		})))
		.mount(&mock)
		.await;
	let policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = callback::handle_callback(
		&policy,
		None,
		callback_context(
			"csrf",
			encoded_transaction(
				&policy,
				"csrf",
				TEST_NONCE,
				"https://evil.example.com/path",
				now_unix() + 60,
			),
		),
		crate::proxy::httpproxy::PolicyClient {
			inputs: proxy.inputs(),
		},
	)
	.await
	.expect("callback success");
	let direct = response.direct_response.expect("direct response");
	assert_eq!(direct.headers().get(header::LOCATION).unwrap(), "/");
}

#[tokio::test]
async fn listener_callback_handling_ignores_non_callback_traffic_on_same_path() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy)],
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?foo=bar",
		None,
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = listener
		.maybe_handle_oidc_callback(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("non-callback response");
	assert!(response.direct_response.is_none());
}

#[tokio::test]
async fn listener_callback_handling_surfaces_provider_error_callbacks() {
	let policy = test_callback_policy(Uri::from_static("https://issuer.example.com/token"));
	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy)],
	);
	let mut req = request(
		Method::GET,
		"https://app.example.com/oauth/callback?error=access_denied&state=csrf",
		None,
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let err = listener
		.maybe_handle_oidc_callback(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect_err("provider callback error should fail");
	assert!(matches!(err, Error::ProviderCallback(ref value) if value == "access_denied"));
}

#[test]
fn oidc_proxy_errors_use_error_specific_status_codes() {
	let response = ProxyError::OidcFailure(Error::InvalidCallback).into_response();
	assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

	let response = ProxyError::OidcFailure(Error::TokenExchangeFailed(anyhow::anyhow!(
		"upstream timeout"
	)))
	.into_response();
	assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn listener_callback_handling_matches_exact_host_and_port() {
	let mock = MockServer::start().await;
	Mock::given(method("POST"))
		.and(path("/token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id_token": signed_id_token(TEST_NONCE),
			"token_type": "Bearer"
		})))
		.mount(&mock)
		.await;

	let mut policy = test_callback_policy(format!("{}/token", mock.uri()).parse().unwrap());
	policy.redirect_uri =
		RedirectUri::parse("https://app.example.com:8443/oauth/callback".into()).expect("redirect uri");
	policy.session.cookie_name = "agw_oidc_s_port".into();
	policy.session.transaction_cookie_name = "agw_oidc_t_port".into();
	let listener = oidc_listener(
		8443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy.clone())],
	);
	let proxy = setup_proxy_test("{}").expect("proxy test harness");

	let mut wrong_port = request(
		Method::GET,
		"https://app.example.com/oauth/callback?code=abc&state=csrf",
		None,
	);
	let response = listener
		.maybe_handle_oidc_callback(
			None,
			&mut wrong_port,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("wrong port should not intercept");
	assert!(response.direct_response.is_none());

	let mut req = request(
		Method::GET,
		"https://app.example.com:8443/oauth/callback?code=abc&state=csrf",
		None,
	);
	let transaction = encoded_transaction(&policy, "csrf", TEST_NONCE, "/done", now_unix() + 60);
	req.headers_mut().insert(
		header::COOKIE,
		format!("{}={transaction}", policy.session.transaction_cookie_name)
			.parse()
			.expect("cookie header"),
	);
	let response = listener
		.maybe_handle_oidc_callback(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("callback response");
	let direct = response.direct_response.expect("direct response");
	assert_eq!(direct.headers().get(header::LOCATION).unwrap(), "/done");
}

// Listener-owned callback validation.
#[tokio::test]
async fn compiled_policy_uses_fixed_defaults_and_stable_cookie_derivation() {
	let make_config = |redirect_uri: &str| LocalOidcConfig {
		issuer: "https://issuer.example.com".into(),
		discovery: None,
		authorization_endpoint: Some(Uri::from_static("https://issuer.example.com/authorize")),
		token_endpoint: Some(Uri::from_static("https://issuer.example.com/token")),
		jwks: Some(FileInlineOrRemote::Inline(
			serde_json::to_string(&test_jwks()).unwrap(),
		)),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretBasic],
		client_id: "client-id".into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: redirect_uri.into(),
		scopes: vec!["profile".into()],
	};
	let oidc_cookie_secret = test_oidc_cookie_secret();
	let policy_a = make_config("https://APP.example.com/oauth/callback")
		.translate(test_client(), &oidc_cookie_secret)
		.await
		.expect("policy a");
	let policy_b = make_config("https://app.example.com:443/oauth/callback")
		.translate(test_client(), &oidc_cookie_secret)
		.await
		.expect("policy b");
	assert_eq!(policy_a.policy_id, policy_b.policy_id);
	assert_eq!(policy_a.session.cookie_name, policy_b.session.cookie_name);
	assert_eq!(policy_a.session.same_site, SameSiteMode::Lax);
	assert_eq!(policy_a.session.secure, CookieSecureMode::Auto);
	assert_eq!(policy_a.session.ttl, Duration::from_secs(3600));
	assert_eq!(policy_a.session.transaction_ttl, Duration::from_secs(300));
	assert_eq!(
		policy_a.scopes,
		vec!["openid".to_string(), "profile".to_string()]
	);
	assert_eq!(
		policy_a.client.token_endpoint_auth,
		TokenEndpointAuth::ClientSecretBasic
	);
}

#[tokio::test]
async fn issuer_only_oidc_config_uses_discovery() {
	let mock = MockServer::start().await;
	let issuer = format!("{}/issuer", mock.uri());
	let authorization_endpoint = format!("{issuer}/authorize");
	let token_endpoint = format!("{issuer}/token");
	let jwks_uri = format!("{issuer}/jwks.json");

	Mock::given(method("GET"))
		.and(path("/issuer/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": issuer.clone(),
			"authorization_endpoint": authorization_endpoint.clone(),
			"token_endpoint": token_endpoint.clone(),
			"jwks_uri": jwks_uri.clone(),
			"token_endpoint_auth_methods_supported": ["client_secret_post"]
		})))
		.mount(&mock)
		.await;
	Mock::given(method("GET"))
		.and(path("/issuer/jwks.json"))
		.respond_with(ResponseTemplate::new(200).set_body_json(test_jwks()))
		.mount(&mock)
		.await;

	let policy = LocalOidcConfig {
		issuer: format!("{}/issuer", mock.uri()),
		discovery: None,
		authorization_endpoint: None,
		token_endpoint: None,
		jwks: None,
		token_endpoint_auth_methods_supported: vec![],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "https://app.example.com/oauth/callback".into(),
		scopes: vec!["profile".into()],
	}
	.translate(test_client(), &test_oidc_cookie_secret())
	.await
	.expect("discovery-backed policy");

	assert_eq!(
		policy.provider.authorization_endpoint,
		authorization_endpoint.parse::<Uri>().unwrap()
	);
	assert_eq!(
		policy.provider.token_endpoint,
		token_endpoint.parse::<Uri>().unwrap()
	);
	assert_eq!(
		policy.provider.token_endpoint_auth_methods_supported,
		vec![TokenEndpointAuth::ClientSecretPost]
	);
	assert_eq!(
		policy.client.token_endpoint_auth,
		TokenEndpointAuth::ClientSecretPost
	);
}

#[tokio::test]
async fn explicit_oidc_fields_override_discovery_without_loading_discovered_jwks() {
	let mock = MockServer::start().await;
	let issuer = format!("{}/issuer", mock.uri());
	let discovered_token_endpoint = format!("{issuer}/token");

	Mock::given(method("GET"))
		.and(path("/issuer/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": issuer.clone(),
			"authorization_endpoint": format!("{issuer}/authorize"),
			"token_endpoint": discovered_token_endpoint.clone(),
			"jwks_uri": format!("{issuer}/discovered-jwks.json"),
			"token_endpoint_auth_methods_supported": ["client_secret_basic"]
		})))
		.mount(&mock)
		.await;

	let policy = LocalOidcConfig {
		issuer: format!("{}/issuer", mock.uri()),
		discovery: None,
		authorization_endpoint: Some(Uri::from_static("https://override.example.com/authorize")),
		token_endpoint: None,
		jwks: Some(FileInlineOrRemote::Inline(
			serde_json::to_string(&test_jwks()).unwrap(),
		)),
		token_endpoint_auth_methods_supported: vec![TokenEndpointAuth::ClientSecretPost],
		client_id: TEST_CLIENT_ID.into(),
		client_secret: SecretString::new("client-secret".into()),
		redirect_uri: "https://app.example.com/oauth/callback".into(),
		scopes: vec!["profile".into()],
	}
	.translate(test_client(), &test_oidc_cookie_secret())
	.await
	.expect("mixed discovery/override policy");

	assert_eq!(
		policy.provider.authorization_endpoint,
		Uri::from_static("https://override.example.com/authorize")
	);
	assert_eq!(
		policy.provider.token_endpoint,
		discovered_token_endpoint.parse::<Uri>().unwrap()
	);
	assert_eq!(
		policy.provider.token_endpoint_auth_methods_supported,
		vec![TokenEndpointAuth::ClientSecretPost]
	);
	assert_eq!(
		policy.client.token_endpoint_auth,
		TokenEndpointAuth::ClientSecretPost
	);
}

#[test]
fn distinct_callback_owners_can_share_a_listener() {
	let policy = test_policy();
	let mut other_policy = test_policy();
	other_policy.policy_id = "other-policy".into();
	other_policy.redirect_uri =
		RedirectUri::parse("https://app.example.com/oauth/callback-b".into()).expect("redirect uri");
	other_policy.session.cookie_name = "agw_oidc_s_other".into();
	other_policy.session.transaction_cookie_name = "agw_oidc_t_other".into();

	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![
			test_provider("corp", policy.clone()),
			test_provider("other", other_policy.clone()),
		],
	);
	listener
		.validate_oidc(443)
		.expect("distinct callback owners should validate");
}

#[test]
fn duplicate_callback_ownership_is_rejected() {
	let policy = test_policy();
	let mut other_policy = test_policy();
	other_policy.policy_id = "other-policy".into();
	other_policy.session.cookie_name = "agw_oidc_s_other".into();
	other_policy.session.transaction_cookie_name = "agw_oidc_t_other".into();

	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![
			test_provider("corp", policy),
			test_provider("other", other_policy),
		],
	);
	let err = listener
		.validate_oidc(443)
		.expect_err("duplicate callback ownership should fail");
	assert!(
		err
			.to_string()
			.contains("duplicate oidc callback ownership")
	);
}

#[test]
fn redirect_host_must_be_covered_by_listener_hostname() {
	let mut policy = test_policy();
	policy.redirect_uri =
		RedirectUri::parse("https://api.example.com/oauth/callback".into()).expect("redirect uri");
	policy.policy_id = "api-policy".into();
	policy.session.cookie_name = "agw_oidc_s_api".into();
	policy.session.transaction_cookie_name = "agw_oidc_t_api".into();

	let listener = oidc_listener(
		443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", policy)],
	);
	let err = listener
		.validate_oidc(443)
		.expect_err("redirect host coverage should fail");
	assert!(err.to_string().contains("not covered by listener hostname"));
}

#[test]
fn redirect_port_must_match_listener_port() {
	let listener = oidc_listener(
		8443,
		ListenerProtocol::HTTPS(ServerTLSConfig::new_invalid()),
		"app.example.com",
		vec![test_provider("corp", test_policy())],
	);
	let err = listener
		.validate_oidc(8443)
		.expect_err("redirect port mismatch should fail");
	assert!(err.to_string().contains("must use listener port"));
}

#[tokio::test]
async fn apply_finds_session_cookie_across_multiple_cookie_headers() {
	let policy = test_policy();
	let id_token = signed_id_token(TEST_NONCE);
	let encoded = policy
		.session
		.encode_browser_session(&BrowserSession {
			policy_id: policy.policy_id.clone(),
			raw_id_token: SecretString::new(id_token.clone().into()),
			expires_at_unix: Some(now_unix() + 60),
		})
		.expect("encode session");

	let mut req = request(
		Method::GET,
		"https://app.example.com/protected",
		Some("text/html"),
	);
	req.headers_mut().append(
		header::COOKIE,
		"unrelated=value; theme=dark"
			.parse()
			.expect("cookie header"),
	);
	req.headers_mut().append(
		header::COOKIE,
		format!("{}={encoded}", policy.session.cookie_name)
			.parse()
			.expect("cookie header"),
	);

	let proxy = setup_proxy_test("{}").expect("proxy test harness");
	let response = policy
		.apply(
			None,
			&mut req,
			crate::proxy::httpproxy::PolicyClient {
				inputs: proxy.inputs(),
			},
		)
		.await
		.expect("browser policy apply");
	assert!(response.direct_response.is_none());
	let claims = req
		.extensions()
		.get::<jwt::Claims>()
		.expect("claims extension");
	assert_eq!(claims.inner.get("sub"), Some(&json!("user-1")));
}

#[test]
fn strip_browser_auth_cookies_across_multiple_cookie_headers() {
	let mut req = request(Method::GET, "https://app.example.com/", None);
	req.headers_mut().append(
		header::COOKIE,
		http::HeaderValue::from_static("session=abc"),
	);
	req.headers_mut().append(
		header::COOKIE,
		http::HeaderValue::from_static("agw_oidc_s_test=encrypted; agw_oidc_t_test=txn"),
	);
	req
		.headers_mut()
		.append(header::COOKIE, http::HeaderValue::from_static("theme=dark"));

	crate::http::request_cookies::strip_cookies_by_prefix(&mut req, RESERVED_COOKIE_PREFIX);
	assert_eq!(
		req.headers().get(header::COOKIE).unwrap(),
		"session=abc; theme=dark"
	);
}
