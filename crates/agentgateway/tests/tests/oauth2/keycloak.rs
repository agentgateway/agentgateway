use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use http::StatusCode;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use tracing::warn;
use url::Url;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{complete_callback, start_auth};
use crate::common::gateway::AgentGateway;
use crate::common::oauth2::{
	cookie_header_from_response, gateway_url, require_e2e, session_cookie_header, set_cookie_values,
};

static KEYCLOAK_TEST_MUTEX: Lazy<tokio::sync::Mutex<()>> =
	Lazy::new(|| tokio::sync::Mutex::new(()));
static KEYCLOAK_LOGIN_FORM_TAG_RE: Lazy<Regex> = Lazy::new(|| {
	Regex::new(r#"(?is)<form\b(?P<attrs>[^>]*)>"#)
		.expect("keycloak login form-tag regex must compile")
});
static KEYCLOAK_LOGIN_FORM_ATTR_RE: Lazy<Regex> = Lazy::new(|| {
	Regex::new(
		r#"(?is)\b(?P<name>[a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*(?:"(?P<dq>[^"]*)"|'(?P<sq>[^']*)'|(?P<bare>[^\s"'=<>`]+))"#,
	)
	.expect("keycloak login form-attribute regex must compile")
});

#[tokio::test]
async fn oauth2_keycloak_auth_code_flow_forwards_access_token() {
	let _serial = KEYCLOAK_TEST_MUTEX.lock().await;
	let Some(ctx) = setup_context().await else {
		return;
	};

	let (auth_url, original_state, nonce_cookie) =
		start_auth(&ctx.browser, &ctx.gateway, "/private/data?from=keycloak").await;
	let callback_location = ctx
		.keycloak
		.perform_browser_login_and_get_callback(
			auth_url,
			&ctx.keycloak.test_username,
			&ctx.keycloak.test_password,
		)
		.await
		.expect("keycloak browser login should complete with callback redirect");

	let callback_url = Url::parse(&callback_location).unwrap();
	let callback_code = callback_url
		.query_pairs()
		.find(|(k, _)| k == "code")
		.map(|(_, v)| v.into_owned())
		.expect("callback code must be present");
	let callback_state = callback_url
		.query_pairs()
		.find(|(k, _)| k == "state")
		.map(|(_, v)| v.into_owned())
		.expect("callback state must be present");
	assert_eq!(callback_state, original_state);

	let callback = complete_callback(
		&ctx.browser,
		&ctx.gateway,
		&callback_code,
		&callback_state,
		&nonce_cookie,
	)
	.await;
	assert_eq!(callback.status(), StatusCode::FOUND);
	assert_eq!(
		callback
			.headers()
			.get(reqwest::header::LOCATION)
			.unwrap()
			.to_str()
			.unwrap(),
		"/private/data?from=keycloak"
	);

	let session_cookie = session_cookie_header(&set_cookie_values(&callback))
		.expect("session cookie must be set after callback");

	let protected = ctx
		.browser
		.get(gateway_url(&ctx.gateway, "/private/data?from=keycloak"))
		.header(reqwest::header::COOKIE, session_cookie)
		.send()
		.await
		.unwrap();
	assert_eq!(protected.status(), StatusCode::OK);
	let upstream_body: serde_json::Value = protected.json().await.unwrap();
	let auth = upstream_body
		.get("authorization")
		.and_then(|v| v.as_str())
		.unwrap_or_default();
	assert!(
		auth.starts_with("Bearer "),
		"expected bearer token to be forwarded, got: {auth}"
	);

	ctx.shutdown().await;
}

#[tokio::test]
async fn oauth2_keycloak_invalid_credentials_do_not_complete_login() {
	let _serial = KEYCLOAK_TEST_MUTEX.lock().await;
	let Some(ctx) = setup_context().await else {
		return;
	};

	let (auth_url, _, _) =
		start_auth(&ctx.browser, &ctx.gateway, "/private/data?from=keycloak").await;
	let bad = ctx
		.keycloak
		.perform_browser_login_and_get_callback(
			auth_url,
			&ctx.keycloak.test_username,
			"definitely-wrong-password",
		)
		.await;
	assert!(bad.is_err(), "login should fail with invalid credentials");

	ctx.shutdown().await;
}

#[tokio::test]
async fn oauth2_keycloak_tampered_state_is_rejected() {
	let _serial = KEYCLOAK_TEST_MUTEX.lock().await;
	let Some(ctx) = setup_context().await else {
		return;
	};

	let (auth_url, original_state, nonce_cookie) =
		start_auth(&ctx.browser, &ctx.gateway, "/private/data?from=keycloak").await;
	let callback_location = ctx
		.keycloak
		.perform_browser_login_and_get_callback(
			auth_url,
			&ctx.keycloak.test_username,
			&ctx.keycloak.test_password,
		)
		.await
		.expect("keycloak browser login should complete with callback redirect");

	let callback_url = Url::parse(&callback_location).unwrap();
	let callback_code = callback_url
		.query_pairs()
		.find(|(k, _)| k == "code")
		.map(|(_, v)| v.into_owned())
		.expect("callback code must be present");
	let callback_state = callback_url
		.query_pairs()
		.find(|(k, _)| k == "state")
		.map(|(_, v)| v.into_owned())
		.expect("callback state must be present");
	assert_eq!(callback_state, original_state);

	let tampered_state = format!("{callback_state}tampered");
	let callback = complete_callback(
		&ctx.browser,
		&ctx.gateway,
		&callback_code,
		&tampered_state,
		&nonce_cookie,
	)
	.await;
	assert_eq!(callback.status(), StatusCode::BAD_REQUEST);

	ctx.shutdown().await;
}

struct RealKeycloakContext {
	keycloak: KeycloakEnv,
	admin_token: String,
	client_id: String,
	gateway: AgentGateway,
	browser: reqwest::Client,
	_upstream: MockServer,
}

impl RealKeycloakContext {
	async fn shutdown(self) {
		let _ = self
			.keycloak
			.delete_oidc_client_if_exists(&self.admin_token, &self.client_id)
			.await;
		self.gateway.shutdown().await;
	}
}

async fn setup_context() -> Option<RealKeycloakContext> {
	if !require_e2e() {
		return None;
	}

	let keycloak = KeycloakEnv::from_env();
	if !keycloak.is_available().await {
		warn!(
			"Keycloak not available at {}, skipping OAuth2 real-idp tests",
			keycloak.base_url
		);
		return None;
	}

	let Some(admin_token) = keycloak.admin_token().await else {
		warn!("failed to get keycloak admin token, skipping test");
		return None;
	};

	let client_id = unique_client_id();
	let client_secret = "ag-e2e-secret";
	let redirect_uri = "http://example.test/_gateway/callback";
	if let Err(err) = keycloak
		.ensure_oidc_client(&admin_token, &client_id, client_secret, redirect_uri)
		.await
	{
		warn!("failed to provision keycloak oidc client: {err}, skipping test");
		return None;
	}

	let upstream = MockServer::start().await;
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

	let gateway = AgentGateway::new(oauth2_config(
		&keycloak.issuer_url(),
		&client_id,
		client_secret,
		&upstream.address().to_string(),
	))
	.await
	.unwrap();

	let browser = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.build()
		.unwrap();

	Some(RealKeycloakContext {
		keycloak,
		admin_token,
		client_id,
		gateway,
		browser,
		_upstream: upstream,
	})
}

struct KeycloakEnv {
	base_url: String,
	realm: String,
	admin_username: String,
	admin_password: String,
	test_username: String,
	test_password: String,
	client: reqwest::Client,
}

impl KeycloakEnv {
	fn from_env() -> Self {
		Self {
			base_url: std::env::var("KEYCLOAK_BASE_URL")
				.unwrap_or_else(|_| "http://localhost:7080".to_string()),
			realm: std::env::var("KEYCLOAK_REALM").unwrap_or_else(|_| "mcp".to_string()),
			admin_username: std::env::var("KEYCLOAK_ADMIN_USER").unwrap_or_else(|_| "admin".to_string()),
			admin_password: std::env::var("KEYCLOAK_ADMIN_PASSWORD")
				.unwrap_or_else(|_| "admin".to_string()),
			test_username: std::env::var("KEYCLOAK_TEST_USER").unwrap_or_else(|_| "testuser".to_string()),
			test_password: std::env::var("KEYCLOAK_TEST_PASSWORD")
				.unwrap_or_else(|_| "testpass".to_string()),
			client: reqwest::Client::builder()
				.redirect(reqwest::redirect::Policy::none())
				.build()
				.unwrap(),
		}
	}

	fn issuer_url(&self) -> String {
		format!("{}/realms/{}", self.base_url, self.realm)
	}

	async fn is_available(&self) -> bool {
		let well_known = format!(
			"{}/realms/{}/.well-known/openid-configuration",
			self.base_url, self.realm
		);
		self
			.client
			.get(well_known)
			.send()
			.await
			.map(|r| r.status().is_success())
			.unwrap_or(false)
	}

	async fn admin_token(&self) -> Option<String> {
		let url = format!(
			"{}/realms/master/protocol/openid-connect/token",
			self.base_url
		);
		let resp = self
			.client
			.post(url)
			.form(&[
				("grant_type", "password"),
				("client_id", "admin-cli"),
				("username", self.admin_username.as_str()),
				("password", self.admin_password.as_str()),
			])
			.send()
			.await
			.ok()?;
		if !resp.status().is_success() {
			return None;
		}
		resp
			.json::<serde_json::Value>()
			.await
			.ok()?
			.get("access_token")
			.and_then(|v| v.as_str())
			.map(ToOwned::to_owned)
	}

	async fn ensure_oidc_client(
		&self,
		admin_token: &str,
		client_id: &str,
		client_secret: &str,
		redirect_uri: &str,
	) -> anyhow::Result<()> {
		let admin_clients_url = format!("{}/admin/realms/{}/clients", self.base_url, self.realm);

		let existing = self
			.client
			.get(format!("{admin_clients_url}?clientId={client_id}"))
			.bearer_auth(admin_token)
			.send()
			.await?;
		if existing.status().is_success() {
			let entries = existing.json::<Vec<serde_json::Value>>().await?;
			if !entries.is_empty() {
				return Ok(());
			}
		}

		let create_payload = json!({
			"clientId": client_id,
			"enabled": true,
			"protocol": "openid-connect",
			"publicClient": false,
			"secret": client_secret,
			"standardFlowEnabled": true,
			"directAccessGrantsEnabled": true,
			"serviceAccountsEnabled": false,
			"redirectUris": [redirect_uri],
			"webOrigins": ["*"]
		});

		let created = self
			.client
			.post(admin_clients_url)
			.bearer_auth(admin_token)
			.json(&create_payload)
			.send()
			.await?;

		match created.status() {
			StatusCode::CREATED | StatusCode::CONFLICT => Ok(()),
			status => {
				let body = created.text().await.unwrap_or_default();
				anyhow::bail!("failed to create keycloak client: status={status}, body={body}")
			},
		}
	}

	async fn delete_oidc_client_if_exists(
		&self,
		admin_token: &str,
		client_id: &str,
	) -> anyhow::Result<()> {
		let admin_clients_url = format!("{}/admin/realms/{}/clients", self.base_url, self.realm);
		let existing = self
			.client
			.get(format!("{admin_clients_url}?clientId={client_id}"))
			.bearer_auth(admin_token)
			.send()
			.await?;
		if !existing.status().is_success() {
			return Ok(());
		}
		let entries = existing.json::<Vec<serde_json::Value>>().await?;
		for entry in entries {
			if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
				let _ = self
					.client
					.delete(format!("{admin_clients_url}/{id}"))
					.bearer_auth(admin_token)
					.send()
					.await;
			}
		}
		Ok(())
	}

	async fn perform_browser_login_and_get_callback(
		&self,
		authorize_url: Url,
		username: &str,
		password: &str,
	) -> anyhow::Result<String> {
		let browser = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.build()?;

		let login_page = browser.get(authorize_url.clone()).send().await?;
		if !login_page.status().is_success() {
			anyhow::bail!("authorize endpoint failed: {}", login_page.status());
		}
		let login_cookies = cookie_header_from_response(&login_page);
		let html = login_page.text().await?;
		let form_action = extract_keycloak_login_action(&html)
			.ok_or_else(|| anyhow::anyhow!("unable to find keycloak login form action"))?;
		let action_url = normalize_action_url(&authorize_url, &form_action)?;

		let login_resp = browser
			.post(action_url)
			.header(reqwest::header::COOKIE, login_cookies)
			.form(&[("username", username), ("password", password)])
			.send()
			.await?;

		if !matches!(
			login_resp.status(),
			StatusCode::FOUND | StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT
		) {
			let status = login_resp.status();
			let body = login_resp.text().await.unwrap_or_default();
			anyhow::bail!(
				"unexpected keycloak login response: status={}, body-prefix={}",
				status,
				truncate(&body, 240)
			);
		}

		let location = login_resp
			.headers()
			.get(reqwest::header::LOCATION)
			.and_then(|v| v.to_str().ok())
			.ok_or_else(|| anyhow::anyhow!("missing keycloak login redirect location"))?
			.to_string();
		Ok(location)
	}
}

fn extract_keycloak_login_action(html: &str) -> Option<String> {
	let mut all_actions = Vec::new();
	let mut keycloak_actions = Vec::new();

	for form in KEYCLOAK_LOGIN_FORM_TAG_RE.captures_iter(html) {
		let Some(attrs) = form.name("attrs") else {
			continue;
		};
		let attrs = attrs.as_str();
		let attrs = parse_form_attributes(attrs);
		let Some(action) = attrs.get("action") else {
			continue;
		};
		let action = decode_html_attr(action);
		if attrs
			.get("id")
			.is_some_and(|v| v.eq_ignore_ascii_case("kc-form-login"))
			|| attrs
				.get("name")
				.is_some_and(|v| v.eq_ignore_ascii_case("kc-form-login"))
		{
			return Some(action);
		}

		if action.contains("/login-actions/authenticate") {
			keycloak_actions.push(action.clone());
		}
		all_actions.push(action);
	}

	if keycloak_actions.len() == 1 {
		return keycloak_actions.into_iter().next();
	}
	if all_actions.len() == 1 {
		return all_actions.into_iter().next();
	}
	None
}

fn parse_form_attributes(attrs: &str) -> HashMap<String, String> {
	let mut parsed = HashMap::new();
	for cap in KEYCLOAK_LOGIN_FORM_ATTR_RE.captures_iter(attrs) {
		let Some(name) = cap.name("name") else {
			continue;
		};
		let value = cap
			.name("dq")
			.or_else(|| cap.name("sq"))
			.or_else(|| cap.name("bare"))
			.map(|v| v.as_str())
			.unwrap_or_default()
			.to_string();
		parsed.insert(name.as_str().to_ascii_lowercase(), value);
	}
	parsed
}

fn decode_html_attr(value: &str) -> String {
	value
		.replace("&amp;", "&")
		.replace("&quot;", "\"")
		.replace("&apos;", "'")
		.replace("&#39;", "'")
}

fn normalize_action_url(base: &Url, action: &str) -> anyhow::Result<Url> {
	if let Ok(url) = Url::parse(action) {
		return Ok(url);
	}
	base.join(action).map_err(Into::into)
}

fn truncate(s: &str, max: usize) -> String {
	if s.len() <= max {
		s.to_string()
	} else {
		let mut end = max;
		while end > 0 && !s.is_char_boundary(end) {
			end -= 1;
		}
		format!("{}...", &s[..end])
	}
}

fn unique_client_id() -> String {
	let ts = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_micros())
		.unwrap_or_default();
	format!("ag-e2e-{ts}")
}

fn oauth2_config(issuer: &str, client_id: &str, client_secret: &str, upstream: &str) -> String {
	format!(
		r#"config: {{}}
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: oauth2-keycloak
      policies:
        oauth2:
          issuer: "{issuer}"
          clientId: "{client_id}"
          clientSecret: "{client_secret}"
          redirectUri: "http://example.test/_gateway/callback"
          scopes:
          - openid
          - profile
        backendAuth:
          passthrough: {{}}
      backends:
      - host: {upstream}
"#
	)
}

#[test]
fn extract_keycloak_login_action_prefers_kc_form_login_id() {
	let html = r#"
		<html><body>
			<form action="/not-this"></form>
			<form id="kc-form-login" action="/real-login?x=1&amp;y=2"></form>
		</body></html>
	"#;
	assert_eq!(
		extract_keycloak_login_action(html).as_deref(),
		Some("/real-login?x=1&y=2")
	);
}

#[test]
fn extract_keycloak_login_action_handles_single_quotes_and_reordered_attributes() {
	let html = r#"
		<form method='post' action='/real-login' class='kc-form' id='kc-form-login'></form>
	"#;
	assert_eq!(
		extract_keycloak_login_action(html).as_deref(),
		Some("/real-login")
	);
}

#[test]
fn extract_keycloak_login_action_falls_back_to_keycloak_authenticate_pattern() {
	let html = r#"
		<form action="/something-else"></form>
		<form action="/realms/mcp/login-actions/authenticate?session_code=abc"></form>
	"#;
	assert_eq!(
		extract_keycloak_login_action(html).as_deref(),
		Some("/realms/mcp/login-actions/authenticate?session_code=abc")
	);
}

#[test]
fn extract_keycloak_login_action_returns_none_when_ambiguous() {
	let html = r#"
		<form action="/a"></form>
		<form action="/b"></form>
	"#;
	assert!(extract_keycloak_login_action(html).is_none());
}
