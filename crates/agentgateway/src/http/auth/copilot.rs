use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use ::http::HeaderValue;

use crate::http::Request;

const TOKEN_ENV_VARS: &[&str] = &["GH_COPILOT_TOKEN", "COPILOT_GITHUB_TOKEN"];
const DOMAIN: &str = "github.com";
const TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const TOKEN_REFRESH_SKEW_SECS: u64 = 300;
const EXCHANGE_FAILURE_RETRY_SECS: u64 = 300;

#[derive(Clone, Debug)]
struct ApiToken {
	source_token: String,
	token: String,
	expires_at_unix: u64,
	refresh_at_unix: u64,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
	token: String,
	expires_at: u64,
	#[serde(default)]
	refresh_in: Option<u64>,
}

static API_TOKEN: OnceLock<Mutex<Option<ApiToken>>> = OnceLock::new();

pub(super) async fn insert_headers(req: &mut Request) -> anyhow::Result<()> {
	let token = load_api_token().await?;
	let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))?;
	auth.set_sensitive(true);

	req.headers_mut().insert(http::header::AUTHORIZATION, auth);
	req.headers_mut().insert(
		http::header::CONTENT_TYPE,
		HeaderValue::from_static("application/json"),
	);
	req.headers_mut().insert(
		"editor-version",
		HeaderValue::from_static(concat!("agentgateway/", env!("CARGO_PKG_VERSION"))),
	);
	req.headers_mut().insert(
		"x-github-api-version",
		HeaderValue::from_static("2025-10-01"),
	);
	req
		.headers_mut()
		.insert("x-initiator", HeaderValue::from_static("agent"));
	req.headers_mut().insert(
		"x-interaction-type",
		HeaderValue::from_static("conversation-agent"),
	);
	req.headers_mut().insert(
		"openai-intent",
		HeaderValue::from_static("conversation-agent"),
	);

	Ok(())
}

async fn load_api_token() -> anyhow::Result<String> {
	let source_token = load_source_token()?;
	let now = now_unix();
	let cache = API_TOKEN.get_or_init(|| Mutex::new(None));

	if let Some(token) = cached_api_token(cache, &source_token, now, false) {
		return Ok(token);
	}

	match fetch_api_token(&source_token, now).await {
		Ok(token) => {
			*cache.lock().expect("copilot token cache mutex poisoned") = Some(token.clone());
			Ok(token.token)
		},
		Err(err) => {
			if let Some(token) = cached_api_token(cache, &source_token, now, true) {
				return Ok(token);
			}

			// Preserve support for callers that provide an already-exchanged Copilot token.
			tracing::warn!(error = %err, "failed to exchange GitHub token for Copilot API token; using source token directly");
			let token = fallback_api_token(source_token, now);
			*cache.lock().expect("copilot token cache mutex poisoned") = Some(token.clone());
			Ok(token.token)
		},
	}
}

fn cached_api_token(
	cache: &Mutex<Option<ApiToken>>,
	source_token: &str,
	now: u64,
	allow_until_expiry: bool,
) -> Option<String> {
	let guard = cache.lock().expect("copilot token cache mutex poisoned");
	let cached = guard.as_ref()?;
	if cached.source_token != source_token {
		return None;
	}
	let usable_until = if allow_until_expiry {
		cached.expires_at_unix
	} else {
		cached.refresh_at_unix
	};
	(now < usable_until).then(|| cached.token.clone())
}

async fn fetch_api_token(source_token: &str, now: u64) -> anyhow::Result<ApiToken> {
	let response = reqwest::Client::new()
		.get(TOKEN_URL)
		.header(reqwest::header::ACCEPT, "application/json")
		.bearer_auth(source_token)
		.header(
			"editor-version",
			concat!("agentgateway/", env!("CARGO_PKG_VERSION")),
		)
		.header("x-github-api-version", "2025-10-01")
		.header("x-initiator", "agent")
		.header("x-interaction-type", "conversation-agent")
		.header("openai-intent", "conversation-agent")
		.send()
		.await?;

	if !response.status().is_success() {
		anyhow::bail!(
			"Copilot token exchange failed with status {}",
			response.status()
		);
	}

	let response: TokenResponse = serde_json::from_str(&response.text().await?)?;
	Ok(ApiToken {
		source_token: source_token.to_string(),
		token: response.token,
		expires_at_unix: response.expires_at,
		refresh_at_unix: refresh_at_unix(now, response.expires_at, response.refresh_in),
	})
}

fn fallback_api_token(source_token: String, now: u64) -> ApiToken {
	let retry_at = now.saturating_add(EXCHANGE_FAILURE_RETRY_SECS);
	ApiToken {
		source_token: source_token.clone(),
		token: source_token,
		expires_at_unix: retry_at,
		refresh_at_unix: retry_at,
	}
}

fn refresh_at_unix(now: u64, expires_at: u64, refresh_in: Option<u64>) -> u64 {
	let before_expiry = expires_at.saturating_sub(TOKEN_REFRESH_SKEW_SECS);
	refresh_in
		.map(|refresh_in| now.saturating_add(refresh_in).min(before_expiry))
		.unwrap_or(before_expiry)
}

fn now_unix() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

fn load_source_token() -> anyhow::Result<String> {
	for key in TOKEN_ENV_VARS {
		if let Ok(token) = std::env::var(key)
			&& !token.trim().is_empty()
		{
			return Ok(token);
		}
	}

	for path in copilot_config_paths() {
		if let Ok(contents) = std::fs::read_to_string(path)
			&& let Some(token) = extract_json_oauth_token(&contents, DOMAIN)
		{
			return Ok(token);
		}
	}

	for path in gh_config_paths() {
		if let Ok(contents) = std::fs::read_to_string(path)
			&& let Some(token) = extract_yaml_oauth_token(&contents, DOMAIN)
		{
			return Ok(token);
		}
	}

	anyhow::bail!(
		"Copilot token not found; set GH_COPILOT_TOKEN or authenticate with GitHub Copilot/GitHub CLI"
	)
}

fn copilot_config_paths() -> Vec<PathBuf> {
	config_dir()
		.map(|config| {
			let base = config.join("github-copilot");
			vec![base.join("hosts.json"), base.join("apps.json")]
		})
		.unwrap_or_default()
}

fn gh_config_paths() -> Vec<PathBuf> {
	config_dir()
		.map(|config| vec![config.join("gh").join("hosts.yml")])
		.unwrap_or_default()
}

fn config_dir() -> Option<PathBuf> {
	std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn extract_json_oauth_token(contents: &str, domain: &str) -> Option<String> {
	let value: serde_json::Value = serde_json::from_str(contents).ok()?;
	value.as_object()?.iter().find_map(|(key, value)| {
		if key.starts_with(domain) {
			value["oauth_token"].as_str().map(ToOwned::to_owned)
		} else {
			None
		}
	})
}

fn extract_yaml_oauth_token(contents: &str, domain: &str) -> Option<String> {
	let value: serde_yaml::Value = serde_yaml::from_str(contents).ok()?;
	value.as_mapping()?.iter().find_map(|(key, value)| {
		if key.as_str().is_some_and(|key| key.starts_with(domain)) {
			value["oauth_token"].as_str().map(ToOwned::to_owned)
		} else {
			None
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn json_token_extraction() {
		let contents = r#"{
			"github.com": {
				"oauth_token": "copilot-token"
			},
			"enterprise.example.com": {
				"oauth_token": "wrong-token"
			}
		}"#;

		assert_eq!(
			extract_json_oauth_token(contents, "github.com").as_deref(),
			Some("copilot-token")
		);
	}

	#[test]
	fn yaml_token_extraction() {
		let contents = r#"
github.com:
  oauth_token: copilot-token
  user: octocat
enterprise.example.com:
  oauth_token: wrong-token
"#;

		assert_eq!(
			extract_yaml_oauth_token(contents, "github.com").as_deref(),
			Some("copilot-token")
		);
	}

	#[test]
	fn refresh_at_prefers_refresh_in() {
		assert_eq!(refresh_at_unix(1_000, 5_000, Some(600)), 1_600);
	}

	#[test]
	fn refresh_at_uses_expiry_skew() {
		assert_eq!(refresh_at_unix(1_000, 5_000, None), 4_700);
		assert_eq!(refresh_at_unix(1_000, 5_000, Some(10_000)), 4_700);
	}

	#[test]
	fn cached_api_token_refresh_and_expiry_windows() {
		let cache = std::sync::Mutex::new(Some(ApiToken {
			source_token: "source".to_string(),
			token: "api".to_string(),
			expires_at_unix: 2_000,
			refresh_at_unix: 1_500,
		}));

		assert_eq!(
			cached_api_token(&cache, "source", 1_400, false).as_deref(),
			Some("api")
		);
		assert_eq!(cached_api_token(&cache, "source", 1_600, false), None);
		assert_eq!(
			cached_api_token(&cache, "source", 1_600, true).as_deref(),
			Some("api")
		);
		assert_eq!(cached_api_token(&cache, "other-source", 1_400, true), None);
	}

	#[test]
	fn fallback_api_token_uses_source_token_temporarily() {
		let token = fallback_api_token("source".to_string(), 1_000);

		assert_eq!(token.source_token, "source");
		assert_eq!(token.token, "source");
		assert_eq!(token.refresh_at_unix, 1_300);
		assert_eq!(token.expires_at_unix, 1_300);
	}
}
