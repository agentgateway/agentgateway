use crate::http::x_headers;
use agent_core::durfmt;
use http::{HeaderMap, HeaderName, StatusCode, header};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
mod tests {
	use super::*;
	use http::HeaderMap;
	use std::time::{Duration, SystemTime, UNIX_EPOCH};

	fn assert(headers: &[(&str, &str)], want: Option<Duration>) {
		let mut h = HeaderMap::new();
		for (k, v) in headers.iter() {
			h.insert(HeaderName::from_str(k).unwrap(), v.parse().unwrap());
		}
		let got = process_rate_limit_headers(&h);
		assert_eq!(got, want, "headers: {:?} wanted {:?}", headers, want);
	}

	#[test]
	fn test_process_rate_limit_headers() {
		let now = SystemTime::now();
		let get = |headers: &[(&str, &str)]| {
			let mut h = HeaderMap::new();
			for (k, v) in headers.iter() {
				h.insert(HeaderName::from_str(k).unwrap(), v.parse().unwrap());
			}
			process_rate_limit_headers(&h, now)
		};
		let assert = |headers: &[(&str, &str)], want: Option<Duration>| {
			let got = get(headers);
			assert_eq!(got, want, "headers: {:?} wanted {:?}", headers, want);
		};
		assert(&[("retry-after", "120")], Some(Duration::from_secs(120)));
		assert(&[("retry-after", "60")], Some(Duration::from_secs(60)));
		assert(&[("retry-after", "0")], Some(Duration::from_secs(0)));

		assert(&[("retry-after", "120s")], None);
		assert(&[("retry-after", "invalid")], None);
		assert(&[("retry-after", "")], None);

		// These are second-based, so explicitly round
		let future_time = now + Duration::from_secs(300);
		let ds = httpdate::fmt_http_date(future_time);
		assert_eq!(get(&[("retry-after", &ds)]).unwrap().as_secs(), 299);
		let future_timestamp = (now + Duration::from_secs(240))
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
			.to_string();
		assert_eq!(
			get(&[("x-ratelimit-reset", &future_timestamp)])
				.unwrap()
				.as_secs(),
			239
		);
		// Epoch timestamp in the past
		let past_timestamp = (now - Duration::from_secs(99999))
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		assert(&[("x-ratelimit-reset", &past_timestamp.to_string())], None);

		// Seconds
		assert(
			&[("x-ratelimit-reset", "1234")],
			Some(Duration::from_secs(1234)),
		);

		assert(
			&[("x-ratelimit-reset-requests", "5m")],
			Some(Duration::from_secs(300)),
		);
		assert(
			&[("x-ratelimit-reset-requests", "1h")],
			Some(Duration::from_secs(3600)),
		);
		assert(
			&[("x-ratelimit-reset-requests", "30s")],
			Some(Duration::from_secs(30)),
		);
		assert(
			&[("x-ratelimit-reset-tokens", "2m30s")],
			Some(Duration::from_secs(150)),
		);
		assert(
			&[("x-ratelimit-reset-tokens", "1m")],
			Some(Duration::from_secs(60)),
		);
		assert(
			&[("x-ratelimit-reset-requests-day", "24h")],
			Some(Duration::from_secs(86400)),
		);
		assert(
			&[("x-ratelimit-reset-tokens-minute", "60s")],
			Some(Duration::from_secs(60)),
		);
		assert(
			&[("x-ratelimit-reset-tokens-minute", "1m")],
			Some(Duration::from_secs(60)),
		);
		assert(
			&[("x-ratelimit-reset-requests", "120")],
			Some(Duration::from_secs(120)),
		);
		assert(
			&[("x-ratelimit-reset-tokens", "300")],
			Some(Duration::from_secs(300)),
		);

		// Test multiple headers - should return smallest duration
		assert(
			&[
				("x-ratelimit-reset-requests", "300"),
				("x-ratelimit-reset-tokens", "60"),
			],
			Some(Duration::from_secs(60)),
		);
		assert(
			&[
				("x-ratelimit-reset-requests-day", "33011.382867097855"),
				("x-ratelimit-reset-tokens-minute", "11.1"),
			],
			Some(Duration::from_millis(11_100)),
		);

		assert(
			&[
				("x-ratelimit-reset-tokens", "1m"),
				("x-ratelimit-reset-requests", "2m"),
			],
			Some(Duration::from_secs(60)),
		);

		assert(&[("x-ratelimit-reset-requests", "invalid")], None);
		assert(&[("x-ratelimit-reset-tokens", "")], None);
		assert(&[], None);
		assert(&[("x-ratelimit-reset-requests", "1m2x")], None);
		assert(&[("x-ratelimit-reset-tokens", "abc")], None);
		assert(&[("x-ratelimit-reset-requests", "-1m")], None);
	}
}
