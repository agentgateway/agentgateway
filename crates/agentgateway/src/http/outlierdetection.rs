use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_core::durfmt;
use http::{HeaderMap, HeaderName, StatusCode, header};

use crate::http::x_headers;

fn get_header_as<T: FromStr>(h: &HeaderMap, name: &HeaderName) -> Option<T> {
	h.get(name)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.parse().ok())
}

fn get_header<'a>(h: &'a HeaderMap, name: &HeaderName) -> Option<&'a str> {
	h.get(name).and_then(|v| v.to_str().ok())
}

pub fn retry_after(status: StatusCode, h: &HeaderMap) -> Option<std::time::Duration> {
	if status == http::StatusCode::TOO_MANY_REQUESTS {
		process_rate_limit_headers(h, SystemTime::now())
	} else {
		None
	}
}

/// Some APIs may return rate limit information via response headers.
/// There is no single standard for this, so we must check a few common implementations.
fn process_rate_limit_headers(h: &HeaderMap, now: SystemTime) -> Option<std::time::Duration> {
	// `Retry-After`: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After
	// Value may be in seconds, or an HTTP date.
	// This is the only standardized header we can use.
	// Known to be used by: Anthropic.
	if let Some(retry_after) = get_header(h, &header::RETRY_AFTER) {
		if let Ok(seconds) = retry_after.parse::<u64>() {
			return Some(std::time::Duration::from_secs(seconds));
		}
		if let Ok(http_date) = httpdate::parse_http_date(retry_after)
			&& let Ok(duration) = http_date.duration_since(now)
		{
			return Some(duration);
		}
	}

	// x-ratelimit-reset: commonly used.
	// Typically this is a unix epoch timestamp OR number of seconds. Rarely it is number of milliseconds.
	// Known to be used by: GitHub.
	if let Some(retry_after) = get_header_as::<u64>(h, &x_headers::X_RATELIMIT_RESET) {
		const DAY: Duration = Duration::from_secs(60 * 60 * 24);
		if retry_after < 30 * DAY.as_secs() {
			// If the time is less than 30 days, its probably absolute seconds
			return Some(Duration::from_secs(retry_after));
		}
		// Else, its probably a unix epoch timestamp.
		let rt: SystemTime = UNIX_EPOCH + std::time::Duration::from_secs(retry_after);
		if let Ok(dur) = rt.duration_since(now) {
			return Some(dur);
		}
	}

	let smallest = &[
		// Used by OpenAI
		x_headers::X_RATELIMIT_RESET_REQUESTS,
		x_headers::X_RATELIMIT_RESET_TOKENS,
		// Used by Cerebras: https://inference-docs.cerebras.ai/support/rate-limits#rate-limit-headers
		x_headers::X_RATELIMIT_RESET_REQUESTS_DAY,
		x_headers::X_RATELIMIT_RESET_TOKENS_MINUTE,
	]
	.iter()
	.filter_map(|hn| {
		get_header(h, hn).and_then(|v| {
			if let Ok(d) = durfmt::parse(v) {
				Some(d)
			} else if v
				.chars()
				.last()
				.map(|c| c.is_ascii_digit())
				.unwrap_or(false)
			{
				// Treat as seconds
				durfmt::parse(&(v.to_string() + "s")).ok()
			} else {
				None
			}
		})
	})
	.min();
	if let Some(smallest) = smallest {
		return Some(*smallest);
	}
	None
}

#[cfg(test)]
#[path = "outlierdetction_tests.rs"]
mod tests;
