use std::net::SocketAddr;

use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::prelude::*;

fn http2_client() -> Client<HttpConnector, Body> {
	Client::builder(TokioExecutor::new())
		.timer(TokioTimer::new())
		.http2_only(true)
		.build_http()
}

async fn get(client: Client<HttpConnector, Body>, addr: SocketAddr, path: &str) -> StatusCode {
	let url = format!("http://127.0.0.1:{}{path}", addr.port());
	RequestBuilder::new(Method::GET, &url)
		.version(Version::HTTP_2)
		.send(client)
		.await
		.unwrap()
		.status()
}

async fn gateway(
	http: Option<serde_json::Value>,
	tcp: Option<serde_json::Value>,
) -> (MockServer, TestBind, SocketAddr) {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path("/slow"))
		.respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
		.mount(&mock)
		.await;
	Mock::given(wiremock::matchers::path("/fast"))
		.respond_with(ResponseTemplate::new(200))
		.mount(&mock)
		.await;

	let mut test = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	let mut policy = serde_json::Map::new();
	if let Some(http) = http {
		policy.insert("http".into(), http);
	}
	if let Some(tcp) = tcp {
		policy.insert("tcp".into(), tcp);
	}
	test.attach_frontend_policy(policy.into()).await;
	let addr = test.serve_real_listener(BIND_KEY).await;
	(mock, test, addr)
}

#[tokio::test]
async fn request_limit_rejects_an_extra_http2_stream() {
	let (_mock, _test, addr) = gateway(Some(json!({ "maxConcurrentRequests": 1 })), None).await;
	let client = http2_client();

	let slow = tokio::spawn(get(client.clone(), addr, "/slow"));
	tokio::time::sleep(Duration::from_millis(50)).await;
	assert_eq!(
		get(client, addr, "/fast").await,
		StatusCode::SERVICE_UNAVAILABLE
	);
	assert_eq!(slow.await.unwrap(), StatusCode::OK);
}

#[tokio::test]
async fn connection_limit_closes_overflow_and_recovers_capacity() {
	let (_mock, test, _addr) = gateway(None, Some(json!({ "maxConnections": 1 }))).await;
	let addr = test.serve_gateway_listener(BIND_KEY).await;
	let first = TcpStream::connect(addr).await.unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;

	let mut overflow = TcpStream::connect(addr).await.unwrap();
	let write = overflow
		.write_all(b"GET /fast HTTP/1.1\r\nHost: localhost\r\n\r\n")
		.await;
	if write.is_ok() {
		let mut byte = [0];
		let read = tokio::time::timeout(Duration::from_secs(1), overflow.read(&mut byte))
			.await
			.expect("overflow connection must be closed promptly");
		assert!(matches!(read, Ok(0) | Err(_)));
	}

	drop(first);
	tokio::time::sleep(Duration::from_millis(50)).await;
	let mut admitted = TcpStream::connect(addr).await.unwrap();
	admitted
		.write_all(b"GET /fast HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
		.await
		.unwrap();
	let mut response = Vec::new();
	tokio::time::timeout(Duration::from_secs(2), admitted.read_to_end(&mut response))
		.await
		.unwrap()
		.unwrap();
	assert!(response.starts_with(b"HTTP/1.1 200"));
}
