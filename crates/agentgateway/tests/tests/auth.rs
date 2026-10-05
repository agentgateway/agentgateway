use hyper_util::client::legacy::Client;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};

use crate::common::prelude::*;
use crate::tests::tls::route_with_prefix;

fn test_oidc_cookie_encoder() -> agentgateway::http::sessionpersistence::Encoder {
	agentgateway::http::sessionpersistence::Encoder::aes(
		"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
	)
	.expect("aes encoder")
}

pub(in crate::tests) fn setup_proxy_test_with_oidc() -> TestBind {
	let mut config = agentgateway::config::parse_config("{}".to_string(), None).expect("config");
	config.oidc_cookie_encoder = Some(test_oidc_cookie_encoder());
	setup_proxy_test_with_config(config)
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

fn signed_id_token(nonce: &str) -> String {
	jsonwebtoken::encode(
		&Header {
			alg: Algorithm::ES256,
			kid: Some(TEST_KEY_ID.into()),
			..Header::default()
		},
		&TestIdTokenClaims {
			iss: TEST_ISSUER,
			aud: TEST_CLIENT_ID,
			exp: agentgateway::http::oidc::now_unix() + 300,
			nonce,
			sub: "user-1",
		},
		&EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("encoding key"),
	)
	.expect("signed id token")
}

pub(in crate::tests) fn gateway_oidc_policy(token_endpoint: impl Into<String>) -> Value {
	json!({
		"oidc": {
			"issuer": TEST_ISSUER,
			"authorizationEndpoint": format!("{TEST_ISSUER}/authorize"),
			"tokenEndpoint": token_endpoint.into(),
			"jwks": serde_json::to_string(&test_jwks()).expect("jwks"),
			"clientId": TEST_CLIENT_ID,
			"clientSecret": "client-secret",
			"redirectURI": "http://lo/oauth/callback"
		}
	})
}

fn find_set_cookie_pair(headers: &::http::HeaderMap, prefix: &str) -> String {
	headers
		.get_all(header::SET_COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.find_map(|value| {
			let cookie = cookie::Cookie::parse(value.to_string()).ok()?;
			cookie
				.name()
				.starts_with(prefix)
				.then(|| format!("{}={}", cookie.name(), cookie.value()))
		})
		.unwrap_or_else(|| panic!("missing set-cookie with prefix {prefix}"))
}

fn query_param(uri: &str, name: &str) -> String {
	Url::parse(uri)
		.expect("absolute url")
		.query_pairs()
		.find_map(|(key, value)| (key == name).then(|| value.into_owned()))
		.unwrap_or_else(|| panic!("missing query param {name}"))
}

pub async fn oidc_backend_mock() -> (MockServer, Arc<StdMutex<Option<String>>>) {
	let token_response = Arc::new(StdMutex::new(None));
	let mock = MockServer::start().await;
	let token_response_clone = Arc::clone(&token_response);
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |req: &wiremock::Request| {
			if req.method == Method::POST && req.url.path() == "/token" {
				let id_token = token_response_clone
					.lock()
					.expect("token mutex")
					.clone()
					.expect("token response configured");
				return ResponseTemplate::new(200).set_body_json(json!({
					"id_token": id_token,
				}));
			}

			let request = RequestDump {
				method: req.method.clone(),
				uri: req.url.to_string().parse().expect("request uri"),
				headers: req.headers.clone(),
				body: bytes::Bytes::copy_from_slice(&req.body),
				version: req.version,
			};
			ResponseTemplate::new(200).set_body_json(request)
		})
		.mount(&mock)
		.await;
	(mock, token_response)
}

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgltxBTVDLg7C6vE1T
7OtwJIZ/dpm8ygE2MBTjPCY3hgahRANCAARYzu50EeBrT0rELmTGroaGtn0zdjxL
1lOGr9fGw5wOGcXO0+Gn5F5sIxGyTM0FwnUHFNz2SoixZR5dtxhNc+Lo
-----END PRIVATE KEY-----
";
const TEST_KEY_ID: &str = "kid-1";
const TEST_ISSUER: &str = "https://issuer.example.com";
const TEST_CLIENT_ID: &str = "client-id";

#[derive(Serialize)]
struct TestIdTokenClaims<'a> {
	iss: &'a str,
	aud: &'a str,
	exp: u64,
	nonce: &'a str,
	sub: &'a str,
}

#[tokio::test]
async fn reserved_oidc_cookies_are_stripped_before_proxying() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	let io = t.serve_http(BIND_KEY);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo",
		&[(
			"cookie",
			"agw_oidc_s_test=session; app_cookie=keep; agw_oidc_t_test=txn",
		)],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	let cookie = body
		.headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();
	assert!(cookie.contains("app_cookie=keep"));
	assert!(!cookie.contains("agw_oidc_s_test"));
	assert!(!cookie.contains("agw_oidc_t_test"));
}

#[tokio::test]
async fn gateway_phase_oidc_redirects_before_route_selection() {
	let (mock, _token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let io = bind.serve_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://lo/private").await;

	assert_eq!(res.status(), 302);
	let location = res.hdr(header::LOCATION);
	assert!(location.starts_with("https://issuer.example.com/authorize?"));
	assert!(location.contains("redirect_uri=http%3A%2F%2Flo%2Foauth%2Fcallback"));
}

#[tokio::test]
async fn gateway_phase_oidc_callback_authenticates_and_strips_reserved_cookies() {
	let (mock, token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let oidc = bind
		.pi
		.stores
		.read_binds()
		.gateway_policies(&agentgateway::types::agent::ListenerName::default())
		.oidc
		.iter()
		.next()
		.cloned()
		.expect("compiled gateway oidc policy")
		.pol;

	let io = bind.serve_http(BIND_KEY);
	let login = send_request(io.clone(), Method::GET, "http://lo/private").await;
	assert_eq!(login.status(), 302);

	let state = query_param(login.hdr(header::LOCATION), "state");
	let transaction_cookie = login
		.headers()
		.get(header::SET_COOKIE)
		.and_then(|value| value.to_str().ok())
		.expect("transaction set-cookie");
	let transaction_cookie =
		cookie::Cookie::parse(transaction_cookie.to_string()).expect("transaction cookie");
	let transaction = oidc
		.session
		.decode_transaction(transaction_cookie.value())
		.expect("decode transaction cookie");
	*token_response.lock().expect("token mutex") = Some(signed_id_token(&transaction.nonce));

	let callback = send_request_headers(
		io.clone(),
		Method::GET,
		&format!("http://lo/oauth/callback?code=auth-code&state={state}"),
		&[(
			"cookie",
			&format!(
				"{}={}",
				transaction_cookie.name(),
				transaction_cookie.value()
			),
		)],
	)
	.await;
	assert_eq!(callback.status(), 302);
	assert_eq!(callback.hdr(header::LOCATION), "/private");

	let session_cookie = find_set_cookie_pair(callback.headers(), "agw_oidc_s_");
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("cookie", &format!("{session_cookie}; app_cookie=keep"))],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	let cookie = body
		.headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();
	assert!(cookie.contains("app_cookie=keep"));
	assert!(!cookie.contains("agw_oidc_s_"));
	assert!(!cookie.contains("agw_oidc_t_"));
}

#[derive(Serialize)]
struct TestAccessTokenClaims<'a> {
	iss: &'a str,
	aud: &'a str,
	exp: u64,
	sub: &'a str,
}

/// A bearer token for `jwt_auth_policy`, signed with the same key as the OIDC ID tokens.
fn signed_access_token() -> String {
	jsonwebtoken::encode(
		&Header {
			alg: Algorithm::ES256,
			kid: Some(TEST_KEY_ID.into()),
			..Header::default()
		},
		&TestAccessTokenClaims {
			iss: TEST_ISSUER,
			aud: TEST_CLIENT_ID,
			exp: agentgateway::http::oidc::now_unix() + 300,
			sub: "cli-user",
		},
		&EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("encoding key"),
	)
	.expect("signed access token")
}

fn jwt_auth_policy(mode: &str) -> Value {
	json!({
		"issuer": TEST_ISSUER,
		"audiences": [TEST_CLIENT_ID],
		"jwks": serde_json::to_string(&test_jwks()).expect("jwks"),
		"mode": mode,
	})
}

/// The OIDC policy from `gateway_oidc_policy`, with `allowWithoutSession` set to `allow`, plus
/// `siblings`, as one policy document.
fn oidc_policy_with(token_endpoint: &str, allow: &[&str], siblings: Value) -> Value {
	let mut policy = gateway_oidc_policy(token_endpoint);
	if !allow.is_empty() {
		policy["oidc"]["allowWithoutSession"] = json!(allow);
	}
	let object = policy.as_object_mut().expect("policy object");
	for (key, value) in siblings.as_object().expect("sibling object") {
		object.insert(key.clone(), value.clone());
	}
	policy
}

/// A bind with one route at `prefix` to an OIDC-aware backend mock, with no policies attached.
/// The token-response slot lets a test complete the browser login flow.
async fn oidc_test_bind(prefix: &str) -> (MockServer, Arc<StdMutex<Option<String>>>, TestBind) {
	let (mock, token_response) = oidc_backend_mock().await;
	let bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), prefix));
	(mock, token_response, bind)
}

/// [`oidc_policy_with`], using `mock` as the token endpoint.
fn oidc_policy_for(mock: &MockServer, allow: &[&str], siblings: Value) -> Value {
	oidc_policy_with(&format!("{}/token", mock.uri()), allow, siblings)
}

/// A bind whose `/upstream` route carries OIDC plus `siblings` as route policies.
/// The returned bind must outlive the requests sent through the client.
async fn oidc_route_setup(
	allow: &[&str],
	siblings: Value,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	bind
		.attach_route_policy(oidc_policy_for(&mock, allow, siblings))
		.await;
	let io = bind.serve_http(BIND_KEY);
	(mock, bind, io)
}

/// Replaces the route-phase jwtAuth policy attached to `bind` with the same policy behind
/// `condition`.
fn make_route_jwt_conditional(bind: &mut TestBind, condition: &str) {
	use agentgateway::types::agent::{PolicyPhase, PolicyType, TrafficPolicy};
	let jwt_policy = bind
		.pi
		.stores
		.read_binds()
		.all_policies()
		.into_iter()
		.find(|policy| {
			matches!(
				&policy.policy,
				PolicyType::Traffic(traffic)
					if traffic.phase == PolicyPhase::Route
						&& matches!(traffic.policy, TrafficPolicy::JwtAuth(_))
			)
		})
		.expect("compiled route jwtAuth policy");
	let PolicyType::Traffic(traffic) = &jwt_policy.policy else {
		unreachable!()
	};
	let TrafficPolicy::JwtAuth(jwt) = &traffic.policy else {
		unreachable!()
	};
	let condition =
		Arc::new(agentgateway::cel::Expression::new_strict(condition).expect("condition"));
	let conditional = agentgateway::store::RequestPolicy::from_policy_inners(
		jwt
			.clone()
			.into_policy_inners()
			.into_iter()
			.map(|mut entry| {
				entry.condition = Some(condition.clone());
				entry
			}),
	);
	let mut replacement = (*jwt_policy).clone();
	replacement.policy = (TrafficPolicy::JwtAuth(conditional), traffic.phase).into();
	bind.with_policy(replacement);
}

fn assert_oidc_login_redirect(res: &Response) {
	assert_eq!(res.status(), 302);
	assert!(
		res
			.hdr(header::LOCATION)
			.starts_with("https://issuer.example.com/authorize?"),
		"expected the OIDC login redirect"
	);
}

fn assert_rejected_without_login(res: &Response) {
	assert_eq!(res.status(), 401);
	assert!(
		res.headers().get(header::LOCATION).is_none(),
		"a rejected credential must not start the OIDC login flow"
	);
}

/// Completes the browser login flow against `oidc` and returns the session cookie pair.
async fn login_session_cookie(
	io: Client<MemoryConnector, Body>,
	oidc: &agentgateway::http::oidc::OidcPolicy,
	token_response: &Arc<StdMutex<Option<String>>>,
) -> String {
	let login = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/private",
		&[("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&login);

	let state = query_param(login.hdr(header::LOCATION), "state");
	let transaction_cookie = login
		.headers()
		.get(header::SET_COOKIE)
		.and_then(|value| value.to_str().ok())
		.expect("transaction set-cookie");
	let transaction_cookie =
		cookie::Cookie::parse(transaction_cookie.to_string()).expect("transaction cookie");
	let transaction = oidc
		.session
		.decode_transaction(transaction_cookie.value())
		.expect("decode transaction cookie");
	*token_response.lock().expect("token mutex") = Some(signed_id_token(&transaction.nonce));

	let callback = send_request_headers(
		io,
		Method::GET,
		&format!(
			"http://lo{}?code=auth-code&state={state}",
			oidc.redirect_uri.callback_path.path()
		),
		&[(
			"cookie",
			&format!(
				"{}={}",
				transaction_cookie.name(),
				transaction_cookie.value()
			),
		)],
	)
	.await;
	assert_eq!(callback.status(), 302);
	find_set_cookie_pair(callback.headers(), "agw_oidc_s_")
}

/// The compiled route-phase OIDC policy attached to the test route.
fn compiled_route_oidc(bind: &TestBind) -> Arc<agentgateway::http::oidc::OidcPolicy> {
	bind
		.pi
		.stores
		.read_binds()
		.all_policies()
		.into_iter()
		.find_map(|policy| match &policy.policy {
			agentgateway::types::agent::PolicyType::Traffic(traffic)
				if traffic.phase == agentgateway::types::agent::PolicyPhase::Route =>
			{
				match &traffic.policy {
					agentgateway::types::agent::TrafficPolicy::Oidc(oidc) => {
						oidc.iter().next().map(|entry| entry.pol.clone())
					},
					_ => None,
				}
			},
			_ => None,
		})
		.expect("compiled route oidc policy")
}

/// A bind whose `/` route carries OIDC plus `siblings`, with a browser session already logged in.
/// The route matches the callback path so a route-phase login can complete.
async fn oidc_route_session_setup(
	allow: &[&str],
	siblings: Value,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>, String) {
	let (mock, token_response, mut bind) = oidc_test_bind("/").await;
	bind
		.attach_route_policy(oidc_policy_for(&mock, allow, siblings))
		.await;
	let oidc = compiled_route_oidc(&bind);
	let io = bind.serve_http(BIND_KEY);
	let session_cookie = login_session_cookie(io.clone(), &oidc, &token_response).await;
	(mock, bind, io, session_cookie)
}

// ---- `optional` sibling: either the session or the credential is enough.

#[tokio::test]
async fn route_oidc_with_optional_jwt_accepts_valid_bearer_without_session() {
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("optional")}),
	)
	.await;

	let token = signed_access_token();
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_eq!(res.status(), 200);
	// jwtAuth validated the token itself, so it is stripped before forwarding as usual.
	let upstream = read_body(res.into_body()).await;
	assert!(upstream.headers.get(header::AUTHORIZATION).is_none());

	// Without any credential the browser flow still applies.
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_with_optional_jwt_rejects_invalid_bearer() {
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("optional")}),
	)
	.await;

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("authorization", "Bearer not-a-token")],
	)
	.await;
	assert_rejected_without_login(&res);
}

#[tokio::test]
async fn route_oidc_with_optional_jwt_still_rejects_fetch_without_credential() {
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("optional")}),
	)
	.await;

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("sec-fetch-mode", "cors")],
	)
	.await;
	assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn route_oidc_with_optional_jwt_accepts_session_and_validates_present_bearer() {
	let (_mock, _bind, io, session_cookie) = oidc_route_session_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("optional")}),
	)
	.await;

	// Session alone is enough; jwtAuth is optional and sees no token.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("cookie", &session_cookie)],
	)
	.await;
	assert_eq!(res.status(), 200);

	// A token presented alongside the session is validated by jwtAuth as usual: `optional`
	// means not required, not unchecked.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", &session_cookie),
			("authorization", "Bearer not-a-token"),
		],
	)
	.await;
	assert_rejected_without_login(&res);

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", &session_cookie),
			("authorization", &format!("Bearer {token}")),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	let upstream = read_body(res.into_body()).await;
	assert!(upstream.headers.get(header::AUTHORIZATION).is_none());
}

#[tokio::test]
async fn route_oidc_with_optional_api_key_accepts_either() {
	let api_key = json!({
		"keys": [{"key": "sk-123", "metadata": {"group": "eng"}}],
		"mode": "optional",
		"location": {"header": {"name": "x-api-key"}},
	});
	let (_mock, _bind, io) = oidc_route_setup(&["apiKey"], json!({"apiKey": api_key})).await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("x-api-key", "sk-123")],
	)
	.await;
	assert_eq!(res.status(), 200);

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("x-api-key", "sk-999")],
	)
	.await;
	assert_rejected_without_login(&res);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_with_optional_basic_auth_accepts_either() {
	use base64::Engine;
	let basic_auth = json!({
		"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00",
		"mode": "optional",
	});
	let (_mock, _bind, io) = oidc_route_setup(&["basicAuth"], json!({"basicAuth": basic_auth})).await;

	let good = base64::prelude::BASE64_STANDARD.encode(b"user:password");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Basic {good}"))],
	)
	.await;
	assert_eq!(res.status(), 200);

	let bad = base64::prelude::BASE64_STANDARD.encode(b"user:wrong");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Basic {bad}"))],
	)
	.await;
	assert_rejected_without_login(&res);

	// A bearer is not a basic-auth credential, so it does not let OIDC step aside.
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", "Bearer whatever"),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

// ---- `strict` sibling: both the session and the credential are required, as before.

#[tokio::test]
async fn route_oidc_with_strict_jwt_requires_both() {
	let (_mock, _bind, io, session_cookie) =
		oidc_route_session_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth_policy("strict")})).await;
	let token = signed_access_token();

	// A valid bearer alone does not bypass OIDC: strict jwtAuth is an additional requirement.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);

	// A session alone is rejected by strict jwtAuth.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("cookie", &session_cookie)],
	)
	.await;
	assert_eq!(res.status(), 401);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", &session_cookie),
			("authorization", &format!("Bearer {token}")),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
}

// ---- the opt-in: without it, nothing changes.

#[tokio::test]
async fn route_oidc_without_opt_in_keeps_requiring_session() {
	// Default-mode jwtAuth is `optional`. Without `allowWithoutSession`, a bearer alone still
	// enters login, as before.
	let mut jwt_auth = jwt_auth_policy("optional");
	jwt_auth.as_object_mut().unwrap().remove("mode");
	let (_mock, _bind, io, session_cookie) =
		oidc_route_session_setup(&[], json!({"jwtAuth": jwt_auth})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);

	// The session still works, and a token presented with it is still validated.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("cookie", &session_cookie)],
	)
	.await;
	assert_eq!(res.status(), 200);
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", &session_cookie),
			("authorization", "Bearer not-a-token"),
		],
	)
	.await;
	assert_rejected_without_login(&res);
}

#[tokio::test]
async fn route_oidc_opt_in_is_per_sibling() {
	// Allowing apiKey does not allow jwtAuth.
	let (_mock, _bind, io) =
		oidc_route_setup(&["apiKey"], json!({"jwtAuth": jwt_auth_policy("optional")})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

// ---- cases that must never let OIDC step aside.

#[tokio::test]
async fn route_oidc_ignores_conditional_sibling() {
	// A conditional sibling cannot be probed deterministically, so the session stays required.
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	bind
		.attach_route_policy(oidc_policy_for(
			&mock,
			&["jwtAuth"],
			json!({"jwtAuth": jwt_auth_policy("optional")}),
		))
		.await;
	make_route_jwt_conditional(&mut bind, "true");
	let io = bind.serve_http(BIND_KEY);

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_ignores_expression_location() {
	// A CEL credential location cannot be probed deterministically, so the session stays required.
	let mut jwt_auth = jwt_auth_policy("optional");
	jwt_auth["location"] = json!({"expression": "request.headers[\"x-token\"]"});
	let (_mock, _bind, io) = oidc_route_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("x-token", &token), ("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_ignores_mcp_well_known_credential() {
	// MCP-enabled jwtAuth skips validation on the OAuth well-known endpoints, so a credential there
	// must not let OIDC step aside. Elsewhere it is validated as usual.
	let mcp_authentication = json!({
		"issuer": TEST_ISSUER,
		"audiences": [TEST_CLIENT_ID],
		"jwks": serde_json::to_string(&test_jwks()).expect("jwks"),
		"mode": "optional",
		"resourceMetadata": {"mcpResourceUri": "mcp://test"},
	});
	// The route must also match the well-known path for the request to reach the policies.
	let (mock, _token_response, mut bind) = oidc_test_bind("/").await;
	bind
		.attach_route_policy(oidc_policy_for(
			&mock,
			&["jwtAuth"],
			json!({"mcpAuthentication": mcp_authentication}),
		))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let token = signed_access_token();
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/.well-known/oauth-protected-resource/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn route_oidc_ignores_permissive_jwt() {
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("permissive")}),
	)
	.await;

	// Permissive jwtAuth would let an invalid token through, so OIDC must keep enforcing.
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", "Bearer not-a-token"),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_without_sibling_policy_ignores_bearer() {
	let (_mock, _bind, io) = oidc_route_setup(&["jwtAuth"], json!({})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_fallthrough_ignores_reserved_cookie_credential() {
	// Reserved `agw_oidc_` cookies are stripped before the route siblings run, so a sibling reading
	// one never qualifies.
	let mut jwt_auth = jwt_auth_policy("optional");
	jwt_auth["location"] = json!({"cookie": {"name": "agw_oidc_s_bogus"}});
	let (_mock, _bind, io) = oidc_route_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth})).await;

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", "agw_oidc_s_bogus=not-a-token"),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn route_oidc_fallthrough_keeps_credentials_in_other_cookie_headers() {
	// A reserved cookie in a separate Cookie header does not hide the credential from the sibling.
	let mut jwt_auth = jwt_auth_policy("optional");
	jwt_auth["location"] = json!({"cookie": {"name": "token"}});
	let (_mock, _bind, io) = oidc_route_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", "agw_oidc_s_bogus=junk"),
			("cookie", &format!("token={token}")),
		],
	)
	.await;
	assert_eq!(res.status(), 200);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", "agw_oidc_s_bogus=junk"),
			("cookie", "token=not-a-token"),
			("accept", "text/html"),
		],
	)
	.await;
	assert_rejected_without_login(&res);
}

// ---- phases.

/// A bind whose gateway phase carries OIDC plus `siblings`.
async fn oidc_gateway_setup(
	allow: &[&str],
	siblings: Value,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	bind
		.attach_gateway_policy(oidc_policy_for(&mock, allow, siblings))
		.await;
	let io = bind.serve_http(BIND_KEY);
	(mock, bind, io)
}

#[tokio::test]
async fn gateway_phase_oidc_with_strict_jwt_keeps_requiring_session() {
	let (_mock, _bind, io) =
		oidc_gateway_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth_policy("strict")})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn gateway_phase_oidc_ignores_reserved_cookie_credential() {
	// A sibling reading a reserved cookie never qualifies, in the gateway phase too, even though
	// that phase does not strip them.
	let mut jwt_auth = jwt_auth_policy("optional");
	jwt_auth["location"] = json!({"cookie": {"name": "agw_oidc_s_bogus"}});
	let (_mock, _bind, io) = oidc_gateway_setup(&["jwtAuth"], json!({"jwtAuth": jwt_auth})).await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("cookie", &format!("agw_oidc_s_bogus={token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn gateway_phase_oidc_with_optional_jwt_accepts_valid_bearer() {
	let (_mock, _bind, io) = oidc_gateway_setup(
		&["jwtAuth"],
		json!({"jwtAuth": jwt_auth_policy("optional")}),
	)
	.await;

	let token = signed_access_token();
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_eq!(res.status(), 200);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn oidc_fallthrough_does_not_cross_phases() {
	// Gateway-level OIDC only steps aside for a gateway-level sibling. A route-level optional
	// jwtAuth runs after routing and cannot satisfy the gateway phase.
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	bind
		.attach_gateway_policy(oidc_policy_for(&mock, &["jwtAuth"], json!({})))
		.await;
	bind
		.attach_route_policy(json!({"jwtAuth": jwt_auth_policy("optional")}))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("authorization", &format!("Bearer {token}")),
			("accept", "text/html"),
		],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

// ---- only a listed sibling may stand in for the session.

#[tokio::test]
async fn route_oidc_rejects_unlisted_sibling_consuming_credential() {
	// jwtAuth and apiKey both read `Authorization: Bearer`. The listed apiKey starts the
	// fall-through, but the unlisted jwtAuth runs first and consumes the bearer, so reject.
	let api_key = json!({"keys": [{"key": "sk-123"}], "mode": "optional"});
	let (_mock, _bind, io) = oidc_route_setup(
		&["apiKey"],
		json!({"jwtAuth": jwt_auth_policy("optional"), "apiKey": api_key}),
	)
	.await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_rejected_without_login(&res);
	assert!(
		!String::from_utf8_lossy(&read_body!(res)).contains("allowWithoutSession"),
		"the rejection must not expose configuration details"
	);
}

#[tokio::test]
async fn route_oidc_rejects_listed_but_ineligible_sibling() {
	// A listed `strict` jwtAuth is not a session alternative. The optional apiKey starts the
	// fall-through, but the strict jwtAuth consumes the bearer, so a valid JWT alone is rejected.
	let api_key = json!({"keys": [{"key": "sk-123"}], "mode": "optional"});
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth", "apiKey"],
		json!({"jwtAuth": jwt_auth_policy("strict"), "apiKey": api_key}),
	)
	.await;

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_rejected_without_login(&res);
}

#[tokio::test]
async fn route_oidc_deferred_check_ignores_claims_from_earlier_phase() {
	// The route-phase check must ignore the gateway-phase apiKey's success; otherwise it would
	// pass while the unlisted route jwtAuth consumed the bearer.
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	bind
		.attach_gateway_policy(json!({"apiKey": {
			"keys": [{"key": "sk-gateway"}],
			"mode": "optional",
			"location": {"header": {"name": "x-gateway-key"}},
		}}))
		.await;
	bind
		.attach_route_policy(oidc_policy_for(
			&mock,
			&["apiKey"],
			json!({
				"jwtAuth": jwt_auth_policy("optional"),
				"apiKey": {"keys": [{"key": "sk-123"}], "mode": "optional"},
			}),
		))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let token = signed_access_token();
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("x-gateway-key", "sk-gateway"),
			("authorization", &format!("Bearer {token}")),
		],
	)
	.await;
	assert_rejected_without_login(&res);
}

#[tokio::test]
async fn route_oidc_deferral_keeps_earlier_claims_visible_to_conditions() {
	// Deferring must not hide gateway-phase claims from route-phase conditions, or a step-up
	// jwtAuth conditioned on them would be skipped.
	let (mock, _token_response, mut bind) = oidc_test_bind("/upstream").await;
	let mut gateway_jwt = jwt_auth_policy("optional");
	gateway_jwt["location"] = json!({"header": {"name": "x-gateway-token"}});
	bind
		.attach_gateway_policy(json!({"jwtAuth": gateway_jwt}))
		.await;
	let mut step_up = jwt_auth_policy("strict");
	step_up["location"] = json!({"header": {"name": "x-step-up-token"}});
	bind
		.attach_route_policy(oidc_policy_for(
			&mock,
			&["apiKey"],
			json!({
				"jwtAuth": step_up,
				"apiKey": {
					"keys": [{"key": "sk-123"}],
					"mode": "optional",
					"location": {"header": {"name": "x-api-key"}},
				},
			}),
		))
		.await;
	make_route_jwt_conditional(&mut bind, "has(jwt.sub)");
	let io = bind.serve_http(BIND_KEY);
	let token = signed_access_token();

	// The gateway identified the caller, so the step-up jwtAuth applies and requires its token.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("x-gateway-token", &token), ("x-api-key", "sk-123")],
	)
	.await;
	assert_rejected_without_login(&res);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[
			("x-gateway-token", &token),
			("x-api-key", "sk-123"),
			("x-step-up-token", &token),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn route_oidc_with_multiple_listed_siblings_accepts_any() {
	// Each listed sibling can stand in alone, and every present credential is still validated.
	// The apiKey reads a query parameter.
	let api_key = json!({
		"keys": [{"key": "sk-123"}],
		"mode": "optional",
		"location": {"queryParameter": {"name": "api_key"}},
	});
	let (_mock, _bind, io) = oidc_route_setup(
		&["jwtAuth", "apiKey"],
		json!({"jwtAuth": jwt_auth_policy("optional"), "apiKey": api_key}),
	)
	.await;
	let token = signed_access_token();

	let res = send_request(io.clone(), Method::GET, "http://lo/upstream?api_key=sk-123").await;
	assert_eq!(res.status(), 200);
	let upstream = read_body(res.into_body()).await;
	assert!(
		!upstream.uri.to_string().contains("api_key"),
		"apiKey strips its validated credential before forwarding"
	);

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/upstream",
		&[("authorization", &format!("Bearer {token}"))],
	)
	.await;
	assert_eq!(res.status(), 200);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream?api_key=sk-123",
		&[("authorization", "Bearer not-a-token")],
	)
	.await;
	assert_rejected_without_login(&res);
}

// ---- `strict` and `permissive` siblings never let OIDC step aside, for every policy type.

#[rstest::rstest]
#[case::api_key_strict(
	"apiKey",
	json!({"keys": [{"key": "sk-123"}], "mode": "strict", "location": {"header": {"name": "x-api-key"}}}),
	("x-api-key", "sk-123".to_string()),
)]
#[case::api_key_permissive(
	"apiKey",
	json!({"keys": [{"key": "sk-123"}], "mode": "permissive", "location": {"header": {"name": "x-api-key"}}}),
	("x-api-key", "sk-123".to_string()),
)]
#[case::basic_auth_strict(
	"basicAuth",
	json!({"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00", "mode": "strict"}),
	// base64("user:password")
	("authorization", "Basic dXNlcjpwYXNzd29yZA==".to_string()),
)]
#[tokio::test]
async fn route_oidc_ignores_non_optional_sibling(
	#[case] kind: &str,
	#[case] policy: Value,
	#[case] credential: (&str, String),
) {
	let (_mock, _bind, io) = oidc_route_setup(&[kind], json!({ kind: policy })).await;

	// Even a valid credential does not replace the session.
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[(credential.0, &credential.1), ("accept", "text/html")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

// ---- the built-in UI's OIDC shape: a managed login endpoint with a pre-login redirect.

/// A bind whose `/` route carries OIDC configured the way the built-in UI configures it
/// (`login.path` `/api/auth/login`, `login.redirect` `/ui/login`), plus an optional apiKey.
async fn ui_shaped_oidc_setup(
	allow: &[&str],
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let (mock, _token_response, mut bind) = oidc_test_bind("/").await;
	let mut policy = oidc_policy_for(
		&mock,
		allow,
		json!({"apiKey": {
			"keys": [{"key": "sk-123"}],
			"mode": "optional",
			"location": {"header": {"name": "x-api-key"}},
		}}),
	);
	policy["oidc"]["login"] = json!({"path": "/api/auth/login", "redirect": "/ui/login"});
	bind.attach_route_policy(policy).await;
	let io = bind.serve_http(BIND_KEY);
	(mock, bind, io)
}

fn assert_ui_login_required(res: &Response) {
	assert_eq!(res.status(), 401);
	assert_eq!(res.hdr(header::LOCATION), "/ui/login");
}

#[tokio::test]
async fn ui_shaped_oidc_with_optional_api_key_reaches_config_api() {
	let (_mock, _bind, io) = ui_shaped_oidc_setup(&["apiKey"]).await;

	// A fetch carrying a valid key reaches the config API without a session.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/api/config",
		&[("sec-fetch-mode", "cors"), ("x-api-key", "sk-123")],
	)
	.await;
	assert_eq!(res.status(), 200);

	// An invalid key is rejected by apiKey, without pointing the caller at the login page.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/api/config",
		&[("sec-fetch-mode", "cors"), ("x-api-key", "sk-999")],
	)
	.await;
	assert_rejected_without_login(&res);

	// A fetch without any credential still gets the UI's login hint.
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/api/config",
		&[("sec-fetch-mode", "cors")],
	)
	.await;
	assert_ui_login_required(&res);

	// The managed login endpoint still starts login, even with a key present.
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/api/auth/login",
		&[("accept", "text/html"), ("x-api-key", "sk-123")],
	)
	.await;
	assert_oidc_login_redirect(&res);
}

#[tokio::test]
async fn ui_shaped_oidc_without_opt_in_keeps_config_api_behind_login() {
	let (_mock, _bind, io) = ui_shaped_oidc_setup(&[]).await;

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/api/config",
		&[("sec-fetch-mode", "cors"), ("x-api-key", "sk-123")],
	)
	.await;
	assert_ui_login_required(&res);
}

#[tokio::test]
async fn gateway_phase_authorization_runs_before_route_selection() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"authorization": {
				"rules": [
					{"allow": "request.headers[\"x-pre-routing\"] == \"yes\""}
				]
			}
		}))
		.await;

	let denied = send_request(io.clone(), Method::GET, "http://lo/no-route-needed").await;
	assert_eq!(denied.status(), 403);
	assert_eq!(read_body!(denied).as_ref(), b"authorization failed");

	let allowed = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("x-pre-routing", "yes")],
	)
	.await;
	assert_eq!(allowed.status(), 200);
}

#[tokio::test]
async fn network_authorization_allow() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_frontend_policy(json!({
			"networkAuthorization": {
				"rules": ["source.port == 12345"], // NOTE: the tests hardcode a dummy src port that matches
			},
		}))
		.await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn network_authorization_deny() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_frontend_policy(json!({
			"networkAuthorization": {
				"rules": ["source.port == 54321"], // NOTE: the tests hardcode a dummy src port that does not match
			},
		}))
		.await;

	RequestBuilder::new(Method::GET, "http://lo")
		.send(io)
		.await
		.expect_err("should be denied");
}

#[tokio::test]
async fn network_http_ext_authz_denies_tcp_connection() {
	let backend = simple_mock().await;
	let (_backend, mut bind, io) = setup_tcp_mock(backend);
	let authz = MockServer::start().await;
	Mock::given(wiremock::matchers::path("/check"))
		.respond_with(ResponseTemplate::new(403))
		.mount(&authz)
		.await;

	bind
		.attach_frontend_policy(json!({
			"networkExtAuthz": {
				"host": authz.address().to_string(),
				"protocol": {
					"http": {
						"path": "\"/check\""
					}
				}
			}
		}))
		.await;

	RequestBuilder::new(Method::GET, "http://lo")
		.send(io)
		.await
		.expect_err("network ext authz should close a denied TCP connection");
	assert_eq!(authz.received_requests().await.unwrap().len(), 1);
}

#[rstest::rstest]
#[case::upstream(false)]
#[case::direct_response(true)]
#[tokio::test]
async fn local_ratelimit(#[case] direct_response: bool) {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "1s",
			}],
		}))
		.await;

	if direct_response {
		bind
			.attach_route_policy(json!({
				"directResponse": {"status": 200, "body": "hello"}
			}))
			.await;
	}

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	assert!(res.headers().get(header::RETRY_AFTER).is_none());
	// Allowed responses advertise the current limit so clients can self-throttle.
	assert_eq!(res.hdr("x-ratelimit-limit"), "1");
	assert_eq!(res.hdr("x-ratelimit-remaining"), "0");
	assert!(!res.hdr("x-ratelimit-reset").is_empty());

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);
	// The 429 still carries the limit info.
	assert_eq!(res.hdr("x-ratelimit-limit"), "1");
	assert_eq!(res.hdr("x-ratelimit-remaining"), "0");
	assert_eq!(res.hdr("retry-after"), "1");
}

#[tokio::test]
async fn mcp_authentication_runs_in_route_policy_path() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
      .attach_route_policy(json!({
			"mcpAuthentication": {
				"issuer": "https://example.com",
				"audiences": ["test-aud"],
				"jwks": "{\"keys\":[{\"use\":\"sig\",\"kty\":\"EC\",\"kid\":\"XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI\",\"crv\":\"P-256\",\"alg\":\"ES256\",\"x\":\"XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo\",\"y\":\"g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE\"}]}",
				"resourceMetadata": {
					"mcpResourceUri": "mcp://test"
				}
			}
		}))
      .await;

	let res = send_request(
		io,
		Method::GET,
		"http://lo/.well-known/oauth-protected-resource/mcp",
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("content-type"), "application/json");
}

#[tokio::test]
async fn api_key() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"apiKey": {
				"keys": [
					{
						"key": "sk-123",
						"metadata": {"group": "eng"},
					},
					{
						"key": "sk-456",
						"metadata": {"group": "sales"},
					}
				],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'eng'"],
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-123")],
	)
	.await;
	assert_eq!(res.status(), 200);
	// Match but fails authz
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-456")],
	)
	.await;
	assert_eq!(res.status(), 403);
	// No match
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-789")],
	)
	.await;
	assert_eq!(res.status(), 401);
	// No match
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn basic_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
      .attach_route_policy(json!({
			"basicAuth": {
				"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00\nbcrypt_test:$2y$05$nC6nErr9XZJuMJ57WyCob.EuZEjylDt2KaHfbfOtyb.EgL1I2jCVa\nsha1_test:{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=\ncrypt_test:bGVh02xkuGli2",
				"realm": "my-realm",
				"mode": "strict",
			},
			"authorization": {
				"rules": ["basicAuth.username == 'user'"],
			},
		}))
      .await;

	use base64::Engine;
	let md5 = base64::prelude::BASE64_STANDARD.encode(b"user:password");
	let sha1 = base64::prelude::BASE64_STANDARD.encode(b"sha1_test:password");
	let bcrypt = base64::prelude::BASE64_STANDARD.encode(b"bcrypt_test:password");
	let crypt = base64::prelude::BASE64_STANDARD.encode(b"crypt_test:password");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {md5}"))],
	)
	.await;
	assert_eq!(res.status(), 200);
	// Match but fails authz
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {sha1}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {crypt}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {bcrypt}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	// No match
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);
	let md5_wrong = base64::prelude::BASE64_STANDARD.encode(b"user:not-password");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {md5_wrong}"))],
	)
	.await;
	assert_eq!(res.status(), 401);
}
