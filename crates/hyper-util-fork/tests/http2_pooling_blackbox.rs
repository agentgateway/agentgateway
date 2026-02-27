use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Uri, Version};
use hyper_util_fork::client::legacy::connect::{Connected, Connection};
use hyper_util_fork::client::legacy::Client;
use hyper_util_fork::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tower_service::Service;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TestPoolKey(&'static str);

#[derive(Clone, Default)]
struct CountingConnector {
	connections: Arc<AtomicUsize>,
	requests: Arc<AtomicUsize>,
	connect_delay: Option<Duration>,
	proto: TestProto,
}

#[derive(Clone, Copy, Default)]
enum TestProto {
	#[default]
	H2,
	H1AlpnH1,
}

impl CountingConnector {
	fn with_connect_delay(connect_delay: Duration) -> Self {
		Self {
			connections: Arc::new(AtomicUsize::new(0)),
			requests: Arc::new(AtomicUsize::new(0)),
			connect_delay: Some(connect_delay),
			proto: TestProto::H2,
		}
	}

	fn with_h1_alpn() -> Self {
		Self {
			connections: Arc::new(AtomicUsize::new(0)),
			requests: Arc::new(AtomicUsize::new(0)),
			connect_delay: None,
			proto: TestProto::H1AlpnH1,
		}
	}

	fn connection_count(&self) -> usize {
		self.connections.load(Ordering::Acquire)
	}

	fn request_count(&self) -> usize {
		self.requests.load(Ordering::Acquire)
	}
}

struct TestIo {
	stream: DuplexStream,
	proto: TestProto,
}

impl TestIo {
	fn new(stream: DuplexStream, proto: TestProto) -> Self {
		Self { stream, proto }
	}
}

impl AsyncRead for TestIo {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		Pin::new(&mut self.stream).poll_read(cx, buf)
	}
}

impl AsyncWrite for TestIo {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, io::Error>> {
		Pin::new(&mut self.stream).poll_write(cx, buf)
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		Pin::new(&mut self.stream).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		Pin::new(&mut self.stream).poll_shutdown(cx)
	}
}

impl Connection for TestIo {
	fn connected(&self) -> Connected {
		match self.proto {
			TestProto::H2 => Connected::new(),
			TestProto::H1AlpnH1 => Connected::new().negotiated_h1(),
		}
	}
}

impl Service<http::Extensions> for CountingConnector {
	type Response = TokioIo<TestIo>;
	type Error = io::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, _dst: http::Extensions) -> Self::Future {
		self.connections.fetch_add(1, Ordering::AcqRel);
		let (client, server) = tokio::io::duplex(8192);
		let requests = Arc::clone(&self.requests);
		let connect_delay = self.connect_delay;
		let proto = self.proto;
		tokio::spawn(async move {
			let service = service_fn(move |req: Request<Incoming>| {
				let requests = Arc::clone(&requests);
				async move {
					requests.fetch_add(1, Ordering::AcqRel);
					if let Some(delay) = query_delay(req.uri()) {
						tokio::time::sleep(delay).await;
					}
					Ok::<_, Infallible>(
						Response::builder()
							.status(200)
							.version(match proto {
								TestProto::H2 => Version::HTTP_2,
								TestProto::H1AlpnH1 => Version::HTTP_11,
							})
							.body(Full::new(Bytes::from_static(b"ok")))
							.expect("response body"),
					)
				}
			});
			match proto {
				TestProto::H2 => {
					let _ = http2::Builder::new(TokioExecutor::new())
						.serve_connection(TokioIo::new(TestIo::new(server, proto)), service)
						.await;
				},
				TestProto::H1AlpnH1 => {
					let _ = http1::Builder::new()
						.serve_connection(TokioIo::new(TestIo::new(server, proto)), service)
						.await;
				},
			}
		});

		Box::pin(async move {
			if let Some(delay) = connect_delay {
				tokio::time::sleep(delay).await;
			}
			Ok(TokioIo::new(TestIo::new(client, proto)))
		})
	}
}

fn query_delay(uri: &Uri) -> Option<Duration> {
	uri.query()?.split('&').find_map(|pair| {
		let (key, value) = pair.split_once('=')?;
		if key != "delay_ms" {
			return None;
		}
		value.parse::<u64>().ok().map(Duration::from_millis)
	})
}

fn test_client(
	connector: CountingConnector,
) -> Client<CountingConnector, Empty<Bytes>, TestPoolKey> {
	let mut builder = Client::builder(TokioExecutor::new());
	builder.pool_timer(hyper_util_fork::rt::tokio::TokioTimer::new());
	builder.timer(hyper_util_fork::rt::tokio::TokioTimer::new());
	builder.build_with_pool_key(connector)
}

fn test_client_auto(
	connector: CountingConnector,
) -> Client<CountingConnector, Empty<Bytes>, TestPoolKey> {
	let mut builder = Client::builder(TokioExecutor::new());
	builder.pool_timer(hyper_util_fork::rt::tokio::TokioTimer::new());
	builder.timer(hyper_util_fork::rt::tokio::TokioTimer::new());
	builder.build_with_pool_key(connector)
}

fn request(path_and_query: &str) -> Request<Empty<Bytes>> {
	let mut req = Request::builder()
		.version(Version::HTTP_2)
		.uri(format!("http://example.test{path_and_query}"))
		.body(Empty::new())
		.expect("request body");
	req.extensions_mut().insert(TestPoolKey("example"));
	req
}

async fn send_request(
	client: &Client<CountingConnector, Empty<Bytes>, TestPoolKey>,
	path_and_query: &str,
) -> StatusCode {
	let fut = client.request(request(path_and_query));
	let res = tokio::time::timeout(Duration::from_secs(5), fut)
		.await
		.expect("request should not hang")
		.expect("request should succeed");
	let status = res.status();
	res
		.into_body()
		.collect()
		.await
		.expect("response body should complete");
	status
}

#[tokio::test]
async fn http2_blackbox_reuses_connection_for_sequential_requests() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	assert_eq!(send_request(&client, "/").await, StatusCode::OK);
	assert_eq!(send_request(&client, "/").await, StatusCode::OK);
	assert_eq!(connector.request_count(), 2);
	assert_eq!(connector.connection_count(), 1);
}

#[tokio::test]
async fn http2_blackbox_uses_single_connection_for_concurrent_requests() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	let c1 = client.clone();
	let c2 = client.clone();
	let (res1, res2) = tokio::join!(
		send_request(&c1, "/?delay_ms=100"),
		send_request(&c2, "/?delay_ms=100")
	);
	assert_eq!(res1, StatusCode::OK);
	assert_eq!(res2, StatusCode::OK);
	assert_eq!(connector.request_count(), 2);
	assert_eq!(connector.connection_count(), 1);
}

#[tokio::test]
async fn http2_blackbox_reuses_connection_after_concurrent_requests() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	let c1 = client.clone();
	let c2 = client.clone();
	let (res1, res2) = tokio::join!(
		send_request(&c1, "/?delay_ms=100"),
		send_request(&c2, "/?delay_ms=100")
	);
	assert_eq!(res1, StatusCode::OK);
	assert_eq!(res2, StatusCode::OK);

	assert_eq!(send_request(&client, "/").await, StatusCode::OK);
	assert_eq!(connector.request_count(), 3);
	assert_eq!(connector.connection_count(), 1);
}

#[tokio::test]
async fn http2_blackbox_waits_for_connect_instead_of_opening_second_connection() {
	let connector = CountingConnector::with_connect_delay(Duration::from_millis(50));
	let client = test_client(connector.clone());

	let c1 = client.clone();
	let c2 = client.clone();
	let (res1, res2) = tokio::join!(
		send_request(&c1, "/?delay_ms=100"),
		send_request(&c2, "/?delay_ms=100")
	);
	assert_eq!(res1, StatusCode::OK);
	assert_eq!(res2, StatusCode::OK);
	assert_eq!(connector.request_count(), 2);
	assert_eq!(connector.connection_count(), 1);
}

#[tokio::test]
async fn http2_blackbox_completes_burst_without_retry_spin() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	let burst = async {
		let mut tasks = Vec::new();
		for _ in 0..40 {
			let c = client.clone();
			tasks.push(tokio::spawn(async move {
				send_request(&c, "/?delay_ms=200").await
			}));
		}

		for task in tasks {
			let status = task.await.expect("request task should join");
			assert_eq!(status, StatusCode::OK);
		}
	};

	tokio::time::timeout(Duration::from_secs(10), burst)
		.await
		.expect("burst should complete without hanging");
	assert_eq!(connector.request_count(), 40);
}

#[tokio::test]
async fn http2_blackbox_does_not_churn_new_connections_under_21_client_load() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	let warmup = async {
		let mut tasks = Vec::new();
		for _ in 0..40 {
			let c = client.clone();
			tasks.push(tokio::spawn(async move {
				send_request(&c, "/?delay_ms=200").await
			}));
		}
		for task in tasks {
			assert_eq!(
				task.await.expect("warmup request task should join"),
				StatusCode::OK
			);
		}
	};
	tokio::time::timeout(Duration::from_secs(10), warmup)
		.await
		.expect("warmup burst should complete");
	assert_eq!(connector.connection_count(), 2);

	for _ in 0..3 {
		let mut tasks = Vec::new();
		for _ in 0..21 {
			let c = client.clone();
			tasks.push(tokio::spawn(async move {
				send_request(&c, "/?delay_ms=200").await
			}));
		}
		for task in tasks {
			assert_eq!(
				task.await.expect("load request task should join"),
				StatusCode::OK
			);
		}
	}

	assert_eq!(
		connector.connection_count(),
		2,
		"connection pool should reuse 2 h2 upstream connections under steady 21-client load"
	);
}

#[tokio::test]
async fn http2_blackbox_scales_above_two_connections_for_large_burst() {
	let connector = CountingConnector::default();
	let client = test_client(connector.clone());

	let mut tasks = Vec::new();
	for _ in 0..100 {
		let c = client.clone();
		tasks.push(tokio::spawn(async move {
			send_request(&c, "/?delay_ms=200").await
		}));
	}

	for task in tasks {
		assert_eq!(
			task.await.expect("request task should join"),
			StatusCode::OK
		);
	}

	assert!(
		connector.connection_count() > 2,
		"pool should scale past 2 upstream h2 connections for a 100-request burst"
	);
}

#[tokio::test]
async fn http2_blackbox_allows_alpn_downgrade_to_http1_in_auto_mode() {
	let connector = CountingConnector::with_h1_alpn();
	let client = test_client_auto(connector.clone());

	assert_eq!(send_request(&client, "/").await, StatusCode::OK);
	assert_eq!(connector.connection_count(), 1);
	assert_eq!(connector.request_count(), 1);
}
