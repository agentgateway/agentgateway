use std::net::SocketAddr;
use std::time::Instant;

use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use tokio::sync::mpsc;

use crate::common::prelude::*;

const BODY: &[u8] = b"hello";

async fn holding_backend(hold: Duration) -> (SocketAddr, mpsc::UnboundedReceiver<()>) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let (in_flight, in_flight_rx) = mpsc::unbounded_channel();
	tokio::spawn(async move {
		loop {
			let (stream, _) = listener.accept().await.unwrap();
			let in_flight = in_flight.clone();
			tokio::spawn(async move {
				let service = service_fn(move |_req: http::Request<hyper::body::Incoming>| {
					let in_flight = in_flight.clone();
					async move {
						let _ = in_flight.send(());
						tokio::time::sleep(hold).await;
						Ok::<_, Infallible>(http::Response::new(Body::from(BODY)))
					}
				});
				let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
					.serve_connection(TokioIo::new(stream), service)
					.await;
			});
		}
	});
	(addr, in_flight_rx)
}

async fn gateway(
	min_deadline: &str,
	max_deadline: &str,
	backend: SocketAddr,
) -> (TestBind, SocketAddr) {
	let cfg = json!({
		"config": {
			"connectionMinTerminationDeadline": min_deadline,
			"connectionTerminationDeadline": max_deadline,
		}
	});
	let test = setup_proxy_test(&cfg.to_string())
		.unwrap()
		.with_backend(backend)
		.with_bind(simple_bind())
		.with_route(basic_route(backend));
	let addr = test.serve_gateway_listener(BIND_KEY).await;
	(test, addr)
}

fn http1_client() -> Client<HttpConnector, Body> {
	Client::builder(TokioExecutor::new())
		.timer(TokioTimer::new())
		.build_http()
}

fn http2_client() -> Client<HttpConnector, Body> {
	Client::builder(TokioExecutor::new())
		.timer(TokioTimer::new())
		.http2_only(true)
		.build_http()
}

async fn get(
	client: Client<HttpConnector, Body>,
	addr: SocketAddr,
) -> Result<Response, agentgateway::http::Error> {
	let url = format!("http://127.0.0.1:{}/", addr.port());
	RequestBuilder::new(Method::GET, &url).send(client).await
}

async fn assert_full_response(resp: Response) -> HeaderMap {
	assert_eq!(resp.status(), StatusCode::OK);
	let headers = resp.headers().clone();
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(body.as_ref(), BODY);
	headers
}

#[tokio::test]
async fn drain_waits_for_in_flight_request() {
	let (backend, mut in_flight) = holding_backend(Duration::from_secs(1)).await;
	let (test, addr) = gateway("0s", "30s", backend).await;

	let request = tokio::spawn(get(http1_client(), addr));
	in_flight.recv().await.unwrap();
	let drained = tokio::spawn(test.start_drain());

	let headers = assert_full_response(request.await.unwrap().unwrap()).await;
	assert_eq!(headers[header::CONNECTION], "close");
	tokio::time::timeout(Duration::from_secs(1), drained)
		.await
		.expect("drain must finish right after the last connection closes")
		.unwrap();
}

#[tokio::test]
async fn drain_waits_for_in_flight_http2_request() {
	let (backend, mut in_flight) = holding_backend(Duration::from_secs(1)).await;
	let (test, addr) = gateway("0s", "30s", backend).await;

	let request = tokio::spawn(get(http2_client(), addr));
	in_flight.recv().await.unwrap();
	let drained = tokio::spawn(test.start_drain());

	assert_full_response(request.await.unwrap().unwrap()).await;
	tokio::time::timeout(Duration::from_secs(1), drained)
		.await
		.expect("drain must finish right after the last connection closes")
		.unwrap();
}

#[tokio::test]
async fn drain_cuts_request_past_deadline() {
	let (backend, mut in_flight) = holding_backend(Duration::from_secs(10)).await;
	let (test, addr) = gateway("0s", "500ms", backend).await;

	let request = tokio::spawn(get(http1_client(), addr));
	in_flight.recv().await.unwrap();
	let start = Instant::now();
	let drained = tokio::spawn(test.start_drain());

	let err = request.await.unwrap().expect_err("request must be cut");
	assert!(
		format!("{err:?}").contains("IncompleteMessage"),
		"expected a mid-response cut, got {err:?}"
	);
	tokio::time::timeout(Duration::from_secs(2), drained)
		.await
		.expect("drain must finish at the deadline")
		.unwrap();
	let elapsed = start.elapsed();
	assert!(
		elapsed >= Duration::from_millis(500),
		"drain ended before the deadline: {elapsed:?}"
	);
}

#[tokio::test]
async fn drain_serves_new_connections_during_minimum() {
	let (backend, _in_flight) = holding_backend(Duration::ZERO).await;
	let (test, addr) = gateway("1s", "30s", backend).await;

	let start = Instant::now();
	let drained = tokio::spawn(test.start_drain());
	tokio::time::sleep(Duration::from_millis(200)).await;

	assert_full_response(get(http1_client(), addr).await.unwrap()).await;
	tokio::time::timeout(Duration::from_secs(2), drained)
		.await
		.expect("drain must finish once the minimum passes")
		.unwrap();
	let elapsed = start.elapsed();
	assert!(
		elapsed >= Duration::from_secs(1),
		"drain ended before the minimum: {elapsed:?}"
	);
}

#[tokio::test]
async fn drain_deadline_includes_minimum() {
	let (backend, mut in_flight) = holding_backend(Duration::from_secs(10)).await;
	let (test, addr) = gateway("300ms", "500ms", backend).await;

	let request = tokio::spawn(get(http1_client(), addr));
	in_flight.recv().await.unwrap();
	let start = Instant::now();
	let drained = tokio::spawn(test.start_drain());

	request.await.unwrap().expect_err("request must be cut");
	drained.await.unwrap();
	let elapsed = start.elapsed();
	assert!(
		elapsed >= Duration::from_millis(500) && elapsed < Duration::from_millis(800),
		"drain must end at the maximum, not minimum + maximum: {elapsed:?}"
	);
}
