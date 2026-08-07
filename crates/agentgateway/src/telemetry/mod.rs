pub mod log;
pub mod log_store;
pub mod metrics;
pub mod trc;

/// Moves potentially blocking destruction off the calling thread.
#[derive(Debug)]
pub struct NonBlockingDrop<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> NonBlockingDrop<T> {
	pub fn new(value: T) -> Self {
		Self(Some(value))
	}
}

impl<T: Clone + Send + 'static> Clone for NonBlockingDrop<T> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}

impl<T: Send + 'static> std::ops::Deref for NonBlockingDrop<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		self.0.as_ref().expect("value is present until drop")
	}
}

impl<T: Send + 'static> Drop for NonBlockingDrop<T> {
	fn drop(&mut self) {
		let Some(value) = self.0.take() else {
			return;
		};
		if let Ok(runtime) = tokio::runtime::Handle::try_current() {
			let _task = runtime.spawn_blocking(move || drop(value));
		} else {
			let _thread = std::thread::spawn(move || drop(value));
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::mpsc;
	use std::time::Duration;

	use super::NonBlockingDrop;

	struct BlockingDrop {
		started: mpsc::Sender<()>,
		release: mpsc::Receiver<()>,
	}

	impl Drop for BlockingDrop {
		fn drop(&mut self) {
			let _ = self.started.send(());
			let _ = self.release.recv();
		}
	}

	#[test]
	fn non_blocking_drop_returns_before_inner_drop_completes() {
		let (started_tx, started_rx) = mpsc::channel();
		let (release_tx, release_rx) = mpsc::channel();
		let (returned_tx, returned_rx) = mpsc::channel();
		let thread = std::thread::spawn(move || {
			let runtime = tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
				.expect("build runtime");
			runtime.block_on(async move {
				drop(NonBlockingDrop::new(BlockingDrop {
					started: started_tx,
					release: release_rx,
				}));
				let _ = returned_tx.send(());
			});
		});

		let returned = returned_rx.recv_timeout(Duration::from_secs(1));
		let _ = release_tx.send(());
		thread.join().expect("runtime thread exits");

		assert!(returned.is_ok(), "wrapper drop blocked on its inner value");
		assert!(
			started_rx.recv_timeout(Duration::from_secs(1)).is_ok(),
			"inner value was not dropped"
		);
	}
}
