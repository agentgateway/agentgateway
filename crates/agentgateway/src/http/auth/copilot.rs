use std::path::PathBuf;

use ::http::HeaderValue;

use crate::http::Request;

const TOKEN_ENV_VARS: &[&str] = &["GH_COPILOT_TOKEN", "COPILOT_GITHUB_TOKEN"];
const DOMAIN: &str = "github.com";

pub(super) async fn insert_headers(req: &mut Request) -> anyhow::Result<()> {
	let token = load_token().await?;
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

async fn load_token() -> anyhow::Result<String> {
	for key in TOKEN_ENV_VARS {
		if let Ok(token) = std::env::var(key)
			&& !token.trim().is_empty()
		{
			return Ok(token);
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
}
