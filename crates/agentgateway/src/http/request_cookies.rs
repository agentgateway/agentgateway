use http::header;

use crate::http::Request;

/// Iterate all cookie name=value pairs from the request's `Cookie` headers.
/// Uses the `cookie` crate's RFC 6265 parser.
///
/// This helper is intentionally lossy: non-UTF-8 headers and malformed cookie
/// pairs are silently skipped.
pub(crate) fn iter_cookies(req: &Request) -> impl Iterator<Item = (String, String)> + '_ {
	req
		.headers()
		.get_all(header::COOKIE)
		.into_iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|header_value| {
			// Use cookie::Cookie::split_parse for RFC 6265 parsing.
			cookie::Cookie::split_parse(header_value.to_owned())
				.filter_map(|c: Result<cookie::Cookie<'_>, _>| c.ok())
				.map(|c: cookie::Cookie<'_>| (c.name().to_owned(), c.value().to_owned()))
				.collect::<Vec<_>>()
		})
}

/// Look up a single cookie by name from the request's `Cookie` headers.
///
/// When duplicate cookies exist, the last occurrence wins. Non-UTF-8 cookie
/// headers are silently skipped.
pub(crate) fn read_cookie(req: &Request, name: &str) -> Option<String> {
	let mut matched = None;
	for (cookie_name, cookie_value) in iter_cookies(req) {
		if cookie_name == name {
			matched = Some(cookie_value);
		}
	}
	matched
}

/// Remove all cookies whose name starts with `prefix` from the request.
/// Remaining cookies are re-joined into a single `Cookie` header.
/// If no cookies remain, the header is removed entirely.
/// Non-UTF-8 cookie headers are silently skipped.
pub(crate) fn strip_cookies_by_prefix(req: &mut Request, prefix: &str) {
	let preserved: Vec<String> = iter_cookies(req)
		.filter(|(name, _)| !name.starts_with(prefix))
		.map(|(name, value)| format!("{name}={value}"))
		.collect();

	req.headers_mut().remove(header::COOKIE);
	if !preserved.is_empty() {
		// All preserved values came from valid header data, so re-joining cannot produce
		// invalid header bytes.
		let hv = http::HeaderValue::from_str(&preserved.join("; "))
			.expect("re-joined cookie header from valid source data");
		req.headers_mut().insert(header::COOKIE, hv);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Body;

	fn make_request(cookies: &[&str]) -> Request {
		let mut builder = http::Request::builder().uri("http://example.com/");
		for cookie in cookies {
			builder = builder.header(header::COOKIE, *cookie);
		}
		builder.body(Body::empty()).unwrap()
	}

	// --- read_cookie ---

	#[test]
	fn read_cookie_no_cookies() {
		let req = make_request(&[]);
		assert_eq!(read_cookie(&req, "session"), None);
	}

	#[test]
	fn read_cookie_single_header() {
		let req = make_request(&["session=abc123; theme=dark"]);
		assert_eq!(read_cookie(&req, "session"), Some("abc123".into()));
		assert_eq!(read_cookie(&req, "theme"), Some("dark".into()));
		assert_eq!(read_cookie(&req, "missing"), None);
	}

	#[test]
	fn read_cookie_last_occurrence_wins_across_headers() {
		let req = make_request(&["session=first", "session=second"]);
		assert_eq!(read_cookie(&req, "session"), Some("second".into()));
	}

	#[test]
	fn read_cookie_last_occurrence_wins_within_header() {
		let req = make_request(&["session=first; session=second"]);
		assert_eq!(read_cookie(&req, "session"), Some("second".into()));
	}

	#[test]
	fn read_cookie_value_containing_equals() {
		let req = make_request(&["token=abc=def=ghi"]);
		assert_eq!(read_cookie(&req, "token"), Some("abc=def=ghi".into()));
	}

	// --- strip_cookies_by_prefix ---

	#[test]
	fn strip_cookies_removes_matching_prefix() {
		let mut req = make_request(&["agw_oidc_s=xxx; user=alice"]);
		strip_cookies_by_prefix(&mut req, "agw_oidc_");
		let cookie = req.headers().get(header::COOKIE).unwrap().to_str().unwrap();
		assert_eq!(cookie, "user=alice");
	}

	#[test]
	fn strip_cookies_removes_header_when_all_stripped() {
		let mut req = make_request(&["agw_oidc_s=xxx; agw_oidc_t=yyy"]);
		strip_cookies_by_prefix(&mut req, "agw_oidc_");
		assert!(req.headers().get(header::COOKIE).is_none());
	}

	#[test]
	fn strip_cookies_preserves_all_when_no_match() {
		let mut req = make_request(&["user=alice; theme=dark"]);
		strip_cookies_by_prefix(&mut req, "agw_oidc_");
		let cookie = req.headers().get(header::COOKIE).unwrap().to_str().unwrap();
		assert_eq!(cookie, "user=alice; theme=dark");
	}

	// --- iter_cookies ---

	#[test]
	fn iter_cookies_yields_all_pairs_across_headers() {
		let req = make_request(&["a=1; b=2", "c=3"]);
		let pairs: Vec<_> = iter_cookies(&req).collect();
		assert_eq!(
			pairs,
			vec![
				("a".into(), "1".into()),
				("b".into(), "2".into()),
				("c".into(), "3".into())
			]
		);
	}

	#[test]
	fn iter_cookies_skips_malformed_pairs() {
		let req = make_request(&["good=val; malformed; also_good=v2"]);
		let pairs: Vec<_> = iter_cookies(&req).collect();
		assert_eq!(
			pairs,
			vec![
				("good".into(), "val".into()),
				("also_good".into(), "v2".into())
			]
		);
	}

	// --- edge-case tests ---

	#[test]
	fn read_cookie_returns_none_for_non_utf8_cookie_header() {
		let mut req = make_request(&[]);
		req.headers_mut().insert(
			header::COOKIE,
			http::HeaderValue::from_bytes(b"session=\xff\xfe").unwrap(),
		);
		assert_eq!(read_cookie(&req, "session"), None);
	}

	#[test]
	fn read_cookie_handles_empty_cookie_header() {
		let req = make_request(&[""]);
		assert_eq!(read_cookie(&req, "session"), None);
	}

	#[test]
	fn strip_cookies_handles_trailing_semicolons() {
		let mut req = make_request(&["agw_oidc_s_a=1; session=abc; "]);
		strip_cookies_by_prefix(&mut req, "agw_oidc_");
		assert_eq!(req.headers().get(header::COOKIE).unwrap(), "session=abc");
	}

	#[test]
	fn strip_cookies_across_multiple_headers() {
		let mut req = make_request(&["agw_oidc_s_a=1; session=abc", "agw_oidc_t_b=2; theme=dark"]);
		strip_cookies_by_prefix(&mut req, "agw_oidc_");
		assert_eq!(
			req.headers().get(header::COOKIE).unwrap(),
			"session=abc; theme=dark"
		);
	}

	#[test]
	fn iter_cookies_skips_non_utf8_header() {
		let mut req = make_request(&["good=value"]);
		req.headers_mut().append(
			header::COOKIE,
			http::HeaderValue::from_bytes(b"bad=\xff\xfe").unwrap(),
		);
		req.headers_mut().append(
			header::COOKIE,
			http::HeaderValue::from_static("also_good=v2"),
		);
		let pairs: Vec<_> = iter_cookies(&req).collect();
		assert_eq!(
			pairs,
			vec![
				("good".into(), "value".into()),
				("also_good".into(), "v2".into())
			]
		);
	}
}
