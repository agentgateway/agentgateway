use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::time::Duration;

use bytes::Bytes;
use http_body::Frame;
use parking_lot::Mutex;
use tokio::time::{Instant, Sleep};

use crate::BodyTimeoutError;

/// Extracted content shares progress with its owner, but has its own timer/waker.
#[derive(Debug)]
pub(crate) struct IdleTimeout {
	duration: Duration,
	expires_at: Arc<Mutex<Instant>>,
	sleep: Option<Pin<Box<Sleep>>>,
}

impl Clone for IdleTimeout {
	fn clone(&self) -> Self {
		Self {
			duration: self.duration,
			expires_at: self.expires_at.clone(),
			sleep: None,
		}
	}
}

impl IdleTimeout {
	pub(crate) fn new(duration: Duration) -> Self {
		Self {
			duration,
			expires_at: Arc::new(Mutex::new(Instant::now() + duration)),
			sleep: None,
		}
	}

	pub(crate) fn check(&self) -> Result<(), axum_core::Error> {
		if Instant::now() >= *self.expires_at.lock() {
			return Err(axum_core::Error::new(BodyTimeoutError));
		}
		Ok(())
	}

	pub(crate) fn poll_expired(&mut self, cx: &mut Context<'_>) -> bool {
		let expires_at = *self.expires_at.lock();
		if Instant::now() >= expires_at {
			return true;
		}
		let sleep = self
			.sleep
			.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(expires_at)));
		sleep.as_mut().reset(expires_at);
		sleep.as_mut().poll(cx).is_ready()
	}

	pub(crate) fn on_frame(&mut self, _frame: &Frame<Bytes>) {
		// Any successfully received frame signals liveness, including empty DATA.
		*self.expires_at.lock() = Instant::now() + self.duration;
	}
}
