use http::StatusCode;
use serde_json::json;
use url::Url;
use wiremock::matchers::{body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;
use crate::common::oauth2::{
	find_cookie_pair, gateway_url, require_e2e, session_cookie_header, set_cookie_values,
};

pub mod keycloak;

#[tokio::test]
async fn oauth2_backend_passthrough_refresh_and_logout() {
	let Some((gateway, oidc, client)) = setup_gateway(true).await else {
		return;
	};

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=authorization_code"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "access-initial",
			"token_type": "Bearer",
			"expires_in": 0,
			"refresh_token": "refresh-1"
		})))
		.expect(1)
		.mount(&oidc)
		.await;

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=refresh_token"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "access-refreshed",
			"token_type": "Bearer",
			"expires_in": 3600,
			"refresh_token": "refresh-2"
		})))
		.expect(1)
		.mount(&oidc)
		.await;

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data?x=1").await;
	let callback = complete_callback(&client, &gateway, "code-1", &state, &nonce_cookie).await;
	assert_eq!(callback.status(), StatusCode::FOUND);
	assert_eq!(
		callback
			.headers()
			.get(reqwest::header::LOCATION)
			.unwrap()
			.to_str()
			.unwrap(),
		"/private/data?x=1"
	);

	let session_cookie = session_cookie_header(&set_cookie_values(&callback))
		.expect("session cookie must be set after callback");

	let protected = client
		.get(gateway_url(&gateway, "/private/data?x=1"))
		.header(reqwest::header::COOKIE, &session_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(protected.status(), StatusCode::OK);
	assert!(
		!set_cookie_values(&protected).is_empty(),
		"refresh should update session cookie"
	);
	let upstream_body: serde_json::Value = protected.json().await.unwrap();
	assert_eq!(
		upstream_body.get("authorization").and_then(|v| v.as_str()),
		Some("Bearer access-refreshed")
	);

	let logout = client
		.get(gateway_url(&gateway, "/logout"))
		.header(reqwest::header::COOKIE, &session_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(logout.status(), StatusCode::OK);
	let logout_cookies = set_cookie_values(&logout);
	assert!(
		logout_cookies.iter().any(|v| {
			v.starts_with("__Host-ag-session=") && v.contains("Max-Age=0") && v.contains("HttpOnly")
		}),
		"logout must clear session cookie"
	);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_without_backend_passthrough_does_not_forward_authorization() {
	let Some((gateway, oidc, client)) = setup_gateway(false).await else {
		return;
	};

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=authorization_code"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "access-no-forward",
			"token_type": "Bearer",
			"expires_in": 3600,
			"refresh_token": "refresh-1"
		})))
		.expect(1)
		.mount(&oidc)
		.await;

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data").await;
	let callback = complete_callback(&client, &gateway, "code-2", &state, &nonce_cookie).await;
	assert_eq!(callback.status(), StatusCode::FOUND);

	let session_cookie = session_cookie_header(&set_cookie_values(&callback))
		.expect("session cookie must be set after callback");

	let protected = client
		.get(gateway_url(&gateway, "/private/data"))
		.header(reqwest::header::COOKIE, session_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(protected.status(), StatusCode::OK);
	let upstream_body: serde_json::Value = protected.json().await.unwrap();
	assert!(
		upstream_body.get("authorization").is_none()
			|| upstream_body
				.get("authorization")
				.and_then(|v| v.as_str())
				.is_none(),
		"authorization header must not be forwarded when backendAuth.passthrough is disabled"
	);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_callback_requires_handshake_cookie() {
	let Some((gateway, _oidc, client)) = setup_gateway(true).await else {
		return;
	};

	let (_, state, _) = start_auth(&client, &gateway, "/private/data").await;

	let callback = client
		.get(callback_url(&gateway, "code-missing-cookie", &state))
		.send()
		.await
		.unwrap();
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_callback_missing_state_is_rejected() {
	let Some((gateway, _oidc, client)) = setup_gateway(true).await else {
		return;
	};

	let callback = client
		.get(gateway_url(
			&gateway,
			"/_gateway/callback?code=missing-state",
		))
		.send()
		.await
		.unwrap();
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_callback_missing_code_is_rejected() {
	let Some((gateway, _oidc, client)) = setup_gateway(true).await else {
		return;
	};

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data").await;

	let mut callback_url = Url::parse(&gateway_url(&gateway, "/_gateway/callback")).unwrap();
	callback_url.query_pairs_mut().append_pair("state", &state);

	let callback = client
		.get(callback_url)
		.header(reqwest::header::COOKIE, nonce_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_callback_mismatched_handshake_cookie_is_rejected() {
	let Some((gateway, _oidc, client)) = setup_gateway(true).await else {
		return;
	};

	let (_, state_one, _) = start_auth(&client, &gateway, "/private/data").await;
	let (_, _, nonce_cookie_two) = start_auth(&client, &gateway, "/private/data").await;

	let callback = complete_callback(
		&client,
		&gateway,
		"code-mismatched-cookie",
		&state_one,
		&nonce_cookie_two,
	)
	.await;
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_api_clients_receive_401_bearer_challenge() {
	let Some((gateway, oidc, client)) = setup_gateway(true).await else {
		return;
	};

	let resp = client
		.get(gateway_url(&gateway, "/private/data"))
		.header(reqwest::header::ACCEPT, "application/json")
		.send()
		.await
		.unwrap();
	assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
	let www_auth = resp
		.headers()
		.get(reqwest::header::WWW_AUTHENTICATE)
		.and_then(|v| v.to_str().ok())
		.unwrap_or_default()
		.to_string();
	assert!(www_auth.contains("Bearer realm=\""));
	assert!(www_auth.contains(&oidc.uri()));
	assert!(www_auth.contains("scope=\"openid profile\""));

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_exchange_failure_is_rejected() {
	let Some((gateway, oidc, client)) = setup_gateway(true).await else {
		return;
	};

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=authorization_code"))
		.respond_with(ResponseTemplate::new(500))
		.expect(1)
		.mount(&oidc)
		.await;

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data").await;
	let callback = complete_callback(&client, &gateway, "bad-code", &state, &nonce_cookie).await;
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_non_bearer_token_type_is_rejected() {
	let Some((gateway, oidc, client)) = setup_gateway(true).await else {
		return;
	};

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=authorization_code"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "access-token",
			"token_type": "mac",
			"expires_in": 3600,
			"refresh_token": "refresh-1"
		})))
		.expect(1)
		.mount(&oidc)
		.await;

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data").await;
	let callback =
		complete_callback(&client, &gateway, "bad-token-type", &state, &nonce_cookie).await;
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	gateway.shutdown().await;
}

#[tokio::test]
async fn oauth2_refresh_failure_restarts_browser_auth() {
	let Some((gateway, oidc, client)) = setup_gateway(true).await else {
		return;
	};

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=authorization_code"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "access-initial",
			"token_type": "Bearer",
			"expires_in": 0,
			"refresh_token": "refresh-stale"
		})))
		.expect(1)
		.mount(&oidc)
		.await;

	Mock::given(method("POST"))
		.and(path("/token"))
		.and(body_string_contains("grant_type=refresh_token"))
		.respond_with(ResponseTemplate::new(500))
		.expect(1)
		.mount(&oidc)
		.await;

	let (_, state, nonce_cookie) = start_auth(&client, &gateway, "/private/data").await;
	let callback = complete_callback(
		&client,
		&gateway,
		"code-refresh-fail",
		&state,
		&nonce_cookie,
	)
	.await;
	assert_eq!(callback.status(), StatusCode::FOUND);
	let session_cookie = session_cookie_header(&set_cookie_values(&callback))
		.expect("session cookie must be set after callback");

	let resp = client
		.get(gateway_url(&gateway, "/private/data"))
		.header(reqwest::header::ACCEPT, "text/html")
		.header(reqwest::header::COOKIE, session_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(resp.status(), StatusCode::FOUND);
	let location = resp
		.headers()
		.get(reqwest::header::LOCATION)
		.and_then(|v| v.to_str().ok())
		.unwrap_or_default();
	assert!(location.starts_with(&format!("{}/authorize", oidc.uri())));

	gateway.shutdown().await;
}

async fn setup_gateway(
	backend_passthrough: bool,
) -> Option<(AgentGateway, MockServer, reqwest::Client)> {
	if !require_e2e() {
		return None;
	}

	let oidc = MockServer::start().await;
	let upstream = MockServer::start().await;

	let issuer = oidc.uri();
	Mock::given(method("GET"))
		.and(path("/.well-known/openid-configuration"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"issuer": issuer,
			"authorization_endpoint": format!("{}/authorize", oidc.uri()),
			"token_endpoint": format!("{}/token", oidc.uri()),
			"jwks_uri": format!("{}/jwks", oidc.uri())
		})))
		.mount(&oidc)
		.await;

	Mock::given(method("GET"))
		.and(path("/jwks"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"keys": [
				{
					"use": "sig",
					"kty": "EC",
					"kid": "XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI",
					"crv": "P-256",
					"alg": "ES256",
					"x": "XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo",
					"y": "g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE"
				}
			]
		})))
		.mount(&oidc)
		.await;

	Mock::given(method("GET"))
		.and(path_regex("/.*"))
		.respond_with(|req: &wiremock::Request| {
			let auth = req
				.headers
				.get(http::header::AUTHORIZATION)
				.and_then(|v| v.to_str().ok())
				.map(ToOwned::to_owned);
			ResponseTemplate::new(200).set_body_json(json!({
				"authorization": auth,
				"path": req.url.path().to_string()
			}))
		})
		.mount(&upstream)
		.await;

	let gw = AgentGateway::new(oauth2_config(
		&oidc.uri(),
		&upstream.address().to_string(),
		backend_passthrough,
	))
	.await
	.unwrap();

	let client = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.build()
		.unwrap();

	Some((gw, oidc, client))
}

pub(super) async fn start_auth(
	client: &reqwest::Client,
	gateway: &AgentGateway,
	path: &str,
) -> (Url, String, String) {
	let auth_start = client
		.get(gateway_url(gateway, path))
		.header(reqwest::header::ACCEPT, "text/html")
		.send()
		.await
		.unwrap();
	assert_eq!(auth_start.status(), StatusCode::FOUND);

	let auth_location = auth_start
		.headers()
		.get(reqwest::header::LOCATION)
		.unwrap()
		.to_str()
		.unwrap();
	let auth_url = Url::parse(auth_location).unwrap();
	let state = auth_url
		.query_pairs()
		.find(|(k, _)| k == "state")
		.map(|(_, v)| v.into_owned())
		.unwrap();
	let nonce_cookie = find_cookie_pair(&set_cookie_values(&auth_start), "__Host-ag-nonce")
		.expect("handshake cookie must be set");
	(auth_url, state, nonce_cookie)
}

pub(super) async fn complete_callback(
	client: &reqwest::Client,
	gateway: &AgentGateway,
	code: &str,
	state: &str,
	nonce_cookie: &str,
) -> reqwest::Response {
	client
		.get(callback_url(gateway, code, state))
		.header(reqwest::header::COOKIE, nonce_cookie)
		.send()
		.await
		.unwrap()
}

fn callback_url(gateway: &AgentGateway, code: &str, state: &str) -> Url {
	let mut callback_url = Url::parse(&gateway_url(gateway, "/_gateway/callback")).unwrap();
	callback_url
		.query_pairs_mut()
		.append_pair("code", code)
		.append_pair("state", state);
	callback_url
}

fn oauth2_config(issuer: &str, upstream: &str, backend_passthrough: bool) -> String {
	let backend_auth_block = if backend_passthrough {
		String::from(
			r#"        backendAuth:
          passthrough: {}
"#,
		)
	} else {
		String::new()
	};

	format!(
		r#"config: {{}}
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: oauth2
      policies:
        oauth2:
          issuer: "{issuer}"
          clientId: "agentgateway-local"
          clientSecret: "local-secret"
          redirectUri: "http://example.test/_gateway/callback"
          scopes:
          - openid
          - profile
          signOutPath: "/logout"
{backend_auth_block}      backends:
      - host: {upstream}
"#
	)
}
