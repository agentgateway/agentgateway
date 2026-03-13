use cookie::{Cookie, SameSite};
use http::HeaderValue;
use tracing::debug;

pub(super) const MAX_COOKIE_SIZE: usize = 3800;

#[derive(Debug, thiserror::Error)]
pub(super) enum SessionCookieError {
	#[error("encoded session exceeds cookie size budget")]
	TooLarge,
}

pub(super) fn build_clear_cookie(name: String, secure: bool) -> Cookie<'static> {
	Cookie::build((name, ""))
		.path("/")
		.secure(secure)
		.http_only(true)
		.max_age(cookie::time::Duration::seconds(0))
		.build()
}

pub(super) fn build_session_cookie(
	name: String,
	value: String,
	secure: bool,
	cookie_max_age: cookie::time::Duration,
) -> Cookie<'static> {
	Cookie::build((name, value))
		.path("/")
		.secure(secure)
		.http_only(true)
		.same_site(SameSite::Lax)
		.max_age(cookie_max_age)
		.build()
}

pub(super) fn encode_set_cookie_header(
	cookie: &Cookie<'_>,
) -> Result<HeaderValue, http::header::InvalidHeaderValue> {
	HeaderValue::from_str(&cookie.to_string())
}

pub(super) fn append_set_cookie_header(headers: &mut crate::http::HeaderMap, cookie: &Cookie<'_>) {
	if let Ok(value) = encode_set_cookie_header(cookie) {
		headers.append(http::header::SET_COOKIE, value);
	}
}

pub(super) fn for_each_request_cookie(headers: &http::HeaderMap, mut f: impl FnMut(Cookie<'_>)) {
	for cookies in headers.get_all(http::header::COOKIE) {
		let cookies = match cookies.to_str() {
			Ok(value) => value,
			Err(err) => {
				debug!("ignoring non-utf8 cookie header: {err}");
				continue;
			},
		};
		for cookie in Cookie::split_parse(cookies) {
			match cookie {
				Ok(cookie) => f(cookie),
				Err(err) => debug!("ignoring malformed cookie: {err}"),
			}
		}
	}
}

pub(super) fn clear_session_cookies(cookie_name: &str, secure: bool) -> crate::http::HeaderMap {
	let mut response_headers = crate::http::HeaderMap::new();
	let cookie = build_clear_cookie(cookie_name.to_string(), secure);
	append_set_cookie_header(&mut response_headers, &cookie);
	response_headers
}

pub(super) fn set_session_cookies(
	cookie_name: &str,
	secure: bool,
	value: String,
	cookie_max_age: cookie::time::Duration,
) -> Result<crate::http::HeaderMap, SessionCookieError> {
	if value.len() > MAX_COOKIE_SIZE {
		return Err(SessionCookieError::TooLarge);
	}
	let mut headers = crate::http::HeaderMap::new();
	let cookie = build_session_cookie(cookie_name.to_string(), value, secure, cookie_max_age);
	append_set_cookie_header(&mut headers, &cookie);
	Ok(headers)
}
