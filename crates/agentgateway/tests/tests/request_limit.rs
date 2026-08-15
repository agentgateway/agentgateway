use std::net::SocketAddr;

use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::prelude::*;

fn http_client() -> Client<HttpConnector, Body> {
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

async fn http_get(
	client: Client<HttpConnector, Body>,
	addr: SocketAddr,
	path: &str,
) -> Result<StatusCode, String> {
	let url = format!("http://127.0.0.1:{}{path}", addr.port());
	RequestBuilder::new(Method::GET, &url)
		.send(client)
		.await
		.map(|res| res.status())
		.map_err(|e| e.to_string())
}

async fn http2_get(
	client: Client<HttpConnector, Body>,
	addr: SocketAddr,
	path: &str,
) -> Result<StatusCode, String> {
	let url = format!("http://127.0.0.1:{}{path}", addr.port());
	RequestBuilder::new(Method::GET, &url)
		.version(Version::HTTP_2)
		.send(client)
		.await
		.map(|res| res.status())
		.map_err(|e| e.to_string())
}

async fn gateway_with_http_limit(
	http: serde_json::Value,
	slow: Duration,
) -> (MockServer, TestBind, SocketAddr) {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path("/slow"))
		.respond_with(ResponseTemplate::new(200).set_delay(slow))
		.mount(&mock)
		.await;
	Mock::given(wiremock::matchers::path("/fast"))
		.respond_with(ResponseTemplate::new(200))
		.mount(&mock)
		.await;

	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	t.attach_frontend_policy(json!({ "http": http })).await;
	let addr = t.serve_real_listener(BIND_KEY).await;
	(mock, t, addr)
}

#[tokio::test]
async fn max_concurrent_requests_second_waits_then_succeeds() {
	let (_mock, _t, addr) = gateway_with_http_limit(
		json!({
			"maxConcurrentRequests": 1,
			"maxPendingRequests": 4,
			"maxRequestWait": "5s",
		}),
		Duration::from_millis(400),
	)
	.await;
	let client = http_client();

	let a = tokio::spawn(http_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let start = std::time::Instant::now();
	let b = http_get(client, addr, "/fast").await.unwrap();
	let waited = start.elapsed();

	let a = a.await.unwrap().unwrap();
	assert_eq!(a, StatusCode::OK);
	assert_eq!(b, StatusCode::OK);
	assert!(
		waited >= Duration::from_millis(250),
		"fast request must wait for the occupied slot; waited {waited:?}"
	);
}

#[tokio::test]
async fn max_concurrent_requests_queue_full_returns_503() {
	let (_mock, _t, addr) = gateway_with_http_limit(
		json!({
			"maxConcurrentRequests": 1,
			"maxPendingRequests": 0,
			"maxRequestWait": "5s",
		}),
		Duration::from_millis(800),
	)
	.await;
	let client = http_client();

	let a = tokio::spawn(http_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let b = http_get(client, addr, "/fast").await.unwrap();

	assert_eq!(b, StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(a.await.unwrap().unwrap(), StatusCode::OK);
}

#[tokio::test]
async fn http2_max_concurrent_streams_serializes_on_one_connection() {
	let (_mock, _t, addr) = gateway_with_http_limit(
		json!({
			"http2MaxConcurrentStreams": 1,
		}),
		Duration::from_millis(400),
	)
	.await;
	let client = http2_client();

	let a = tokio::spawn(http2_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let start = std::time::Instant::now();
	let b = http2_get(client, addr, "/fast").await.unwrap();
	let waited = start.elapsed();

	let a = a.await.unwrap().unwrap();
	assert_eq!(a, StatusCode::OK);
	assert_eq!(b, StatusCode::OK);
	assert!(
		waited >= Duration::from_millis(250),
		"second H2 stream must wait for SETTINGS_MAX_CONCURRENT_STREAMS=1; waited {waited:?}"
	);
}

#[tokio::test]
async fn http2_request_budget_rejects_extra_stream_with_503() {
	let (_mock, _t, addr) = gateway_with_http_limit(
		json!({
			"maxConcurrentRequests": 1,
			"maxPendingRequests": 0,
			"maxRequestWait": "0s",
		}),
		Duration::from_millis(800),
	)
	.await;
	let client = http2_client();

	let a = tokio::spawn(http2_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let b = http2_get(client, addr, "/fast").await.unwrap();

	assert_eq!(b, StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(a.await.unwrap().unwrap(), StatusCode::OK);
}
