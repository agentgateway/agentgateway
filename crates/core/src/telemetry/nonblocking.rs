// Derived from https://github.com/tokio-rs/tracing/blob/e63ef57f3d686abe3727ddd586eb9af73d6715b7/tracing-appender/src/non_blocking.rs#L1-L491
// Under MIT license
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{SendTimeoutError, Sender, bounded};
use tracing_subscriber::fmt::MakeWriter;

use super::msg::Msg;
use super::worker::Worker;

/// The default maximum number of buffered log lines.
///
/// If [`NonBlocking`] is lossy, it will drop spans/events at capacity.
/// If [`NonBlocking`] is _not_ lossy, backpressure will be exerted on
/// senders, causing them to block their respective threads until there
/// is available capacity.
///
/// Recommended to be a power of 2.
pub const DEFAULT_BUFFERED_LINES_LIMIT: usize = 128_000;

/// A guard that flushes spans/events associated to a [`NonBlocking`] on a drop
///
/// Writing to a [`NonBlocking`] writer will **not** immediately write a span or event to the underlying
/// output. Instead, the span or event will be written by a dedicated logging thread at some later point.
/// To increase throughput, the non-blocking writer will flush to the underlying output on
/// a periodic basis rather than every time a span or event is written. This means that if the program
/// terminates abruptly (such as through an uncaught `panic` or a `std::process::exit`), some spans
/// or events may not be written.
///
/// Since spans/events and events recorded near a crash are often necessary for diagnosing the failure,
/// `WorkerGuard` provides a mechanism to ensure that _all_ buffered logs are flushed to their output.
/// `WorkerGuard` should be assigned in the `main` function or whatever the entrypoint of the program is.
/// This will ensure that the guard will be dropped during an unwinding or when `main` exits
/// successfully.
///
/// # Examples
///
/// ``` rust,no_run
/// fn main () {
/// # fn doc() {
///     use agent_core::telemetry::nonblocking::NonBlockingBuilder;
///     let (non_blocking, _guard) = NonBlockingBuilder::default().finish(std::io::stdout());
///     let subscriber = tracing_subscriber::fmt().with_writer(non_blocking);
///     tracing::subscriber::with_default(subscriber.finish(), || {
///         // Emit some tracing events within context of the non_blocking `_guard` and tracing subscriber
///         tracing::event!(tracing::Level::INFO, "Hello");
///     });
///     // Exiting the context of `main` will drop the `_guard` and any remaining logs should get flushed
/// # }
/// }
/// ```
#[must_use]
#[derive(Debug)]
pub struct WorkerGuard {
	_guard: Option<JoinHandle<()>>,
	sender: Sender<Msg>,
	shutdown: Sender<()>,
}

/// A non-blocking writer.
///
/// While the line between "blocking" and "non-blocking" IO is fuzzy, writing to a file is typically
/// considered to be a _blocking_ operation. For an application whose `Subscriber` writes spans and events
/// as they are emitted, an application might find the latency profile to be unacceptable.
/// `NonBlocking` moves the writing out of an application's data path by sending spans and events
/// to a dedicated logging thread.
///
/// This struct implements [`MakeWriter`] from the `tracing-subscriber`
/// crate. Therefore, it can be used with the [`tracing_subscriber::fmt`][fmt] module
/// or with any other subscriber/layer implementation that uses the `MakeWriter` trait.
///
/// [fmt]: mod@tracing_subscriber::fmt
#[derive(Clone, Debug)]
pub struct NonBlocking {
	error_counter: ErrorCounter,
	channel: Sender<Msg>,
	is_lossy: bool,
}

/// Tracks the number of times a log line was dropped by the background thread.
///
/// If the non-blocking writer is not configured in [lossy mode], the error
/// count should always be 0.
///
/// [lossy mode]: NonBlockingBuilder::lossy
#[derive(Clone, Debug)]
pub struct ErrorCounter(Arc<AtomicUsize>);

impl NonBlocking {
	fn create<T: Write + Send + 'static>(
		writer: T,
		buffered_lines_limit: usize,
		is_lossy: bool,
		thread_name: String,
	) -> (NonBlocking, WorkerGuard) {
		let (sender, receiver) = bounded(buffered_lines_limit);

		let (shutdown_sender, shutdown_receiver) = bounded(0);

		let worker = Worker::new(receiver, writer, shutdown_receiver);
		let worker_guard = WorkerGuard::new(
			worker.worker_thread(thread_name),
			sender.clone(),
			shutdown_sender,
		);

		(
			Self {
				channel: sender,
				error_counter: ErrorCounter(Arc::new(AtomicUsize::new(0))),
				is_lossy,
			},
			worker_guard,
		)
	}

	/// Returns a counter for the number of times logs where dropped. This will always return zero if
	/// `NonBlocking` is not lossy.
	#[cfg(test)]
	pub fn error_counter(&self) -> ErrorCounter {
		self.error_counter.clone()
	}
}

/// A builder for [`NonBlocking`].
#[derive(Debug)]
pub struct NonBlockingBuilder {
	buffered_lines_limit: usize,
	is_lossy: bool,
	thread_name: String,
}

impl NonBlockingBuilder {
	/// Sets the number of lines to buffer before dropping logs or exerting backpressure on senders
	pub fn buffered_lines_limit(mut self, buffered_lines_limit: usize) -> NonBlockingBuilder {
		self.buffered_lines_limit = buffered_lines_limit;
		self
	}

	/// Sets whether `NonBlocking` should be lossy or not.
	///
	/// If set to `true`, logs will be dropped when the buffered limit is reached. If `false`, backpressure
	/// will be exerted on senders, blocking them until the buffer has capacity again.
	///
	/// By default, the built `NonBlocking` will be lossy.
	pub fn lossy(mut self, is_lossy: bool) -> NonBlockingBuilder {
		self.is_lossy = is_lossy;
		self
	}

	/// Completes the builder, returning the configured `NonBlocking`.
	pub fn finish<T: Write + Send + 'static>(self, writer: T) -> (NonBlocking, WorkerGuard) {
		NonBlocking::create(
			writer,
			self.buffered_lines_limit,
			self.is_lossy,
			self.thread_name,
		)
	}
}

impl Default for NonBlockingBuilder {
	fn default() -> Self {
		NonBlockingBuilder {
			buffered_lines_limit: DEFAULT_BUFFERED_LINES_LIMIT,
			is_lossy: true,
			thread_name: "tracing-appender".to_string(),
		}
	}
}

impl NonBlocking {
	pub fn write_vec(&self, buf: Vec<u8>) -> io::Result<usize> {
		let buf_size = buf.len();
		if self.is_lossy {
			if self.channel.try_send(Msg::Line(buf)).is_err() {
				self.error_counter.incr_saturating();
			}
		} else {
			return match self.channel.send(Msg::Line(buf)) {
				Ok(_) => Ok(buf_size),
				Err(_) => Err(io::Error::from(io::ErrorKind::Other)),
			};
		}
		Ok(buf_size)
	}
}

impl std::io::Write for NonBlocking {
	// Slow path: copy the data
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.write_vec(buf.to_vec())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}

	#[inline]
	fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
		self.write(buf).map(|_| ())
	}
}

impl<'a> MakeWriter<'a> for NonBlocking {
	type Writer = NonBlocking;

	fn make_writer(&'a self) -> Self::Writer {
		self.clone()
	}
}

impl WorkerGuard {
	fn new(handle: JoinHandle<()>, sender: Sender<Msg>, shutdown: Sender<()>) -> Self {
		WorkerGuard {
			_guard: Some(handle),
			sender,
			shutdown,
		}
	}
}

impl Drop for WorkerGuard {
	fn drop(&mut self) {
		match self
			.sender
			.send_timeout(Msg::Shutdown, Duration::from_millis(100))
		{
			Ok(_) => {
				// Attempt to wait for `Worker` to flush all messages before dropping. This happens
				// when the `Worker` calls `recv()` on a zero-capacity channel. Use `send_timeout`
				// so that drop is not blocked indefinitely.
				// TODO: Make timeout configurable.
				let _ = self.shutdown.send_timeout((), Duration::from_millis(1000));
			},
			Err(SendTimeoutError::Disconnected(_)) => (),
			Err(SendTimeoutError::Timeout(e)) => {
				println!("Failed to send shutdown signal to logging worker. Error: {e:?}")
			},
		}
	}
}

// === impl ErrorCounter ===

impl ErrorCounter {
	/// Returns the number of log lines that have been dropped.
	///
	/// If the non-blocking writer is not configured in [lossy mode], the error
	/// count should always be 0.
	///
	/// [lossy mode]: NonBlockingBuilder::lossy
	#[cfg(test)]
	pub fn dropped_lines(&self) -> usize {
		self.0.load(Ordering::Acquire)
	}

	fn incr_saturating(&self) {
		let mut curr = self.0.load(Ordering::Acquire);
		// We don't need to enter the CAS loop if the current value is already
		// `usize::MAX`.
		if curr == usize::MAX {
			return;
		}

		// This is implemented as a CAS loop rather than as a simple
		// `fetch_add`, because we don't want to wrap on overflow. Instead, we
		// need to ensure that saturating addition is performed.
		loop {
			let val = curr.saturating_add(1);
			match self
				.0
				.compare_exchange(curr, val, Ordering::AcqRel, Ordering::Acquire)
			{
				Ok(_) => return,
				Err(actual) => curr = actual,
			}
		}
	}
}

#[cfg(test)]
mod test {
	use std::sync::mpsc;
	use std::thread;
	use std::time::Duration;

	use super::*;

	struct MockWriter {
		tx: mpsc::SyncSender<String>,
	}

	impl MockWriter {
		fn new(capacity: usize) -> (Self, mpsc::Receiver<String>) {
			let (tx, rx) = mpsc::sync_channel(capacity);
			(Self { tx }, rx)
		}
	}

	impl std::io::Write for MockWriter {
		fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
			let buf_len = buf.len();
			let _ = self.tx.send(String::from_utf8_lossy(buf).to_string());
			Ok(buf_len)
		}

		fn flush(&mut self) -> std::io::Result<()> {
			Ok(())
		}
	}

	#[test]
	fn backpressure_exerted() {
		let (mock_writer, rx) = MockWriter::new(1);

		let (mut non_blocking, _guard) = self::NonBlockingBuilder::default()
			.lossy(false)
			.buffered_lines_limit(1)
			.finish(mock_writer);

		let error_count = non_blocking.error_counter();

		non_blocking.write_all(b"Hello").expect("Failed to write");
		assert_eq!(0, error_count.dropped_lines());

		let handle = thread::spawn(move || {
			non_blocking.write_all(b", World").expect("Failed to write");
		});

		// Sleep a little to ensure previously spawned thread gets blocked on write.
		thread::sleep(Duration::from_millis(100));
		// We should not drop logs when blocked.
		assert_eq!(0, error_count.dropped_lines());

		// Read the first message to unblock sender.
		let mut line = rx.recv().unwrap();
		assert_eq!(line, "Hello");

		// Wait for thread to finish.
		handle.join().expect("thread should not panic");

		// Thread has joined, we should be able to read the message it sent.
		line = rx.recv().unwrap();
		assert_eq!(line, ", World");
	}

	fn write_non_blocking(non_blocking: &mut NonBlocking, msg: &[u8]) {
		non_blocking.write_all(msg).expect("Failed to write");

		// Sleep a bit to prevent races.
		thread::sleep(Duration::from_millis(200));
	}

	#[test]
	#[ignore] // flaky, see https://github.com/tokio-rs/tracing/issues/751
	fn logs_dropped_if_lossy() {
		let (mock_writer, rx) = MockWriter::new(1);

		let (mut non_blocking, _guard) = self::NonBlockingBuilder::default()
			.lossy(true)
			.buffered_lines_limit(1)
			.finish(mock_writer);

		let error_count = non_blocking.error_counter();

		// First write will not block
		write_non_blocking(&mut non_blocking, b"Hello");
		assert_eq!(0, error_count.dropped_lines());

		// Second write will not block as Worker will have called `recv` on channel.
		// "Hello" is not yet consumed. MockWriter call to write_all will block until
		// "Hello" is consumed.
		write_non_blocking(&mut non_blocking, b", World");
		assert_eq!(0, error_count.dropped_lines());

		// Will sit in NonBlocking channel's buffer.
		write_non_blocking(&mut non_blocking, b"Test");
		assert_eq!(0, error_count.dropped_lines());

		// Allow a line to be written. "Hello" message will be consumed.
		// ", World" will be able to write to MockWriter.
		// "Test" will block on call to MockWriter's `write_all`
		let line = rx.recv().unwrap();
		assert_eq!(line, "Hello");

		// This will block as NonBlocking channel is full.
		write_non_blocking(&mut non_blocking, b"Universe");
		assert_eq!(1, error_count.dropped_lines());

		// Finally the second message sent will be consumed.
		let line = rx.recv().unwrap();
		assert_eq!(line, ", World");
		assert_eq!(1, error_count.dropped_lines());
	}

	#[test]
	fn multi_threaded_writes() {
		let (mock_writer, rx) = MockWriter::new(DEFAULT_BUFFERED_LINES_LIMIT);

		let (non_blocking, _guard) = self::NonBlockingBuilder::default()
			.lossy(true)
			.finish(mock_writer);

		let error_count = non_blocking.error_counter();
		let mut join_handles: Vec<JoinHandle<()>> = Vec::with_capacity(10);

		for _ in 0..10 {
			let cloned_non_blocking = non_blocking.clone();
			join_handles.push(thread::spawn(move || {
				let subscriber = tracing_subscriber::fmt().with_writer(cloned_non_blocking);
				tracing::subscriber::with_default(subscriber.finish(), || {
					tracing::event!(tracing::Level::INFO, "Hello");
				});
			}));
		}

		for handle in join_handles {
			handle.join().expect("Failed to join thread");
		}

		let mut hello_count: u8 = 0;

		while let Ok(event_str) = rx.recv_timeout(Duration::from_millis(25)) {
			assert!(event_str.contains("Hello"));
			hello_count += 1;
		}

		assert_eq!(10, hello_count);
		assert_eq!(0, error_count.dropped_lines());
	}
}
