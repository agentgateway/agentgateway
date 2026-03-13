use agent_core::prelude::Strng;
use agent_core::strng;

use crate::llm::RouteType;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("anthropic");
}
pub const DEFAULT_HOST_STR: &str = "api.anthropic.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";
pub const BETA_HEADER: &str = "anthropic-beta";
pub const OAUTH_BETA_FLAG: &str = "oauth-2025-04-20";

pub fn path(route: RouteType) -> &'static str {
	match route {
		RouteType::AnthropicTokenCount => "/v1/messages/count_tokens",
		_ => "/v1/messages",
	}
}

/// Ensures `flag` is present in the `anthropic-beta` CSV header.
/// Uses `get_all` to handle multiple header instances correctly; merges
/// them all into one consolidated value with the flag appended if absent.
pub fn ensure_beta_flag(headers: &mut ::http::HeaderMap, flag: &str) -> anyhow::Result<()> {
	use ::http::HeaderValue;

	let already_present = headers.get_all(BETA_HEADER).iter().any(|v| {
		v.to_str()
			.is_ok_and(|s| s.split(',').any(|f| f.trim() == flag))
	});

	if !already_present {
		let existing: Vec<String> = headers
			.get_all(BETA_HEADER)
			.iter()
			.filter_map(|v| v.to_str().ok())
			.map(str::to_owned)
			.collect();

		let new_val = if existing.is_empty() {
			flag.to_owned()
		} else {
			format!("{},{flag}", existing.join(","))
		};

		headers.insert(BETA_HEADER, HeaderValue::from_str(&new_val)?);
	}
	Ok(())
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
