use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::HeaderMap;

use super::*;

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
		assert_eq!(got, want, "headers: {headers:?} wanted {want:?}");
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
