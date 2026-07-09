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
async fn local_ratelimit() {
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

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);
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
