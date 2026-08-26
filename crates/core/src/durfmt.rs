use std::time::Duration;

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum Error {
	#[error("parse error: {}", err_str(.0))]
	ParseError(go_parse_duration::Error),
}

fn err_str(e: &go_parse_duration::Error) -> &str {
	match e {
		go_parse_duration::Error::ParseError(s) => s,
	}
}

pub fn parse(string: &str) -> Result<Duration, Error> {
	if string.is_empty() {
		return Err(Error::ParseError(go_parse_duration::Error::ParseError(
			"duration cannot be empty".to_string(),
		)));
	}
	let day_digits = string.bytes().take_while(u8::is_ascii_digit).count();
	let (days, remainder) = if day_digits > 0 && string.as_bytes().get(day_digits) == Some(&b'd') {
		let days = string[..day_digits].parse::<u64>().map_err(|_| {
			Error::ParseError(go_parse_duration::Error::ParseError(
				"invalid day count".to_string(),
			))
		})?;
		(days, &string[day_digits + 1..])
	} else {
		(0, string)
	};
	let day_duration = days
		.checked_mul(24 * 60 * 60)
		.map(Duration::from_secs)
		.ok_or_else(|| {
			Error::ParseError(go_parse_duration::Error::ParseError(
				"duration overflow".to_string(),
			))
		})?;
	if remainder.is_empty() {
		return Ok(day_duration);
	}

	let d = go_parse_duration::parse_duration(remainder).map_err(Error::ParseError)?;
	if d < 0 {
		return Err(Error::ParseError(go_parse_duration::Error::ParseError(
			"negative string not allowed".to_string(),
		)));
	}
	day_duration
		.checked_add(Duration::from_nanos(d as u64))
		.ok_or_else(|| {
			Error::ParseError(go_parse_duration::Error::ParseError(
				"duration overflow".to_string(),
			))
		})
}

pub fn format(d: Duration) -> String {
	durationfmt::to_string(round_to_3_figs(d))
}

fn round_to_3_figs(d: Duration) -> Duration {
	if d <= Duration::from_millis(1) {
		return d;
	}

	let secs = d.as_secs();
	let nanos = d.subsec_nanos();
	if d.as_secs() <= 1 {
		Duration::new(secs, ((nanos as f64) / 1000.0).round() as u32 * 1000)
	} else {
		let rounded = ((nanos as f64) / 1000000.0).round() as u32 * 1000000;
		const NANOS_PER_SEC: u32 = 1_000_000_000;
		if rounded >= NANOS_PER_SEC {
			Duration::new(secs.checked_add(1).unwrap_or(secs), 0)
		} else {
			Duration::new(secs, rounded)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_days() {
		assert!(parse("").is_err());
		assert_eq!(parse("30d"), Ok(Duration::from_secs(30 * 24 * 60 * 60)));
		assert_eq!(
			parse("1d12h30m"),
			Ok(Duration::from_secs((24 + 12) * 60 * 60 + 30 * 60))
		);
		assert_eq!(parse("1d"), parse("24h"));
		assert!(parse("1.5d").is_err());
		assert!(parse("1h1d").is_err());
	}

	#[test]
	fn test_to_string() {
		assert_eq!("0s", format(Duration::new(0, 0)));
		assert_eq!("1ns", format(Duration::new(0, 1)));
		assert_eq!("1.1µs", format(Duration::new(0, 1100)));
		assert_eq!("12.345µs", format(Duration::new(0, 12345)));
		assert_eq!("2.2ms", format(Duration::new(0, 2_200_000)));
		assert_eq!("2.212ms", format(Duration::new(0, 2_212_345)));
		assert_eq!("100.567ms", format(Duration::new(0, 100_567_123)));
		assert_eq!("3.3s", format(Duration::new(3, 300_000_000)));
		assert_eq!("9m13.123s", format(Duration::new(553, 123_456_789)));
		assert_eq!("4m5s", format(Duration::new(4 * 60 + 5, 0)));
		assert_eq!("4m5.001s", format(Duration::new(4 * 60 + 5, 1_000_000)));
		assert_eq!(
			"5h6m7.001s",
			format(Duration::new((5 * 60 * 60) + (6 * 60) + 7, 1_000_000))
		);
		assert_eq!("8m0s", format(Duration::new(8 * 60, 1)));
		assert_eq!(
			"2562047h47m16.855s",
			format(Duration::new(
				(2562047 * 60 * 60) + (47 * 60) + 16,
				854_775_807
			))
		);
		assert_eq!("5124095576030431h0m15s", format(Duration::new(u64::MAX, 0)));
		assert_eq!(
			"5124095576030431h0m15s",
			format(Duration::new(u64::MAX, 999_999_999))
		);
		assert_eq!(
			"5124095576030431h0m15s",
			format(Duration::new(u64::MAX, 1_000))
		);
		assert_eq!(
			"5124095576030431h0m15.001s",
			format(Duration::new(u64::MAX, 1_000_000))
		);
	}
}
