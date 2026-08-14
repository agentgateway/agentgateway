use crate::common::prelude::*;
use crate::tests::auth::{gateway_oidc_policy, oidc_backend_mock, setup_proxy_test_with_oidc};
use crate::tests::tls::route_with_prefix;

#[tokio::test]
async fn gateway_phase_oidc_bypasses_cors_preflight_requests() {
	let (mock, _token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let io = bind.serve_http(BIND_KEY);
	let res = send_request_headers(
		io,
		Method::OPTIONS,
		"http://lo/upstream",
		&[
			("origin", "https://frontend.example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::OPTIONS);
}

#[tokio::test]
async fn gateway_phase_cors_handles_preflight_before_route_selection() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
		}))
		.await;

	let res = send_request_headers(
		io,
		Method::OPTIONS,
		"http://lo/no-route-needed",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;

	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// the rate limit is exhausted, because CORS runs before authentication and rate limiting.
#[tokio::test]
async fn cors_preflight_bypasses_ratelimit() {
	let (_mock, mut bind, io) = basic_setup().await;

	// Attach CORS + rate limit (1 token, essentially immediately exhausted after first real request)
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "100s",
			}],
		}))
		.await;

	// First real request exhausts the single token
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	// Second real request should be rate limited
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);

	// A CORS preflight should still succeed (200) even though rate limit is exhausted
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that when a cross-origin request is rate limited (429), the response
/// still carries the CORS headers so browsers can read the error.
#[tokio::test]
async fn cors_headers_present_on_ratelimited_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "100s",
			}],
		}))
		.await;

	// Exhaust rate limit with a normal cross-origin GET
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");

	// Second cross-origin request is rate limited, but should still have CORS headers
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 429);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present even on rate-limited responses"
	);
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// API key authentication is required, because CORS runs before authentication
/// and authorization.
#[tokio::test]
async fn cors_preflight_bypasses_api_key_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
				}],
				"mode": "strict",
			},
		}))
		.await;

	// Request without credentials should be rejected
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);

	// CORS preflight should succeed without any credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// basic authentication is required, because CORS runs before authentication
/// and authorization.
#[tokio::test]
async fn cors_preflight_bypasses_basic_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"basicAuth": {
				"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00",
				"realm": "my-realm",
				"mode": "strict",
			},
		}))
		.await;

	// Request without credentials should be rejected
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);

	// CORS preflight should succeed without any credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// authorization rules would reject the request, because CORS runs before
/// authorization.
#[tokio::test]
async fn cors_preflight_bypasses_authorization() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
					"metadata": {"group": "eng"},
				}],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'admin'"],
			},
		}))
		.await;

	// Authenticated request should be rejected by authorization (403)
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-123")],
	)
	.await;
	assert_eq!(res.status(), 403);

	// CORS preflight should still succeed without credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that when authentication or authorization rejects a cross-origin
/// request, the response still carries CORS headers so browsers can read the
/// error body.
#[tokio::test]
async fn cors_headers_present_on_auth_rejected_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
					"metadata": {"group": "eng"},
				}],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'admin'"],
			},
		}))
		.await;

	// 401: missing credentials, CORS headers should still be present
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 401);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present on 401 responses"
	);

	// 403: valid key but fails authorization, CORS headers should still be present
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("authorization", "bearer sk-123"),
		],
	)
	.await;
	assert_eq!(res.status(), 403);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present on 403 responses"
	);
}
