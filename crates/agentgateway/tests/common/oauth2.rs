use std::sync::OnceLock;

use tracing::warn;

use crate::common::gateway::AgentGateway;

pub(crate) const SESSION_COOKIE_NAME: &str = "__Host-ag-session";
static E2E_ENABLED: OnceLock<bool> = OnceLock::new();

pub(crate) fn gateway_url(gateway: &AgentGateway, path: &str) -> String {
	format!("http://127.0.0.1:{}{path}", gateway.port())
}

pub(crate) fn set_cookie_values(resp: &reqwest::Response) -> Vec<String> {
	resp
		.headers()
		.get_all(reqwest::header::SET_COOKIE)
		.iter()
		.filter_map(|v| v.to_str().ok().map(ToOwned::to_owned))
		.collect()
}

pub(crate) fn cookie_header_from_response(resp: &reqwest::Response) -> String {
	set_cookie_values(resp)
		.into_iter()
		.filter_map(|v| v.split(';').next().map(str::trim).map(ToOwned::to_owned))
		.collect::<Vec<_>>()
		.join("; ")
}

pub(crate) fn find_cookie_pair(
	set_cookie_headers: &[String],
	cookie_name_prefix: &str,
) -> Option<String> {
	for header in set_cookie_headers {
		let pair = header.split(';').next()?.trim();
		if let Some((name, _)) = pair.split_once('=')
			&& (name == cookie_name_prefix || name.starts_with(&format!("{cookie_name_prefix}.")))
		{
			return Some(pair.to_string());
		}
	}
	None
}

pub(crate) fn session_cookie_header(set_cookie_headers: &[String]) -> Option<String> {
	let mut pairs = Vec::new();
	for header in set_cookie_headers {
		let pair = match header.split(';').next() {
			Some(v) => v.trim(),
			None => continue,
		};
		let Some((name, _)) = pair.split_once('=') else {
			continue;
		};
		if name == SESSION_COOKIE_NAME || name.starts_with(&format!("{SESSION_COOKIE_NAME}.")) {
			pairs.push(pair.to_string());
		}
	}
	if pairs.is_empty() {
		None
	} else {
		Some(pairs.join("; "))
	}
}

pub(crate) fn require_e2e() -> bool {
	*E2E_ENABLED.get_or_init(|| {
		agent_core::telemetry::testing::setup_test_logging();
		let found = std::env::var("AGENTGATEWAY_E2E").is_ok();
		if !found {
			warn!("environment variable AGENTGATEWAY_E2E not set, skipping test");
		}
		found
	})
}
