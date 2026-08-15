use std::net::SocketAddr;

use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::prelude::*;

fn http_client() -> Client<HttpConnector, Body> {
	Client::builder(TokioExecutor::new())
		.timer(TokioTimer::new())
		.pool_max_idle_per_host(0)
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

async fn gateway_with_tcp_limit(
	max_connections: u32,
	max_pending: u32,
	wait: &str,
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
	t.attach_frontend_policy(json!({
		"tcp": {
			"maxConnections": max_connections,
			"maxPendingConnections": max_pending,
			"maxConnectionWait": wait,
		},
	}))
	.await;
	let addr = t.serve_real_listener(BIND_KEY).await;
	(mock, t, addr)
}

#[tokio::test]
async fn max_connections_second_client_waits_then_succeeds() {
	let (_mock, _t, addr) = gateway_with_tcp_limit(1, 4, "5s", Duration::from_millis(400)).await;
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
		"fast request must wait for the occupied slot, not race past it; waited {waited:?}"
	);
	assert!(
		waited < Duration::from_secs(2),
		"waiter should proceed once the slot is free, waited {waited:?}"
	);
	assert_ne!(b, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn max_connections_queue_full_closes_without_503() {
	let (_mock, _t, addr) = gateway_with_tcp_limit(1, 0, "5s", Duration::from_millis(800)).await;
	let client = http_client();

	let a = tokio::spawn(http_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let b = http_get(client, addr, "/fast").await;

	assert!(
		b.is_err(),
		"overflow should drop the socket without an HTTP 503, got {b:?}"
	);
	assert_eq!(a.await.unwrap().unwrap(), StatusCode::OK);
}

#[tokio::test]
async fn max_connections_wait_timeout_closes_without_503() {
	let (_mock, _t, addr) = gateway_with_tcp_limit(1, 4, "100ms", Duration::from_millis(800)).await;
	let client = http_client();

	let a = tokio::spawn(http_get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(80)).await;
	let b = http_get(client, addr, "/fast").await;

	assert!(
		b.is_err(),
		"wait timeout should drop the socket without an HTTP 503, got {b:?}"
	);
	assert_eq!(a.await.unwrap().unwrap(), StatusCode::OK);
}

#[tokio::test]
async fn max_connections_flood_does_not_exceed_cap() {
	let (_mock, _t, addr) = gateway_with_tcp_limit(2, 0, "5s", Duration::from_millis(600)).await;
	let client = http_client();
	let mut joins = Vec::new();
	for _ in 0..40 {
		joins.push(tokio::spawn(http_get(client.clone(), addr, "/slow")));
	}
	let mut ok = 0;
	let mut err = 0;
	let mut unexpected = 0;
	for j in joins {
		match j.await.unwrap() {
			Ok(StatusCode::OK) => ok += 1,
			Ok(_) => unexpected += 1,
			Err(_) => err += 1,
		}
	}
	// Exact counts would be timing-dependent: a slot freed early lets another request through.
	// What must hold is that the cap is never exceeded at any instant and overflow is dropped
	// rather than answered.
	assert_eq!(unexpected, 0, "overflow must not be HTTP 503");
	assert!(
		(1..=2).contains(&ok),
		"at most maxConnections requests may complete, got {ok}"
	);
	assert_eq!(ok + err, 40, "every request must be answered or dropped");
}
