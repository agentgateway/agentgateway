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

pub(super) fn is_localhost_request(req: &Request) -> bool {
	if !has_localhost_authority(req) {
		return false;
	}

	let mut origins = req.headers().get_all(header::ORIGIN).iter();
	let Some(origin) = origins.next() else {
		// Non-browser and same-origin requests may omit Origin.
		return true;
	};
	// Origin is a singleton header. Reject duplicates rather than validating only
	// the first value and accidentally ignoring a conflicting one.
	if origins.next().is_some() {
		return false;
	}
	origin.to_str().ok().is_some_and(is_localhost_origin)
}

fn has_localhost_authority(req: &Request) -> bool {
	// Inbound HTTP/1 Host is normalized into the URI authority before routing;
	// HTTP/2 already carries it as :authority. Keep that normalized value as the
	// sole source of truth.
	req.uri().host().is_some_and(is_localhost_host)
}

fn is_localhost_origin(origin: &str) -> bool {
	// `null` is a real browser Origin for opaque origins, not an absent header.
	// It is deliberately rejected by this localhost-only policy.
	let Ok(origin) = url::Url::parse(origin.trim()) else {
		return false;
	};
	matches!(origin.scheme(), "http" | "https")
		&& origin.username().is_empty()
		&& origin.password().is_none()
		&& origin.path() == "/"
		&& origin.query().is_none()
		&& origin.fragment().is_none()
		&& origin.host_str().is_some_and(is_localhost_host)
}

fn is_localhost_host(host: &str) -> bool {
	let host = host.trim_matches(['[', ']']);
	host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

#[cfg(test)]
mod tests {
	use super::*;

	fn req(authority: Option<&str>, origin: Option<&str>) -> Request {
		let uri = authority
			.map(|authority| format!("http://{authority}/mcp"))
			.unwrap_or_else(|| "/mcp".to_string());
		let mut b = ::http::Request::builder().method("POST").uri(uri);
		if let Some(origin) = origin {
			b = b.header(header::ORIGIN, origin);
		}
		b.body(crate::http::Body::empty()).unwrap()
	}

	#[test]
	fn accepts_localhost_hosts() {
		for host in [
			"localhost",
			"localhost:8080",
			"127.0.0.1",
			"127.0.0.1:9",
			"[::1]",
			"[::1]:8080",
		] {
			assert!(is_localhost_request(&req(Some(host), None)), "host {host}");
		}
	}

	#[test]
	fn rejects_non_localhost_host() {
		assert!(!is_localhost_request(&req(Some("evil.com"), None)));
		assert!(!is_localhost_request(&req(Some("evil.com:443"), None)));
		assert!(!is_localhost_request(&req(None, None)));
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
			Some("127.0.0.1"),
			Some("http://127.0.0.1")
		)));
	}

	#[test]
	fn rejects_null_and_malformed_origins() {
		for origin in [
			"null",
			"http://[::1].evil.example",
			"http://[::1]garbage",
			"http://localhost/path",
			"ftp://localhost",
		] {
			assert!(
				!is_localhost_request(&req(Some("localhost"), Some(origin))),
				"origin {origin}"
			);
		}
	}

	#[test]
	fn rejects_duplicate_origins() {
		let request = ::http::Request::builder()
			.method("POST")
			.uri("http://localhost/mcp")
			.header(header::ORIGIN, "http://localhost")
			.header(header::ORIGIN, "http://evil.example")
			.body(crate::http::Body::empty())
			.unwrap();
		assert!(!is_localhost_request(&request));
	}

	#[test]
	fn reject_non_localhost_builds_403() {
		let resp = reject_non_localhost(&req(Some("evil.com"), None)).expect("rejected");
		assert_eq!(resp.status(), StatusCode::FORBIDDEN);
		assert!(reject_non_localhost(&req(Some("localhost"), None)).is_none());
	}
}
