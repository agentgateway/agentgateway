use chrono::{DateTime, FixedOffset, Local};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug)]
pub struct Timestamp {
	instant: Instant,
	system: SystemTime,
}

impl Timestamp {
	pub fn now() -> Self {
		Self {
			instant: Instant::now(),
			system: SystemTime::now(),
		}
	}

	pub fn elapsed(&self) -> Duration {
		self.instant.elapsed()
	}

	pub fn as_system_time(&self) -> SystemTime {
		self.system
	}

	pub fn as_instant(&self) -> Instant {
		self.instant
	}

	/// The wall-clock time this Timestamp was created, as a DateTime<FixedOffset>
	pub fn as_datetime(&self) -> DateTime<FixedOffset> {
		DateTime::<Local>::from(self.system).into()
	}

	pub fn now_system(&self) -> SystemTime {
		self.system + self.instant.elapsed()
	}

	pub fn duration_since(&self, earlier: &Timestamp) -> Duration {
		self.instant.duration_since(earlier.instant)
	}
}
