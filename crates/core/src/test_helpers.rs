use std::future::Future;
use std::ops::Add;
use std::time::{Duration, SystemTime};

use tracing::trace;

/// check_eventually runs a function many times until it reaches the expected result.
/// If it doesn't the last result is returned.
pub async fn check_eventually<F, CF, T, Fut>(dur: Duration, f: F, expected: CF) -> Result<T, T>
where
	F: Fn() -> Fut,
	Fut: Future<Output = T>,
	CF: Fn(&T) -> bool,
{
	let mut delay = Duration::from_millis(10);
	let end = SystemTime::now().add(dur);
	let mut last: T;
	let mut attempts = 0;
	loop {
		attempts += 1;
		last = f().await;
		if expected(&last) {
			return Ok(last);
		}
		trace!("attempt {attempts} with delay {delay:?}");
		if SystemTime::now().add(delay) > end {
			return Err(last);
		}
		tokio::time::sleep(delay).await;
		delay *= 2;
	}
}
