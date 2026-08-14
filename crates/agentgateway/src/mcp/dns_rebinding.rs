use ::http::{StatusCode, header};

use crate::http::{Request, Response};

/// Opt-in DNS rebinding protection for MCP servers (see #1855).
///
/// The MCP spec requires localhost servers to reject non-localhost `Host` /
/// `Origin` headers. agentgateway is typically not a browser-facing localhost
/// server, so this is off by default.
pub fn reject_non_localhost(req: &Request) -> Option<Response> {
	if is_localhost_request(req) {
		return None;
	}
	let response = ::http::Response::builder()
		.status(StatusCode::FORBIDDEN)
		.header(header::CONTENT_TYPE, "text/plain")
		.body(crate::http::Body::from(
			"MCP DNS rebinding protection: Host/Origin must be localhost, 127.0.0.1, or [::1]",
		))
		.expect("valid response");
	Some(response)
}

fn is_localhost_request(req: &Request) -> bool {
	let origin = req
		.headers()
		.get(header::ORIGIN)
		.and_then(|v| v.to_str().ok())
		.filter(|o| *o != "null");
	if let Some(origin) = origin
		&& !is_localhost_origin(origin)
	{
		return false;
	}

	if let Some(host) = request_host(req) {
		return is_localhost_host(host);
	}

	// No Host: accept only when Origin already proved localhost.
	origin.is_some_and(is_localhost_origin)
}

fn request_host(req: &Request) -> Option<&str> {
	req
		.headers()
		.get(header::HOST)
		.and_then(|v| v.to_str().ok())
		.or_else(|| req.uri().host())
}

fn is_localhost_origin(origin: &str) -> bool {
	let Some(rest) = origin
		.strip_prefix("http://")
		.or_else(|| origin.strip_prefix("https://"))
	else {
		return false;
	};
	let host = rest.split('/').next().unwrap_or(rest);
	is_localhost_host(host)
}

fn is_localhost_host(host: &str) -> bool {
	let host = strip_port(host.trim());
	let host = host
		.strip_prefix('[')
		.and_then(|h| h.strip_suffix(']'))
		.unwrap_or(host);
	host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Strip a trailing `:port`, keeping IPv6 bracket form (`[::1]:8080` → `[::1]`).
fn strip_port(host: &str) -> &str {
	if let Some(rest) = host.strip_prefix('[')
		&& let Some(end) = rest.find(']')
	{
		// `end` is relative to `rest`; include the leading '[' and trailing ']'.
		return &host[..=end + 1];
	}
	if let Some((name, port)) = host.rsplit_once(':')
		&& !port.is_empty()
		&& port.bytes().all(|b| b.is_ascii_digit())
	{
		return name;
	}
	host
}

#[cfg(test)]
mod tests {
	use super::*;

	fn req(host: Option<&str>, origin: Option<&str>) -> Request {
		let mut b = ::http::Request::builder().method("POST").uri("http://127.0.0.1:8080/mcp");
		if let Some(host) = host {
			b = b.header(header::HOST, host);
		}
		if let Some(origin) = origin {
			b = b.header(header::ORIGIN, origin);
		}
		b.body(crate::http::Body::empty()).unwrap()
	}

	#[test]
	fn accepts_localhost_hosts() {
		for host in ["localhost", "localhost:8080", "127.0.0.1", "127.0.0.1:9", "[::1]", "[::1]:8080"]
		{
			assert!(
				is_localhost_request(&req(Some(host), None)),
				"host {host}"
			);
		}
	}

	#[test]
	fn rejects_non_localhost_host() {
		assert!(!is_localhost_request(&req(Some("evil.com"), None)));
		assert!(!is_localhost_request(&req(Some("evil.com:443"), None)));
	}

	#[test]
	fn rejects_non_localhost_origin_even_with_localhost_host() {
		assert!(!is_localhost_request(&req(
			Some("127.0.0.1:8080"),
			Some("http://evil.com")
		)));
	}

	#[test]
	fn accepts_localhost_origin() {
		assert!(is_localhost_request(&req(
			Some("127.0.0.1:8080"),
			Some("http://localhost:8080")
		)));
		assert!(is_localhost_request(&req(
			None,
			Some("http://127.0.0.1")
		)));
	}

	#[test]
	fn reject_non_localhost_builds_403() {
		let resp = reject_non_localhost(&req(Some("evil.com"), None)).expect("rejected");
		assert_eq!(resp.status(), StatusCode::FORBIDDEN);
		assert!(reject_non_localhost(&req(Some("localhost"), None)).is_none());
	}
}
