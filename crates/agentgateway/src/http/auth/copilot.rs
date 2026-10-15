use std::path::PathBuf;

use ::http::HeaderValue;
use agent_llm::copilot::Provider;

use crate::http::Request;

const TOKEN_ENV_VARS: &[&str] = &["GH_COPILOT_TOKEN", "COPILOT_GITHUB_TOKEN"];
const DOMAIN: &str = "github.com";

pub(super) async fn insert_headers(req: &mut Request) -> anyhow::Result<()> {
	let token = load_token().await?;
	insert_headers_with_token(req, &token)
}

fn insert_headers_with_token(req: &mut Request, token: &str) -> anyhow::Result<()> {
	let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))?;
	auth.set_sensitive(true);

	req.headers_mut().insert(http::header::AUTHORIZATION, auth);
	Provider::set_required_headers(req.headers_mut());
	Ok(())
}

async fn load_token() -> anyhow::Result<String> {
	// Do not cache file-backed tokens here. GitHub/Copilot tooling may rotate them
	// independently, and direct-token auth should pick up those changes.
	for key in TOKEN_ENV_VARS {
		if let Ok(token) = std::env::var(key)
			&& let Some(token) = nonempty_trimmed(token.as_str())
		{
			return Ok(token.to_string());
		}
	}

	for path in copilot_config_paths() {
		if let Ok(contents) = tokio::fs::read_to_string(path).await
			&& let Some(token) = extract_json_oauth_token(&contents, DOMAIN)
		{
			return Ok(token);
		}
	}

	for path in gh_config_paths() {
		if let Ok(contents) = tokio::fs::read_to_string(path).await
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
		.or_else(platform_config_dir)
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[cfg(windows)]
fn platform_config_dir() -> Option<PathBuf> {
	std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(not(windows))]
fn platform_config_dir() -> Option<PathBuf> {
	None
}

fn nonempty_trimmed(value: &str) -> Option<&str> {
	let value = value.trim();
	(!value.is_empty()).then_some(value)
}

fn extract_json_oauth_token(contents: &str, domain: &str) -> Option<String> {
	let value: serde_json::Value = serde_json::from_str(contents).ok()?;
	value.as_object()?.iter().find_map(|(key, value)| {
		if key.starts_with(domain) {
			value["oauth_token"]
				.as_str()
				.and_then(nonempty_trimmed)
				.map(ToOwned::to_owned)
		} else {
			None
		}
	})
}

fn extract_yaml_oauth_token(contents: &str, domain: &str) -> Option<String> {
	let value: serde_norway::Value = serde_norway::from_str(contents).ok()?;
	value.as_mapping()?.iter().find_map(|(key, value)| {
		if key.as_str().is_some_and(|key| key.starts_with(domain)) {
			value["oauth_token"]
				.as_str()
				.and_then(nonempty_trimmed)
				.map(ToOwned::to_owned)
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
				"oauth_token": " copilot-token\n"
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
	fn supplied_token_sets_protocol_headers() {
		let mut req = ::http::Request::new(crate::http::Body::empty());
		insert_headers_with_token(&mut req, "configured-token").unwrap();

		for (name, value) in [
			("authorization", "Bearer configured-token"),
			("content-type", "application/json"),
			(
				"editor-version",
				concat!("agentgateway/", env!("CARGO_PKG_VERSION")),
			),
			("openai-intent", "conversation-agent"),
			("x-github-api-version", "2025-10-01"),
			("x-initiator", "agent"),
			("x-interaction-type", "conversation-agent"),
		] {
			assert_eq!(req.headers()[name], value);
		}
		assert!(req.headers()["authorization"].is_sensitive());
	}

	#[test]
	fn yaml_token_extraction() {
		let contents = r#"
github.com:
  oauth_token: " copilot-token\n"
  user: octocat
enterprise.example.com:
  oauth_token: wrong-token
"#;

		assert_eq!(
			extract_yaml_oauth_token(contents, "github.com").as_deref(),
			Some("copilot-token")
		);
	}
}
