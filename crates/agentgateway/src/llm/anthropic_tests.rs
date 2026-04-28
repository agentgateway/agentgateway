use super::OAUTH_TOKEN_PREFIX;
use crate::http::auth::{AppliedBackendAuthLocation, AuthorizationLocation};
use crate::llm::AIProvider;

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

fn make_bearer_request_with_explicit_auth(token: &str) -> crate::http::Request {
	let mut req = make_bearer_request(token);
	req.extensions_mut().insert(AppliedBackendAuthLocation {
		location: AuthorizationLocation::bearer_header(),
		explicit: true,
	});
	req
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

// ── Explicit backend auth location tests ────────────────────────────────────

#[test]
fn set_required_fields_explicit_authorization_preserved() {
	// When backend auth location is explicitly set to Authorization header,
	// Anthropic provider must NOT rewrite it to x-api-key (e.g. Databricks).
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request_with_explicit_auth("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9");

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must be preserved.
	assert!(
		req.headers().contains_key(::http::header::AUTHORIZATION),
		"explicit Authorization auth must be preserved"
	);
	let authz = req
		.headers()
		.get(::http::header::AUTHORIZATION)
		.unwrap()
		.to_str()
		.unwrap();
	assert!(
		authz.contains("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9"),
		"token value must be unchanged"
	);
	// x-api-key must NOT be set.
	assert!(
		!req.headers().contains_key("x-api-key"),
		"x-api-key must not be set when Authorization is explicit"
	);
	// anthropic-version must still be set.
	assert!(req.headers().contains_key("anthropic-version"));
}

#[test]
fn set_required_fields_default_auth_still_rewrites() {
	// When backend auth location was NOT explicitly set (defaulted),
	// non-OAuth tokens must still be rewritten to x-api-key.
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request("sk-ant-api01234567890abcdef");

	// Simulate default (non-explicit) auth location
	req.extensions_mut().insert(AppliedBackendAuthLocation {
		location: AuthorizationLocation::bearer_header(),
		explicit: false,
	});

	provider.set_required_fields(&mut req).unwrap();

	// Authorization header must be removed (default behavior).
	assert!(
		!req.headers().contains_key(::http::header::AUTHORIZATION),
		"default auth location must not prevent rewrite"
	);
	// Token moved to x-api-key.
	assert!(req.headers().contains_key("x-api-key"));
	// anthropic-version must be set.
	assert!(req.headers().contains_key("anthropic-version"));
}

#[test]
fn set_required_fields_explicit_x_api_key_preserved() {
	// If user explicitly configures a non-Authorization header (e.g. x-api-key directly),
	// Anthropic setup should still rewrite Authorization to x-api-key as normal,
	// since the explicit location is not Authorization.
	let provider = AIProvider::Anthropic(super::Provider { model: None });
	let mut req = make_bearer_request("sk-ant-api01234567890abcdef");

	req.extensions_mut().insert(AppliedBackendAuthLocation {
		location: AuthorizationLocation::Header {
			name: ::http::header::HeaderName::from_static("x-api-key"),
			prefix: None,
		},
		explicit: true,
	});

	provider.set_required_fields(&mut req).unwrap();

	// Even though the location was explicit, it's not Authorization, so normal rewrite applies.
	assert!(!req.headers().contains_key(::http::header::AUTHORIZATION));
	assert!(req.headers().contains_key("x-api-key"));
	assert!(req.headers().contains_key("anthropic-version"));
}
