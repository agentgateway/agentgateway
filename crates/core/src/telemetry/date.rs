use std::cell::RefCell;
use std::fmt::{self, Write};
use std::str;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// "2025-07-16T18:32:01.".len()
pub(crate) const DATE_VALUE_LENGTH: usize = 20;

pub fn build() -> String {
	let mut w = String::with_capacity(27);
	write(&mut w);
	w
}
pub(crate) fn write(dst: &mut String) {
	write_at(dst, SystemTime::now());
}

fn write_at(dst: &mut String, now: SystemTime) {
	CACHED.with(|cache| {
		cache.borrow_mut().check(&now);
		// Push the base
		dst.push_str(cache.borrow().buffer());
		// Now we need the nanos which are not cached
		let duration = now.duration_since(UNIX_EPOCH).unwrap();
		let nanos = duration.subsec_nanos();
		let micros = nanos / 1000;
		let mut buf = itoa::Buffer::new();
		let s = buf.format(micros);
		// Zero-pad to six digits so 3270 renders as "003270", not "3270"
		dst.push_str(&"000000"[s.len()..]);
		dst.push_str(s);
		// Finish off with a Z
		dst.push('Z');
	})
}

struct CachedDate {
	bytes: [u8; DATE_VALUE_LENGTH],
	pos: usize,
	next_update: SystemTime,
}

thread_local!(static CACHED: RefCell<CachedDate> = RefCell::new(CachedDate::new()));

impl CachedDate {
	fn new() -> Self {
		let mut cache = CachedDate {
			bytes: [0; DATE_VALUE_LENGTH],
			pos: 0,
			next_update: SystemTime::now(),
		};
		cache.update(&SystemTime::now());
		cache
	}

	fn buffer(&self) -> &str {
		unsafe { std::str::from_utf8_unchecked(&self.bytes[..]) }
	}

	fn check(&mut self, now: &SystemTime) {
		if now > &self.next_update {
			self.update(now);
		}
	}

	fn update(&mut self, now: &SystemTime) {
		let nanos = now
			.duration_since(UNIX_EPOCH)
			.unwrap_or_default()
			.subsec_nanos();

		self.render(now);
		self.next_update = *now + Duration::new(1, 0) - Duration::from_nanos(nanos as u64);
	}

	fn render(&mut self, now: &SystemTime) {
		self.pos = 0;
		let duration = now.duration_since(UNIX_EPOCH).unwrap();
		let seconds = duration.as_secs();

		let datetime = time::OffsetDateTime::from_unix_timestamp(seconds as i64).unwrap();
		let _ = write!(
			self,
			"{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.",
			datetime.year(),
			datetime.month() as u8,
			datetime.day(),
			datetime.hour(),
			datetime.minute(),
			datetime.second(),
		);
		debug_assert!(self.pos == DATE_VALUE_LENGTH);
	}
}

impl fmt::Write for CachedDate {
	fn write_str(&mut self, s: &str) -> fmt::Result {
		let len = s.len();
		self.bytes[self.pos..self.pos + len].copy_from_slice(s.as_bytes());
		self.pos += len;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn timestamp_fraction_has_six_digits() {
		let base = UNIX_EPOCH + Duration::from_secs(1_750_000_000);
		// The cache normally starts at the current time; seed it for this fixed timestamp.
		CACHED.with(|cache| cache.borrow_mut().update(&base));

		for (micros, expected) in [
			(0, "000000"),
			(1, "000001"),
			(3270, "003270"),
			(99_999, "099999"),
			(100_000, "100000"),
			(999_999, "999999"),
		] {
			let mut buf = String::new();
			write_at(&mut buf, base + Duration::from_micros(micros));
			assert_eq!(buf, format!("2025-06-15T15:06:40.{expected}Z"));
		}
	}
}
