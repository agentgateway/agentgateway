use bytes::Bytes;
use http_body::Body as _;
use http_body_util::BodyExt;

use crate::Body;

#[tokio::test]
async fn inspection_intent_survives_replacement_and_replay() {
	let mut body = Body::from("original");
	assert!(!body.needs_inspection());
	let _ = body.inspect(100).await.unwrap();
	assert!(!body.needs_inspection());
	body.require_inspection();
	let content = body.take_content();
	assert!(content.needs_inspection());
	assert!(body.needs_inspection());
	body.restore_content(content);
	let (_, state) = body.into_replay_parts();
	let mut body = state.wrap(crate::RawBody::from("original"));
	body.replace_content(crate::RawBody::from("replacement").into());
	assert!(body.needs_inspection());
	assert!(body.inspection().is_none());
	let _ = body.inspect(100).await.unwrap();
	assert_eq!(body.known_bytes().unwrap(), "replacement");
}

#[tokio::test]
async fn restoring_extracted_content_preserves_inspection_and_recording_owner() {
	for limit in [2, 100] {
		let mut body = Body::from_stream(futures_util::stream::iter([
			Ok::<_, std::io::Error>(Bytes::from_static(b"abc")),
			Ok(Bytes::from_static(b"def")),
		]));
		let _ = body.inspect(limit).await.unwrap();
		let buffered = body.known_bytes().cloned();
		body.record(100);
		let recorded = body.recorded().unwrap().clone();
		let content = body.take_content();
		body.restore_content(content);
		assert!(body.inspection().is_some());
		assert_eq!(body.known_bytes(), buffered.as_ref());
		assert_eq!(body.collect().await.unwrap().to_bytes(), "abcdef");
		assert_eq!(recorded.bytes(), "abcdef");
		assert!(recorded.is_complete());
	}
}

#[tokio::test(start_paused = true)]
async fn observers_receive_stream_errors_and_idle_timeouts() {
	use std::sync::{Arc, Mutex};
	struct Observer(Arc<Mutex<Option<String>>>);
	impl crate::BodyObserver for Observer {
		fn on_error(&mut self, error: &crate::Error) {
			*self.0.lock().unwrap() = Some(error.to_string());
		}
	}
	for idle in [false, true] {
		let error = Arc::new(Mutex::new(None));
		let mut body = if idle {
			Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>())
		} else {
			Body::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
				std::io::Error::other("upstream failed"),
			)]))
		}
		.with_observer(Observer(error.clone()));
		body.set_idle_timeout(std::time::Duration::from_secs(1));
		let returned = body.frame().await.unwrap().unwrap_err();
		assert_eq!(
			error.lock().unwrap().as_deref(),
			Some(returned.to_string().as_str())
		);
	}
}

#[tokio::test]
async fn empty_replacement_completes_existing_recording_without_polling() {
	for replacement in [
		crate::BodyContent::Buffered(Bytes::new()),
		crate::BodyContent::Streaming(crate::RawBody::empty()),
	] {
		let mut body = Body::from("old content");
		body.record(1024);
		let recorded = body.recorded().unwrap().clone();
		assert!(body.frame().await.unwrap().is_ok());
		assert_eq!(recorded.bytes(), "old content");

		body.replace_content(replacement);
		assert!(body.is_end_stream());
		// Hyper can drop an empty body without polling it even once.
		drop(body);
		assert!(recorded.is_complete());
		assert!(recorded.bytes().is_empty());
	}
}

#[tokio::test]
async fn replay_state_has_per_attempt_recording_and_shared_lifetime() {
	use std::sync::Arc;
	use std::sync::atomic::{AtomicBool, Ordering};
	struct Guard(Arc<AtomicBool>);
	impl Drop for Guard {
		fn drop(&mut self) {
			self.0.store(true, Ordering::Relaxed);
		}
	}
	let dropped = Arc::new(AtomicBool::new(false));
	let mut original = Body::from("original").with_drop_guard(Guard(dropped.clone()));
	original.record(100);
	let (_input, state) = original.into_replay_parts();
	let mut first = state.wrap(crate::RawBody::from("original"));
	assert_eq!(first.known_bytes().unwrap(), "original");
	let first_recorded = first.recorded().unwrap().clone();
	// Lifecycle observers are already installed; content replacement is no longer legal.
	assert!(first.frame().await.unwrap().is_ok());
	drop(first);
	assert!(!dropped.load(Ordering::Relaxed));
	assert_eq!(first_recorded.bytes(), "original");

	let mut second = state.wrap(crate::RawBody::from("original"));
	let second_recorded = second.recorded().unwrap().clone();
	assert_eq!(second.known_bytes().unwrap(), "original");
	assert!(second_recorded.bytes().is_empty());
	assert!(second.frame().await.unwrap().is_ok());
	assert_eq!(first_recorded.bytes(), "original");
	assert_eq!(second_recorded.bytes(), "original");
	drop(state);
	assert!(!dropped.load(Ordering::Relaxed));
	drop(second);
	assert!(dropped.load(Ordering::Relaxed));
}

#[tokio::test]
async fn preserving_wrapper_keeps_cache_but_collection_still_polls_delivery() {
	use std::sync::Arc;
	use std::sync::atomic::{AtomicUsize, Ordering};
	let polled = Arc::new(AtomicUsize::new(0));
	let seen = polled.clone();
	// Safe: count polls but return every original frame unchanged, including trailers.
	let mut body = Body::from("hello").dangerous_wrap_stream_preserving_content(move |body| {
		crate::RawBody::new(body.map_frame(move |frame| {
			seen.fetch_add(1, Ordering::Relaxed);
			frame
		}))
	});
	assert_eq!(body.known_bytes(), Some(&Bytes::from_static(b"hello")));
	assert!(matches!(
		body.inspect(100).await.unwrap(),
		crate::BodyInspection::Complete(_)
	));
	assert_eq!(polled.load(Ordering::Relaxed), 0);
	assert_eq!(body.into_bytes(100).await.unwrap(), "hello");
	assert_eq!(polled.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn failed_or_cancelled_modification_is_terminal() {
	for cancel in [false, true] {
		let mut body = Body::from("original");
		{
			let modification = body.try_modify(|_content| async move {
				if cancel {
					std::future::pending::<()>().await;
				}
				Err::<crate::BodyContent, _>("modification failed")
			});
			tokio::pin!(modification);
			if cancel {
				assert!(futures_util::poll!(&mut modification).is_pending());
			} else {
				assert_eq!(modification.await, Err("modification failed"));
			}
		}
		assert!(body.inspection().is_none());
		assert!(!body.is_end_stream());
		assert!(body.frame().await.unwrap().is_err());
		assert!(body.inspect(100).await.is_err());
		assert!(body.into_bytes(100).await.is_err());
	}
}

#[tokio::test]
async fn failed_or_cancelled_inspection_is_terminal() {
	use futures_util::{StreamExt, stream};

	for cancel in [false, true] {
		let tail = stream::poll_fn(move |_| {
			if cancel {
				std::task::Poll::Pending
			} else {
				std::task::Poll::Ready(Some(Err(std::io::Error::other("read failed"))))
			}
		});
		let mut body = Body::from_stream(stream::iter([Ok(Bytes::from_static(b"hello"))]).chain(tail));
		assert!(matches!(
			body.inspect(2).await.unwrap(),
			crate::BodyInspection::Partial(_)
		));
		if cancel {
			let inspection = body.inspect(100);
			tokio::pin!(inspection);
			assert!(futures_util::poll!(&mut inspection).is_pending());
		} else {
			assert!(body.inspect(100).await.is_err());
		}
		assert!(body.inspection().is_none());
		assert!(!body.is_end_stream());
		assert_eq!(body.size_hint().exact(), None);
		// Even a smaller inspection must not reuse the old successful prefix.
		assert!(body.inspect(1).await.is_err());
		assert!(body.frame().await.unwrap().is_err());
		assert!(body.into_bytes(100).await.is_err());
	}
}

#[tokio::test(start_paused = true)]
async fn idle_timeout_survives_inspection_and_content_changes() {
	use std::time::Duration;

	use tokio::time::advance;

	let mut body = Body::new(http_body_util::Full::new(Bytes::from_static(b"original")));
	body.set_idle_timeout(Duration::from_secs(10));
	assert!(matches!(
		body.inspect(100).await.unwrap(),
		crate::BodyInspection::Complete(_)
	));
	assert_eq!(
		body.known_bytes().unwrap(),
		&Bytes::from_static(b"original")
	);
	assert_eq!(body.size_hint().exact(), Some(8));

	advance(Duration::from_secs(5)).await;
	body.replace_bytes(Bytes::from_static(b"changed"));
	body = body.transform_stream(|stream| stream);
	// Safe: identity wrapper preserves the entire stream and its trailers.
	body = body.dangerous_wrap_stream_preserving_content(|stream| stream);
	body.replace_bytes(Bytes::from_static(b"replacement"));
	// Content replacement must not restart the idle interval.
	advance(Duration::from_secs(5)).await;
	assert!(body.frame().await.unwrap().is_err());
	assert!(body.inspect(100).await.is_err());
	assert!(body.into_bytes(100).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn extracted_content_times_out_during_reads() {
	use std::time::Duration;

	use tokio::time::Instant;

	for operation in 0..3 {
		let mut body =
			Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
		let deadline = Instant::now() + Duration::from_secs(1);
		body.set_idle_timeout(Duration::from_secs(1));
		let mut content = body.take_content();
		match operation {
			0 => assert!(content.frame().await.unwrap().is_err()),
			1 => assert!(content.inspect(100).await.is_err()),
			2 => assert!(content.into_bytes(100).await.is_err()),
			_ => unreachable!(),
		}
		assert_eq!(Instant::now(), deadline);
		// Extraction also leaves the original owner's idle timeout intact.
		body.replace_bytes(Bytes::from_static(b"replacement"));
		assert!(body.frame().await.unwrap().is_err());
	}
}

#[tokio::test]
async fn recording_completes_on_final_data_frame_without_polling_none() {
	let mut empty = Body::new(http_body_util::Empty::<Bytes>::new());
	empty.record(100);
	assert!(empty.recorded().unwrap().is_complete());
	assert!(empty.recorded().unwrap().bytes().is_empty());

	// Exercise both representations; the streaming case delegates its EOF hint.
	for mut body in [
		Body::from("hello"),
		Body::new(http_body_util::Full::new(Bytes::from_static(b"hello"))),
	] {
		body.record(100);
		let recorded = body.recorded().unwrap().clone();
		assert!(!recorded.is_complete());

		let frame = body.frame().await.unwrap().unwrap();
		assert_eq!(frame.data_ref().unwrap(), &Bytes::from_static(b"hello"));
		assert!(body.is_end_stream());
		// Do not poll again or drop the body: the final frame itself must complete recording.
		assert!(recorded.is_complete());
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));
	}
}

#[tokio::test(start_paused = true)]
async fn inspection_resets_idle_timeout_per_chunk_and_shares_progress_with_owner() {
	use std::time::Duration;

	use tokio::time::{Instant, advance, sleep};

	let start = Instant::now();
	let stream = futures_util::stream::unfold(0, |n| async move {
		if n == 3 {
			return None;
		}
		sleep(Duration::from_millis(750)).await;
		Some((Ok::<_, std::io::Error>(Bytes::from_static(b"x")), n + 1))
	});
	let mut body = Body::from_stream(stream);
	body.set_idle_timeout(Duration::from_secs(1));
	let mut content = body.take_content();
	assert!(matches!(
		content.inspect(100).await.unwrap(),
		crate::BodyInspection::Complete(_)
	));
	assert_eq!(Instant::now() - start, Duration::from_millis(2250));
	// Total reading time exceeded the interval, but no individual gap did.
	// The original owner must see progress made by the extracted content.
	body.replace_bytes(Bytes::from_static(b"replacement"));
	assert!(body.inspect(100).await.is_ok());
	advance(Duration::from_secs(1)).await;
	assert!(body.frame().await.unwrap().is_err());
}

#[tokio::test]
async fn recording_waits_for_pending_trailers() {
	let mut body = Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
		[
			Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::from_static(b"hello"))),
			Ok(http_body::Frame::trailers(http::HeaderMap::new())),
		],
	)));
	body.record(100);
	let recorded = body.recorded().unwrap().clone();

	assert!(body.frame().await.unwrap().unwrap().data_ref().is_some());
	assert!(!recorded.is_complete());
	assert!(
		body
			.frame()
			.await
			.unwrap()
			.unwrap()
			.trailers_ref()
			.is_some()
	);
	// Trailers must complete recording even when the inner body provides no EOF hint.
	assert!(recorded.is_complete());
	assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));
}

#[tokio::test(start_paused = true)]
async fn empty_frames_keep_polling_and_inspection_alive_until_the_stream_stalls() {
	use std::time::Duration;

	use tokio::time::{Instant, sleep};

	for inspect in [false, true] {
		let start = Instant::now();
		let stream = futures_util::stream::unfold(0, |n| async move {
			if n == 3 {
				std::future::pending::<()>().await;
			}
			sleep(Duration::from_millis(750)).await;
			Some((Ok::<_, std::io::Error>(Bytes::new()), n + 1))
		});
		let mut body = Body::from_stream(stream);
		body.set_idle_timeout(Duration::from_secs(1));
		if inspect {
			assert!(body.inspect(100).await.is_err());
		} else {
			for _ in 0..3 {
				assert!(
					body
						.frame()
						.await
						.unwrap()
						.unwrap()
						.into_data()
						.unwrap()
						.is_empty()
				);
			}
			assert!(body.frame().await.unwrap().is_err());
		}
		// Three empty frames kept it alive; only the subsequent silence timed out.
		assert_eq!(Instant::now() - start, Duration::from_millis(3250));
	}
}
