use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response, Uri, Version};
use http_body_util::Empty;
use hyper::body::{Frame, Incoming, SizeHint};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tower_service::Service;

use crate::Client;
use crate::pool::{ExpectedCapacity, Key};
use crate::rt::{TokioExecutor, TokioIo, TokioTimer};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestKey {
	authority: http::uri::Authority,
}

impl Key for TestKey {
	fn expected_capacity(&self) -> ExpectedCapacity {
		ExpectedCapacity::Http2
	}
}

#[derive(Clone)]
struct TestConnector {
	addr: SocketAddr,
}

impl Service<http::Extensions> for TestConnector {
	type Response = TokioIo<TcpStream>;
	type Error = io::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, _req: http::Extensions) -> Self::Future {
		let addr = self.addr;
		Box::pin(async move { TcpStream::connect(addr).await.map(TokioIo::new) })
	}
}

struct BlockingEosBody {
	release: oneshot::Receiver<()>,
	done: bool,
}

impl BlockingEosBody {
	fn new(release: oneshot::Receiver<()>) -> Self {
		Self {
			release,
			done: false,
		}
	}
}

impl hyper::body::Body for BlockingEosBody {
	type Data = Bytes;
	type Error = Infallible;

	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		if self.done {
			return Poll::Ready(None);
		}

		match Pin::new(&mut self.release).poll(cx) {
			Poll::Ready(_) => {
				self.done = true;
				Poll::Ready(None)
			},
			Poll::Pending => Poll::Pending,
		}
	}

	fn is_end_stream(&self) -> bool {
		self.done
	}

	fn size_hint(&self) -> SizeHint {
		SizeHint::default()
	}
}

struct TestServer {
	addr: SocketAddr,
	accepted: Arc<AtomicUsize>,
	shutdown_tx: watch::Sender<bool>,
	accept_task: tokio::task::JoinHandle<()>,
}

impl TestServer {
	async fn spawn(max_streams: u32, blocking_responses: usize) -> (Self, Vec<oneshot::Sender<()>>) {
		let mut response_txs = Vec::with_capacity(blocking_responses);
		let mut response_rxs = VecDeque::with_capacity(blocking_responses);
		for _ in 0..blocking_responses {
			let (tx, rx) = oneshot::channel();
			response_txs.push(tx);
			response_rxs.push_back(rx);
		}
		let response_rxs = Arc::new(Mutex::new(response_rxs));
		let listener = TcpListener::bind(("127.0.0.1", 0))
			.await
			.expect("bind test listener");
		let addr = listener.local_addr().expect("listener addr");
		let accepted = Arc::new(AtomicUsize::new(0));
		let (shutdown_tx, shutdown_rx) = watch::channel(false);
		let accepted_count = accepted.clone();
		let accept_task = tokio::spawn(async move {
			let mut shutdown_rx = shutdown_rx;
			loop {
				tokio::select! {
						_ = shutdown_rx.changed() => break,
						res = listener.accept() => {
							let (stream, _) = res.expect("accept");
							accepted_count.fetch_add(1, Ordering::SeqCst);
							let hold_rx = shutdown_rx.clone();
							let response_rxs = response_rxs.clone();
							let service = service_fn(move |req: Request<Incoming>| {
								let mut hold_rx = hold_rx.clone();
								let response_rx = if req.uri().path() == "/hold-response" {
									response_rxs.lock().expect("response queue").pop_front()
								} else {
									None
								};
								async move {
									let (_parts, body) = req.into_parts();
									tokio::spawn(async move {
										let _body = body;
										let _ = hold_rx.changed().await;
									});
									let body = if let Some(response_rx) = response_rx {
										axum_core::body::Body::new(BlockingEosBody::new(response_rx))
									} else {
										axum_core::body::Body::new(Empty::<Bytes>::new())
									};
									Ok::<_, Infallible>(
										Response::builder()
											.status(200)
											.body(body)
											.expect("response body"),
									)
								}
							});
							tokio::spawn(async move {
							let _ = http2::Builder::new(TokioExecutor::new())
								.max_concurrent_streams(max_streams)
								.serve_connection(TokioIo::new(stream), service)
								.await;
						});
					}
				}
			}
		});

		(
			Self {
				addr,
				accepted,
				shutdown_tx,
				accept_task,
			},
			response_txs,
		)
	}

	fn accepted(&self) -> usize {
		self.accepted.load(Ordering::SeqCst)
	}

	async fn wait_for_accepted(&self, expected: usize) {
		tokio::time::timeout(Duration::from_secs(1), async {
			while self.accepted() < expected {
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap_or_else(|_| panic!("timed out waiting for {expected} accepted connections"));
	}

	async fn shutdown(self) {
		let _ = self.shutdown_tx.send(true);
		self.accept_task.abort();
		let _ = self.accept_task.await;
	}
}

fn make_h2_request(
	uri: &Uri,
	key: &TestKey,
	body: axum_core::body::Body,
) -> Request<axum_core::body::Body> {
	let mut req = Request::builder()
		.method(http::Method::POST)
		.version(Version::HTTP_2)
		.uri(uri.clone())
		.body(body)
		.expect("request");
	req.extensions_mut().insert(key.clone());
	req
}

fn build_client(server_addr: SocketAddr) -> Client<TestConnector, TestKey> {
	let mut builder = Client::<(), TestKey>::builder(TokioExecutor::new());
	builder.pool_timer(TokioTimer::new());
	builder.pool_expected_http2_capacity(2);
	builder.build(TestConnector { addr: server_addr })
}

#[tokio::test]
async fn h2_stream_capacity_must_follow_request_body_lifetime() {
	let (server, _response_txs) = TestServer::spawn(2, 0).await;
	let uri: Uri = format!("http://{}/hold", server.addr).parse().expect("uri");
	let key = TestKey {
		authority: uri.authority().expect("authority").clone(),
	};

	let client = build_client(server.addr);

	let (release1_tx, release1_rx) = oneshot::channel();
	let (release2_tx, release2_rx) = oneshot::channel();

	let response1 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&uri,
			&key,
			axum_core::body::Body::new(BlockingEosBody::new(release1_rx)),
		)),
	)
	.await
	.expect("request 1 timed out")
	.expect("request 1 failed");
	let response2 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&uri,
			&key,
			axum_core::body::Body::new(BlockingEosBody::new(release2_rx)),
		)),
	)
	.await
	.expect("request 2 timed out")
	.expect("request 2 failed");

	server.wait_for_accepted(1).await;
	drop(response1);
	drop(response2);

	let third = tokio::spawn({
		let client = client.clone();
		let uri = uri.clone();
		let key = key.clone();
		async move {
			client
				.request(make_h2_request(
					&uri,
					&key,
					axum_core::body::Body::new(Empty::<Bytes>::new()),
				))
				.await
		}
	});

	server.wait_for_accepted(2).await;

	let _ = release1_tx.send(());
	let _ = release2_tx.send(());
	let _ = tokio::time::timeout(Duration::from_secs(1), third)
		.await
		.expect("third request task timed out")
		.expect("third request join failed")
		.expect("third request failed");

	server.shutdown().await;
}

#[tokio::test]
async fn empty_request_and_response_reuse_single_h2_connection() {
	let (server, _response_txs) = TestServer::spawn(2, 0).await;
	let uri: Uri = format!("http://{}/empty", server.addr)
		.parse()
		.expect("uri");
	let key = TestKey {
		authority: uri.authority().expect("authority").clone(),
	};
	let client = build_client(server.addr);

	for _ in 0..3 {
		let response = tokio::time::timeout(
			Duration::from_secs(1),
			client.request(make_h2_request(
				&uri,
				&key,
				axum_core::body::Body::new(Empty::<Bytes>::new()),
			)),
		)
		.await
		.expect("request timed out")
		.expect("request failed");
		drop(response);
	}

	server.wait_for_accepted(1).await;
	tokio::time::sleep(Duration::from_millis(50)).await;
	assert_eq!(
		server.accepted(),
		1,
		"empty requests should reuse one h2 connection"
	);

	server.shutdown().await;
}

#[tokio::test]
async fn h2_response_body_lifetime_must_hold_capacity() {
	let (server, response_txs) = TestServer::spawn(2, 2).await;
	let hold_uri: Uri = format!("http://{}/hold-response", server.addr)
		.parse()
		.expect("uri");
	let empty_uri: Uri = format!("http://{}/empty", server.addr)
		.parse()
		.expect("uri");
	let key = TestKey {
		authority: hold_uri.authority().expect("authority").clone(),
	};
	let client = build_client(server.addr);

	let response1 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&hold_uri,
			&key,
			axum_core::body::Body::new(Empty::<Bytes>::new()),
		)),
	)
	.await
	.expect("request 1 timed out")
	.expect("request 1 failed");
	let response2 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&hold_uri,
			&key,
			axum_core::body::Body::new(Empty::<Bytes>::new()),
		)),
	)
	.await
	.expect("request 2 timed out")
	.expect("request 2 failed");

	server.wait_for_accepted(1).await;

	let third = tokio::spawn({
		let client = client.clone();
		let empty_uri = empty_uri.clone();
		let key = key.clone();
		async move {
			client
				.request(make_h2_request(
					&empty_uri,
					&key,
					axum_core::body::Body::new(Empty::<Bytes>::new()),
				))
				.await
		}
	});

	server.wait_for_accepted(2).await;

	for tx in response_txs {
		let _ = tx.send(());
	}
	drop(response1);
	drop(response2);
	let _ = tokio::time::timeout(Duration::from_secs(1), third)
		.await
		.expect("third request timed out")
		.expect("third request join failed")
		.expect("third request failed");

	server.shutdown().await;
}

#[tokio::test]
async fn released_response_bodies_allow_reuse_without_new_connection() {
	let (server, response_txs) = TestServer::spawn(2, 2).await;
	let hold_uri: Uri = format!("http://{}/hold-response", server.addr)
		.parse()
		.expect("uri");
	let empty_uri: Uri = format!("http://{}/empty", server.addr)
		.parse()
		.expect("uri");
	let key = TestKey {
		authority: hold_uri.authority().expect("authority").clone(),
	};
	let client = build_client(server.addr);

	let response1 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&hold_uri,
			&key,
			axum_core::body::Body::new(Empty::<Bytes>::new()),
		)),
	)
	.await
	.expect("request 1 timed out")
	.expect("request 1 failed");
	let response2 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&hold_uri,
			&key,
			axum_core::body::Body::new(Empty::<Bytes>::new()),
		)),
	)
	.await
	.expect("request 2 timed out")
	.expect("request 2 failed");

	server.wait_for_accepted(1).await;

	for tx in response_txs {
		let _ = tx.send(());
	}
	drop(response1);
	drop(response2);
	tokio::time::sleep(Duration::from_millis(50)).await;

	let response3 = tokio::time::timeout(
		Duration::from_secs(1),
		client.request(make_h2_request(
			&empty_uri,
			&key,
			axum_core::body::Body::new(Empty::<Bytes>::new()),
		)),
	)
	.await
	.expect("request 3 timed out")
	.expect("request 3 failed");
	drop(response3);

	tokio::time::sleep(Duration::from_millis(50)).await;
	assert_eq!(
		server.accepted(),
		1,
		"released response bodies should let the client reuse the original h2 connection",
	);

	server.shutdown().await;
}
