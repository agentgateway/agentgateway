use std::fs::File;
use std::future::{ready, Ready};
use std::io;
use std::io::Write;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use bytes::Bytes;
use divan::Bencher;
use http::{Request, Uri};
use http_body_util::{BodyExt, Empty};
use hyper_util_fork::client::legacy::connect::{Connected, Connection};
use hyper_util_fork::client::legacy::Client;
use hyper_util_fork::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tower_service::Service as _;

const REQUESTS_PER_ITER: usize = 512;
const RESPONSE_BYTES: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n";

#[divan::bench]
fn pooled_client_call_sequential(b: Bencher) {
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("failed to create tokio runtime");

	b.bench_local(|| {
		runtime.block_on(async {
			let connector = MockConnector::new(REQUESTS_PER_ITER);
			let connects = connector.connects.clone();
			let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(connector);

			let uri: Uri = "http://mock.local/".parse().unwrap();
			let mut service = &client;
			for _ in 0..REQUESTS_PER_ITER {
				let mut req = Request::builder()
					.uri(uri.clone())
					.body(Empty::<Bytes>::new())
					.expect("request must be valid");
				req.extensions_mut().insert((
					http::uri::Scheme::HTTP,
					http::uri::Authority::from_static("mock.local"),
				));

				let response = service.call(req).await.expect("request should succeed");
				let _ = response
					.into_body()
					.collect()
					.await
					.expect("response body should be readable");
			}

			assert_eq!(connects.load(Ordering::Relaxed), 1);
		});
	});
}

#[derive(Clone)]
struct MockConnector {
	responses_per_connection: usize,
	connects: Arc<AtomicUsize>,
}

impl MockConnector {
	fn new(responses_per_connection: usize) -> Self {
		Self {
			responses_per_connection,
			connects: Arc::new(AtomicUsize::new(0)),
		}
	}
}

impl tower_service::Service<http::Extensions> for MockConnector {
	type Response = TokioIo<MockStream>;
	type Error = io::Error;
	type Future = Ready<Result<Self::Response, Self::Error>>;

	fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, _: http::Extensions) -> Self::Future {
		self.connects.fetch_add(1, Ordering::Relaxed);
		ready(Ok(TokioIo::new(MockStream::new(
			self.responses_per_connection,
		))))
	}
}

struct MockStream {
	pending_request: Vec<u8>,
	read_buf: Vec<u8>,
	read_cursor: usize,
	responses_emitted: usize,
	max_responses: usize,
	read_waker: Option<Waker>,
}

impl MockStream {
	fn new(max_responses: usize) -> Self {
		Self {
			pending_request: Vec::new(),
			read_buf: Vec::new(),
			read_cursor: 0,
			responses_emitted: 0,
			max_responses,
			read_waker: None,
		}
	}

	fn try_emit_response(&mut self) {
		while let Some(end) = find_headers_end(&self.pending_request) {
			self.pending_request.drain(..end + 4);

			if self.responses_emitted < self.max_responses {
				self.read_buf.extend_from_slice(RESPONSE_BYTES);
				self.responses_emitted += 1;
			}
		}

		if !self.read_buf.is_empty() {
			if let Some(waker) = self.read_waker.take() {
				waker.wake();
			}
		}
	}
}

impl Connection for MockStream {
	fn connected(&self) -> Connected {
		Connected::new()
	}
}

impl AsyncRead for MockStream {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		if self.read_cursor >= self.read_buf.len() {
			self.read_buf.clear();
			self.read_cursor = 0;
			self.read_waker = Some(cx.waker().clone());
			return Poll::Pending;
		}

		let remaining = &self.read_buf[self.read_cursor..];
		let to_copy = remaining.len().min(buf.remaining());
		buf.put_slice(&remaining[..to_copy]);
		self.read_cursor += to_copy;

		Poll::Ready(Ok(()))
	}
}

impl AsyncWrite for MockStream {
	fn poll_write(
		mut self: Pin<&mut Self>,
		_: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, io::Error>> {
		self.pending_request.extend_from_slice(buf);
		self.try_emit_response();
		Poll::Ready(Ok(buf.len()))
	}

	fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		Poll::Ready(Ok(()))
	}

	fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		Poll::Ready(Ok(()))
	}
}

fn find_headers_end(bytes: &[u8]) -> Option<usize> {
	bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn main() {
	eprintln!("Benchmarking...");
	#[cfg(all(not(test), not(feature = "internal_benches")))]
	panic!("benches must have -F internal_benches");
	with_profiling(divan::main);
}

#[cfg(not(target_family = "unix"))]
pub fn with_profiling(f: impl FnOnce()) {
	f()
}

#[cfg(target_family = "unix")]
pub fn with_profiling(f: impl FnOnce()) {
	use pprof::protos::Message;
	let guard = pprof::ProfilerGuardBuilder::default()
		.frequency(1000)
		.build()
		.unwrap();

	f();
	eprintln!("Writing profile to /tmp/pprof-agentgateway.prof...");

	let report = guard.report().build().unwrap();
	let profile = report.pprof().unwrap();

	let body = profile.write_to_bytes().unwrap();
	File::create("/tmp/pprof-agentgateway.prof")
		.unwrap()
		.write_all(&body)
		.unwrap()
}
