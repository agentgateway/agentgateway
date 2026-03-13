use ::http::{HeaderMap, HeaderValue};

use super::{BETA_HEADER, OAUTH_BETA_FLAG, OAUTH_TOKEN_PREFIX, ensure_beta_flag};

// ── ensure_beta_flag unit tests ──────────────────────────────────────────────

#[test]
fn ensure_beta_flag_no_existing_header() {
	let mut headers = HeaderMap::new();
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	assert_eq!(headers.get(BETA_HEADER).unwrap(), OAUTH_BETA_FLAG);
}

#[test]
fn ensure_beta_flag_existing_other_value() {
	let mut headers = HeaderMap::new();
	headers.insert(BETA_HEADER, HeaderValue::from_static("other-flag"));
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	let val = headers.get(BETA_HEADER).unwrap().to_str().unwrap();
	assert_eq!(val, "other-flag,oauth-2025-04-20");
}

#[test]
fn ensure_beta_flag_already_present() {
	let mut headers = HeaderMap::new();
	headers.insert(BETA_HEADER, HeaderValue::from_static("oauth-2025-04-20"));
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	// Should be no-op – still exactly one entry with the original value.
	let values: Vec<&str> = headers
		.get_all(BETA_HEADER)
		.iter()
		.map(|v| v.to_str().unwrap())
		.collect();
	assert_eq!(values, vec!["oauth-2025-04-20"]);
}

#[test]
fn ensure_beta_flag_already_present_with_spaces() {
	let mut headers = HeaderMap::new();
	headers.insert(BETA_HEADER, HeaderValue::from_static(" oauth-2025-04-20 "));
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	// Trimmed match → no duplicate appended.
	let val = headers.get(BETA_HEADER).unwrap().to_str().unwrap();
	assert!(!val.contains("oauth-2025-04-20,oauth-2025-04-20"));
}

#[test]
fn ensure_beta_flag_multiple_headers() {
	let mut headers = HeaderMap::new();
	// HTTP allows multiple headers with the same name.
	headers.insert(BETA_HEADER, HeaderValue::from_static("flag-a"));
	headers.append(BETA_HEADER, HeaderValue::from_static("flag-b"));
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	let val = headers.get(BETA_HEADER).unwrap().to_str().unwrap();
	// All prior values merged, flag appended.
	assert!(val.contains("flag-a"));
	assert!(val.contains("flag-b"));
	assert!(val.contains(OAUTH_BETA_FLAG));
}

#[test]
fn ensure_beta_flag_csv_in_header() {
	let mut headers = HeaderMap::new();
	headers.insert(BETA_HEADER, HeaderValue::from_static("flag-a,flag-b"));
	ensure_beta_flag(&mut headers, OAUTH_BETA_FLAG).unwrap();
	let val = headers.get(BETA_HEADER).unwrap().to_str().unwrap();
	assert!(val.contains("flag-a"));
	assert!(val.contains("flag-b"));
	assert!(val.contains(OAUTH_BETA_FLAG));
}

// ── set_required_fields integration tests ───────────────────────────────────

fn make_bearer_request(token: &str) -> crate::http::Request {
	::http::Request::builder()
		.method("POST")
		.uri("https://api.anthropic.com/v1/messages")
		.header(::http::header::AUTHORIZATION, format!("Bearer {token}"))
		.body(crate::http::Body::empty())
		.unwrap()
}

#[test]
fn set_required_fields_oauth_token() {
	use crate::llm::AIProvider;

	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request(&format!("{OAUTH_TOKEN_PREFIX}01234567890abcdef"));

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must still be present (OAuth keeps Bearer).
	assert!(req.headers().contains_key(::http::header::AUTHORIZATION));
	// x-api-key must NOT be set.
	assert!(!req.headers().contains_key("x-api-key"));
	// oauth beta flag must be present.
	let beta = req.headers().get(BETA_HEADER).unwrap().to_str().unwrap();
	assert!(beta.split(',').any(|f| f.trim() == OAUTH_BETA_FLAG));
	// anthropic-version must be set.
	assert!(req.headers().contains_key("anthropic-version"));
}

#[test]
fn set_required_fields_api_key_token() {
	use crate::llm::AIProvider;

	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request("sk-ant-api01234567890abcdef");

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must be removed.
	assert!(!req.headers().contains_key(::http::header::AUTHORIZATION));
	// Token moved to x-api-key.
	assert!(req.headers().contains_key("x-api-key"));
	// oauth beta flag must NOT have been added.
	assert!(
		!req
			.headers()
			.get(BETA_HEADER)
			.map(|v: &HeaderValue| v.to_str().unwrap_or("").contains(OAUTH_BETA_FLAG))
			.unwrap_or(false)
	);
	// anthropic-version must be set.
	assert!(req.headers().contains_key("anthropic-version"));
}
