use crate::llm::AIProvider;

use super::OAUTH_TOKEN_PREFIX;

// ── set_required_fields integration tests ───────────────────────────────────

fn make_bearer_request(token: &str) -> crate::http::Request {
	::http::Request::builder()
		.method("POST")
		.uri("https://api.anthropic.com/v1/messages")
		.header(::http::header::AUTHORIZATION, format!("Bearer {token}"))
		.body(crate::http::Body::empty())
		.unwrap()
}

fn make_bearer_request_with_api_key(token: &str, api_key: &str) -> crate::http::Request {
	::http::Request::builder()
		.method("POST")
		.uri("https://api.anthropic.com/v1/messages")
		.header(::http::header::AUTHORIZATION, format!("Bearer {token}"))
		.header("x-api-key", api_key)
		.body(crate::http::Body::empty())
		.unwrap()
}

#[test]
fn set_required_fields_oauth_token() {
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request(&format!("{OAUTH_TOKEN_PREFIX}01234567890abcdef"));

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must still be present (OAuth keeps Bearer).
	assert!(req.headers().contains_key(::http::header::AUTHORIZATION));
	// x-api-key must NOT be set.
	assert!(!req.headers().contains_key("x-api-key"));
	// anthropic-version must be set.
	assert!(req.headers().contains_key("anthropic-version"));
}

#[test]
fn set_required_fields_oauth_token_strips_api_key() {
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request_with_api_key(
		&format!("{OAUTH_TOKEN_PREFIX}01234567890abcdef"),
		"some-stale-key",
	);

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must still be present.
	assert!(req.headers().contains_key(::http::header::AUTHORIZATION));
	// x-api-key must be removed.
	assert!(!req.headers().contains_key("x-api-key"));
}

#[test]
fn set_required_fields_api_key_token() {
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request("sk-ant-api01234567890abcdef");

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must be removed.
	assert!(!req.headers().contains_key(::http::header::AUTHORIZATION));
	// Token moved to x-api-key.
	assert!(req.headers().contains_key("x-api-key"));
	// anthropic-version must be set.
	assert!(req.headers().contains_key("anthropic-version"));
}
