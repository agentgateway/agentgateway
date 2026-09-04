use std::time::Duration;

pub use internal::{
	DrainMode, ReleaseShutdown as DrainBlocker, Signal as DrainTrigger, Upgrader as DrainUpgrader,
	Watch as DrainWatcher,
};
use tokio::sync::watch;
use tracing::{Instrument, debug, info, warn};

/// New constructs a new pair for draining
/// * DrainTrigger can be used to start a draining sequence and wait for it to complete.
/// * DrainWatcher should be held by anything that wants to participate in the draining. This can be cloned,
///   and a drain will not complete until all outstanding DrainWatchers are dropped.
pub fn new() -> (DrainTrigger, DrainWatcher) {
	let (tx, rx) = internal::channel();
	(tx, rx)
}

/// run_with_drain provides a wrapper to run a future with graceful shutdown/draining support.
/// A caller should construct a future with takes two arguments:
/// * drain: while holding onto this, the future is marked as active, which will block the server from shutting down.
///   Additionally, it can be watched (with drain.signaled()) to see when to start a graceful shutdown.
/// * force_shutdown: when this is triggered, the future must forcefully shutdown any ongoing work ASAP.
///   This means the graceful drain exceeded the hard deadline, and all work must terminate now.
///   This is only required for spawned() tasks; otherwise, the future is dropped entirely, canceling all work.
pub async fn run_with_drain<F, O>(
	component: String,
	drain: DrainWatcher,
	deadline: Duration,
	min_delay: Duration,
	make_future: F,
) where
	F: AsyncFnOnce(DrainWatcher, watch::Receiver<()>) -> O,
	O: Send + 'static,
{
	let (sub_drain_signal, sub_drain) = new();
	let (trigger_force_shutdown, force_shutdown) = watch::channel(());
	let trigger_force_shutdown_cpy = trigger_force_shutdown.clone();
	// Stop accepting once we drain.
	// We will then allow connections up to `deadline` to terminate on their own.
	// After that, they will be forcefully terminated.
	let fut = make_future(sub_drain, force_shutdown).in_current_span();
	let watch = async move {
		let res = drain.wait_for_drain().await;
		if res.mode() == DrainMode::Graceful {
			info!(
				component,
				"drain started, waiting {:?}-{:?} for any connections to complete", min_delay, deadline
			);
			// Due to https://github.com/hyperium/hyper/issues/3961, we are forced to not start the drain until after
			// the deadline.
			// If this feature is implemented, we can instead join!() the min_delay and `start_drain_and_wait`.
			tokio::time::sleep(min_delay).await;
			info!(
				component,
				"minimum drain completed, waiting, not accepting new connections and waiting {:?} for any connections to complete",
				deadline
			);
			let res = tokio::time::timeout(
				deadline,
				sub_drain_signal.start_drain_and_wait(DrainMode::Graceful),
			)
			.await;
			if res.is_err() {
				// Not all connections completed within time, we will force shut them down
				warn!(
					component,
					"drain duration expired with pending connections, forcefully shutting down"
				);
			}
		} else {
			debug!(component, "terminating");
		}
		// Trigger force shutdown. In theory, this is only needed in the timeout case. However,
		// it doesn't hurt to always trigger it.
		let _ = trigger_force_shutdown.send(());

		info!(component, "shutdown complete");
	};
	tokio::select! {
		_ = fut => {
			// Trigger force shutdown. This probably is redundant and the future will not complete
			// until all tasks are done. But just to be sure send it, in case the future is watching this
			// but not holding any drain blockers.
			let _ = trigger_force_shutdown_cpy.send(());
		},
		_ = watch => {}
	}
}

mod internal {
	use tokio::sync::{mpsc, watch};

	/// Creates a drain channel.
	///
	/// The `Signal` is used to start a drain, and the `Watch` will be notified
	/// when a drain is signaled.
	pub fn channel() -> (Signal, Watch) {
		let (signal_tx, signal_rx) = watch::channel(None);
		let (drained_tx, drained_rx) = mpsc::channel(1);

		let signal = Signal {
			drained_rx,
			signal_tx,
		};
		let watch = Watch {
			drained_tx,
			signal_rx,
		};
		(signal, watch)
	}

	enum Never {}

	#[derive(Debug, Clone, Copy, PartialEq)]
	pub enum DrainMode {
		Immediate,
		Graceful,
	}

	/// Send a drain command to all watchers.
	pub struct Signal {
		drained_rx: mpsc::Receiver<Never>,
		signal_tx: watch::Sender<Option<DrainMode>>,
	}

	/// Watch for a drain command.
	///
	/// All `Watch` instances must be dropped for a `Signal::signal` call to
	/// complete.
	#[derive(Clone)]
	pub struct Watch {
		drained_tx: mpsc::Sender<Never>,
		signal_rx: watch::Receiver<Option<DrainMode>>,
	}
	#[derive(Clone)]
	pub struct Weak {
		signal_rx: watch::Receiver<Option<DrainMode>>,
	}
	pub struct Upgrader {
		drained_tx: Option<mpsc::Sender<Never>>,
	}

	impl Upgrader {
		pub fn disable(&mut self) {
			self.drained_tx = None;
		}
		pub fn upgrade(&self, other: Weak) -> Watch {
			let drained_tx = self.drained_tx.clone().unwrap_or_else(|| {
				// Create a dummy one if we have been disabled
				let (tx, _) = mpsc::channel(1);
				tx
			});
			Watch {
				drained_tx,
				signal_rx: other.signal_rx,
			}
		}
	}

	impl Watch {
		pub fn into_weak(self) -> (Upgrader, Weak) {
			let Self {
				drained_tx,
				signal_rx,
			} = self;
			(
				Upgrader {
					drained_tx: Some(drained_tx),
				},
				Weak { signal_rx },
			)
		}
	}

	#[must_use = "ReleaseShutdown should be dropped explicitly to release the runtime"]
	#[derive(Clone)]
	#[allow(dead_code)]
	pub struct ReleaseShutdown(mpsc::Sender<Never>, DrainMode);

	impl ReleaseShutdown {
		pub fn mode(&self) -> DrainMode {
			self.1
		}
	}

	impl Signal {
		/// Waits for all [`Watch`] instances to be dropped.
		pub async fn closed(&mut self) {
			self.signal_tx.closed().await;
		}

		pub fn count(&self) -> usize {
			self.signal_tx.receiver_count()
		}
		/// Asynchronously signals all watchers to begin draining gracefully and waits for all
		/// handles to be dropped.
		pub async fn start_drain_and_wait(mut self, mode: DrainMode) {
			// Update the state of the signal watch so that all watchers are observe
			// the change.
			let _ = self.signal_tx.send(Some(mode));

			// Wait for all watchers to release their drain handle.
			match self.drained_rx.recv().await {
				None => {},
				Some(n) => match n {},
			}
		}
	}

	impl Watch {
		/// Wrap a future for graceful shutdown watching.
		pub fn wrap_connection<C: crate::drain::hyperfork::GracefulConnection>(
			self,
			conn: C,
		) -> impl Future<Output = C::Output> {
			crate::drain::hyperfork::GracefulConnectionFuture::new(conn, self.wait_for_drain())
		}

		/// Returns a `ReleaseShutdown` handle after the drain has been signaled. The
		/// handle must be dropped when a shutdown action has been completed to
		/// unblock graceful shutdown.
		pub async fn wait_for_drain(mut self) -> ReleaseShutdown {
			// This future completes once `Signal::signal` has been invoked so that
			// the channel's state is updated.
			let mode = self
				.signal_rx
				.wait_for(Option::is_some)
				.await
				.map(|mode| mode.expect("already asserted it is_some"))
				// If we got an error, then the signal was dropped entirely. Presumably this means a graceful shutdown is not required.
				.unwrap_or(DrainMode::Immediate);

			// Return a handle that holds the drain channel, so that the signal task
			// is only notified when all handles have been dropped.
			ReleaseShutdown(self.drained_tx, mode)
		}
	}

	impl std::fmt::Debug for Signal {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("Signal").finish_non_exhaustive()
		}
	}

	impl std::fmt::Debug for Watch {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("Watch").finish_non_exhaustive()
		}
	}

	impl std::fmt::Debug for ReleaseShutdown {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("ReleaseShutdown").finish_non_exhaustive()
		}
	}
}

mod hyperfork {
	use std::fmt::Debug;
	use std::pin::Pin;
	use std::task::Poll;
	use std::{fmt, task};

	pub use hyper_util::server::graceful::GracefulConnection as HyperGracefulConnection;
	use pin_project_lite::pin_project;

	pub trait GracefulConnection: Future<Output = Result<(), Self::Error>> {
		/// The error type returned by the connection when used as a future.
		type Error;

		/// Start a graceful shutdown process for this connection.
		fn graceful_shutdown(self: Pin<&mut Self>);
	}

	impl<T: HyperGracefulConnection> GracefulConnection for T {
		type Error = T::Error;

		fn graceful_shutdown(self: Pin<&mut Self>) {
			self.graceful_shutdown()
		}
	}

	// Copied from hyper_util since it is private
	pin_project! {
			pub struct GracefulConnectionFuture<C, F: Future> {
					#[pin]
					conn: C,
					#[pin]
					cancel: F,
					#[pin]
					// If cancelled, this is held until the inner conn is done.
					cancelled_guard: Option<F::Output>,
			}
	}

	impl<C, F: Future> GracefulConnectionFuture<C, F> {
		pub fn new(conn: C, cancel: F) -> Self {
			Self {
				conn,
				cancel,
				cancelled_guard: None,
			}
		}
	}

	impl<C, F: Future> Debug for GracefulConnectionFuture<C, F> {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			f.debug_struct("GracefulConnectionFuture").finish()
		}
	}

	impl<C, F> Future for GracefulConnectionFuture<C, F>
	where
		C: GracefulConnection,
		F: Future,
	{
		type Output = C::Output;

		fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
			let mut this = self.project();
			if this.cancelled_guard.is_none()
				&& let Poll::Ready(guard) = this.cancel.poll(cx)
			{
				this.cancelled_guard.set(Some(guard));
				this.conn.as_mut().graceful_shutdown();
			}
			this.conn.poll(cx)
		}
	}
}

#[cfg(test)]
mod test {
	use std::pin::Pin;
	use std::sync::Arc;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::task;
	use std::task::Poll;
	use std::time::Duration;

	use pin_project_lite::pin_project;
	use tokio::sync::oneshot;
	use tokio::task::JoinHandle;

	use crate::drain;
	use crate::drain::DrainMode::Graceful;
	use crate::drain::DrainTrigger;

	#[derive(Debug, PartialEq)]
	enum ConnectionExit {
		Completed,
		Forced,
	}

	struct RunBindHarness {
		drain: DrainTrigger,
		connection_complete: oneshot::Sender<()>,
		connection_exit: oneshot::Receiver<ConnectionExit>,
		started: oneshot::Receiver<()>,
		run: JoinHandle<()>,
	}

	fn run_bind_harness(deadline: Duration, min_delay: Duration) -> RunBindHarness {
		let (drain, watcher) = drain::new();
		let (connection_complete, connection_complete_rx) = oneshot::channel();
		let (connection_exit_tx, connection_exit) = oneshot::channel();
		let (started_tx, started) = oneshot::channel();

		let run = tokio::spawn(drain::run_with_drain(
			"test bind".to_string(),
			watcher,
			deadline,
			min_delay,
			move |drain: drain::DrainWatcher, mut force_shutdown: tokio::sync::watch::Receiver<()>| async move {
				// Model run_bind: spawned connections hold upgraded drain watchers after
				// the accept future stops accepting and returns.
				let drain_watch = drain.clone();
				let (mut upgrader, weak) = drain.into_weak();
				let connection_drain = upgrader.upgrade(weak);
				tokio::spawn(async move {
					let exit = tokio::select! {
						_ = connection_complete_rx => ConnectionExit::Completed,
						_ = force_shutdown.changed() => ConnectionExit::Forced,
					};
					drop(connection_drain);
					let _ = connection_exit_tx.send(exit);
				});
				let _ = started_tx.send(());

				let drain = drain_watch.wait_for_drain().await;
				assert_eq!(drain.mode(), Graceful);
				upgrader.disable();
				drop(drain);
			},
		));

		RunBindHarness {
			drain,
			connection_complete,
			connection_exit,
			started,
			run,
		}
	}

	async fn start_graceful_drain(drain: DrainTrigger) -> JoinHandle<()> {
		let task = tokio::spawn(drain.start_drain_and_wait(Graceful));
		for _ in 0..3 {
			tokio::task::yield_now().await;
		}
		task
	}

	pin_project! {
			#[derive(Debug)]
			struct DummyConnection<F> {
					#[pin]
					future: F,
					shutdown_counter: Arc<AtomicUsize>,
			}
	}

	impl<F: Future> super::hyperfork::GracefulConnection for DummyConnection<F> {
		type Error = ();

		fn graceful_shutdown(self: Pin<&mut Self>) {
			self.shutdown_counter.fetch_add(1, Ordering::SeqCst);
		}
	}

	impl<F: Future> Future for DummyConnection<F> {
		type Output = Result<(), ()>;

		fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
			match self.project().future.poll(cx) {
				Poll::Ready(_) => Poll::Ready(Ok(())),
				Poll::Pending => Poll::Pending,
			}
		}
	}

	#[tokio::test(start_paused = true)]
	async fn run_with_drain_forces_connections_at_deadline() {
		let deadline = Duration::from_secs(5);
		let RunBindHarness {
			drain,
			connection_complete,
			mut connection_exit,
			started,
			run,
		} = run_bind_harness(deadline, Duration::ZERO);
		started.await.unwrap();
		let draining = start_graceful_drain(drain).await;

		tokio::time::advance(deadline - Duration::from_millis(1)).await;
		assert!(matches!(
			connection_exit.try_recv(),
			Err(oneshot::error::TryRecvError::Empty)
		));
		assert!(!run.is_finished());

		tokio::time::advance(Duration::from_millis(1)).await;
		assert_eq!(connection_exit.await.unwrap(), ConnectionExit::Forced);
		run.await.unwrap();
		draining.await.unwrap();
		drop(connection_complete);
	}

	#[tokio::test(start_paused = true)]
	async fn run_with_drain_finishes_when_connections_complete_early() {
		let deadline = Duration::from_secs(5);
		let RunBindHarness {
			drain,
			connection_complete,
			mut connection_exit,
			started,
			run,
		} = run_bind_harness(deadline, Duration::ZERO);
		started.await.unwrap();
		let draining = start_graceful_drain(drain).await;

		tokio::time::advance(Duration::from_secs(2)).await;
		assert!(matches!(
			connection_exit.try_recv(),
			Err(oneshot::error::TryRecvError::Empty)
		));
		connection_complete.send(()).unwrap();

		assert_eq!(connection_exit.await.unwrap(), ConnectionExit::Completed);
		run.await.unwrap();
		draining.await.unwrap();
	}

	#[tokio::test(start_paused = true)]
	async fn run_with_drain_keeps_connections_alive_after_min_delay() {
		let min_delay = Duration::from_secs(3);
		let RunBindHarness {
			drain,
			connection_complete,
			mut connection_exit,
			started,
			run,
		} = run_bind_harness(Duration::from_secs(5), min_delay);
		started.await.unwrap();
		let draining = start_graceful_drain(drain).await;

		tokio::time::advance(min_delay).await;
		tokio::task::yield_now().await;
		assert!(matches!(
			connection_exit.try_recv(),
			Err(oneshot::error::TryRecvError::Empty)
		));
		assert!(!run.is_finished());
		connection_complete.send(()).unwrap();
		assert_eq!(connection_exit.await.unwrap(), ConnectionExit::Completed);

		run.await.unwrap();
		draining.await.unwrap();
	}

	#[tokio::test]
	async fn test_graceful_shutdown_ok() {
		let (trigger, watcher) = drain::new();
		let shutdown_counter = Arc::new(AtomicUsize::new(0));
		let (dummy_tx, _) = tokio::sync::broadcast::channel(1);

		for i in 1..=3 {
			let mut dummy_rx = dummy_tx.subscribe();
			let shutdown_counter = shutdown_counter.clone();

			let future = async move {
				tokio::time::sleep(std::time::Duration::from_millis(i * 10)).await;
				let _ = dummy_rx.recv().await;
			};
			let dummy_conn = DummyConnection {
				future,
				shutdown_counter,
			};
			let conn = watcher.clone().wrap_connection(dummy_conn);
			tokio::spawn(async move {
				conn.await.unwrap();
			});
		}
		drop(watcher);

		assert_eq!(shutdown_counter.load(Ordering::SeqCst), 0);
		let _ = dummy_tx.send(());

		tokio::select! {
			_ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
				panic!("timeout")
			},
			_ = trigger.start_drain_and_wait(Graceful) => {
				assert_eq!(shutdown_counter.load(Ordering::SeqCst), 3);
			}
		}
	}

	#[cfg(not(miri))]
	#[tokio::test]
	async fn test_graceful_shutdown_timeout() {
		let (trigger, watcher) = drain::new();
		let shutdown_counter = Arc::new(AtomicUsize::new(0));

		for i in 1..=3 {
			let shutdown_counter = shutdown_counter.clone();

			let future = async move {
				if i == 1 {
					std::future::pending::<()>().await
				} else {
					std::future::ready(()).await
				}
			};
			let dummy_conn = DummyConnection {
				future,
				shutdown_counter,
			};
			let conn = watcher.clone().wrap_connection(dummy_conn);
			tokio::spawn(async move {
				conn.await.unwrap();
			});
		}
		drop(watcher);

		assert_eq!(shutdown_counter.load(Ordering::SeqCst), 0);
		tokio::select! {
			_ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
				assert_eq!(shutdown_counter.load(Ordering::SeqCst), 3);
			},
			_ = trigger.start_drain_and_wait(Graceful) => {
				panic!("shutdown should not be completed: as not all our conns finish")
			}
		}
	}

	#[tokio::test]
	async fn test_weak() {
		let (trigger, watcher) = drain::new();
		let (mut upgrader, weak) = watcher.into_weak();
		let weak2 = weak.clone();
		let (_, mut weak2_rx) = tokio::sync::broadcast::channel::<()>(1);
		tokio::task::spawn(async move {
			// Block for ever
			weak2_rx.recv().await.unwrap();
			weak2
		});
		let strong = upgrader.upgrade(weak);
		let (strong_tx, mut strong_rx) = tokio::sync::broadcast::channel::<()>(1);
		tokio::task::spawn(async move {
			// Block for ever
			strong_rx.recv().await.unwrap();
			strong
		});

		let wait = trigger.start_drain_and_wait(Graceful);
		tokio::pin!(wait);
		tokio::select! {
			_ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
			_ = &mut wait => {
				panic!("drain should not have completed")
			}
		}
		// Stop our 'strong' latch
		strong_tx.send(()).unwrap();
		// Upgrader is still around so we are not done yet.
		tokio::select! {
			_ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
			_ = &mut wait => {
				panic!("drain should not have completed")
			}
		}
		upgrader.disable();
		// Now we should be complete despite the weak holder
		tokio::select! {
			_ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
				panic!("timeout")
			},
			_ = &mut wait => {
			}
		}
	}
}
