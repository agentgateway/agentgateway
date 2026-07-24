use agentgateway::test_helpers::oteltracemock;
use ppp::v2::{
	Builder as ProxyV2Builder, Command as ProxyV2Command, Protocol as ProxyV2Protocol,
	Version as ProxyV2Version,
};
use tokio::net::TcpListener;

use crate::common::prelude::*;

fn build_proxy_v1_header(src: &str, dst: &str) -> Vec<u8> {
	let src: std::net::SocketAddrV4 = src.parse().unwrap();
	let dst: std::net::SocketAddrV4 = dst.parse().unwrap();
	format!(
		"PROXY TCP4 {} {} {} {}\r\n",
		src.ip(),
		dst.ip(),
		src.port(),
		dst.port()
	)
	.into_bytes()
}

fn build_proxy_v2_header(src: &str, dst: &str) -> Vec<u8> {
	let src: std::net::SocketAddrV4 = src.parse().unwrap();
	let dst: std::net::SocketAddrV4 = dst.parse().unwrap();
	let addresses = ppp::v2::Addresses::IPv4(ppp::v2::IPv4 {
		source_address: *src.ip(),
		destination_address: *dst.ip(),
		source_port: src.port(),
		destination_port: dst.port(),
	});
	ProxyV2Builder::with_addresses(
		ProxyV2Version::Two | ProxyV2Command::Proxy,
		ProxyV2Protocol::Stream,
		addresses,
	)
	.build()
	.unwrap()
}

async fn raw_header_backend() -> (std::net::SocketAddr, oneshot::Receiver<String>) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let (tx, rx) = oneshot::channel();
	tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let mut buf = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let n = stream.read(&mut chunk).await.unwrap();
			assert!(
				n > 0,
				"raw header backend connection closed before request headers"
			);
			buf.extend_from_slice(&chunk[..n]);
			if buf.windows(4).any(|w| w == b"\r\n\r\n") {
				break;
			}
		}
		let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
		tx.send(String::from_utf8(buf[..header_end].to_vec()).unwrap())
			.unwrap();
		stream
			.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
			.await
			.unwrap();
	});
	(addr, rx)
}

#[tokio::test]
async fn basic_handling() {
	let (_mock, _bind, io) = basic_setup().await;
	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.version, Version::HTTP_11);
	assert_eq!(body.method, Method::POST);
}

#[tokio::test]
async fn http_header_case_preserve_forwards_original_case_to_backend() {
	let (backend_addr, captured_request) = raw_header_backend().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(backend_addr)
		.with_bind(simple_bind())
		.with_route(basic_route(backend_addr));
	t.attach_frontend_policy(json!({
		"http": {
			"http1HeaderCase": "preserve",
		},
	}))
	.await;

	let mut io = t.serve(BIND_KEY);
	io.write_all(
		b"GET / HTTP/1.1\r\nHost: lo\r\nX-Case-Probe: preserve-me\r\nConnection: close\r\n\r\n",
	)
	.await
	.unwrap();

	let captured_request = tokio::time::timeout(Duration::from_secs(5), captured_request)
		.await
		.unwrap()
		.unwrap();
	assert!(
		captured_request.contains("\r\nX-Case-Probe: preserve-me\r\n"),
		"backend request did not preserve header case:\n{captured_request}"
	);
	assert!(
		!captured_request.contains("\r\nx-case-probe: preserve-me\r\n"),
		"backend request lowercased preserved header:\n{captured_request}"
	);
}

#[tokio::test]
async fn proxy_policy_optional_mode_allows_plain_http() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	t.attach_frontend_policy(json!({
		"proxyProtocol": {
			"version": "all",
			"mode": "optional",
		},
	}))
	.await;

	let io = t.serve_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn proxy_policy_v1_accepts_v1_header() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	t.attach_frontend_policy(json!({
		"proxyProtocol": {
			"version": "v1",
		},
	}))
	.await;

	let mut io = t.serve_tunnel(BIND_KEY);
	io.write_all(&build_proxy_v1_header("192.168.1.10:40000", "127.0.0.1:80"))
		.await
		.unwrap();
	let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
		.await
		.unwrap();
	let conn = tokio::spawn(conn);
	let res = sender
		.send_request(
			::http::Request::builder()
				.method(Method::GET)
				.uri("/")
				.header(header::HOST, "lo")
				.header(header::CONNECTION, "close")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	conn.abort();
}

#[tokio::test]
async fn proxy_policy_v1_rejects_v2_header() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	t.attach_frontend_policy(json!({
		"proxyProtocol": {
			"version": "v1",
		},
	}))
	.await;

	let mut io = t.serve_tunnel(BIND_KEY);
	io.write_all(&build_proxy_v2_header("192.168.1.10:40000", "127.0.0.1:80"))
		.await
		.unwrap();
	io.write_all(b"GET / HTTP/1.1\r\nHost: lo\r\n\r\n")
		.await
		.unwrap();
	io.shutdown().await.unwrap();

	let mut response = [0u8; 1];
	let read = tokio::time::timeout(Duration::from_secs(2), io.read(&mut response))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(read, 0);
}

#[tokio::test]
async fn tracing_exports_to_otel_trace_mock() {
	unsafe {
		// Drop export time to make tests fast
		std::env::set_var("OTEL_BLRP_SCHEDULE_DELAY", "20");
		std::env::set_var("OTEL_BSP_SCHEDULE_DELAY", "20");
	}
	struct CountingTraceHandler {
		exports: Arc<AtomicUsize>,
	}

	#[async_trait::async_trait]
	impl oteltracemock::Handler for CountingTraceHandler {
		async fn export(
			&mut self,
			_request: &opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest,
		) -> Result<
			opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse,
			tonic::Status,
		> {
			self.exports.fetch_add(1, Ordering::SeqCst);
			oteltracemock::ok_response()
		}
	}

	let exports = Arc::new(AtomicUsize::new(0));
	let otel = oteltracemock::OtelTraceMock::new({
		let exports = Arc::clone(&exports);
		move || CountingTraceHandler {
			exports: Arc::clone(&exports),
		}
	})
	.spawn()
	.await;

	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_frontend_policy(json!({
			"tracing": {
				"host": otel.address.to_string(),
				"randomSampling": true
			}
		}))
		.await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	tokio::time::timeout(Duration::from_secs(2), async {
		while exports.load(Ordering::SeqCst) == 0 {
			tokio::task::yield_now().await;
		}
	})
	.await
	.unwrap();

	assert_eq!(exports.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn multiple_requests() {
	let (_mock, _bind, io) = basic_setup().await;
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn debug_trace_only_captures_one_request_on_keepalive_connection() {
	let (_mock, _bind, io) = basic_setup().await;
	// Watchers are process-global, so use a path disjoint from concurrent debug trace tests.
	let mut trace_rx = agentgateway::proxy::dtrace::track_expression(Some(
		agentgateway::cel::Expression::new_strict("request.path == '/keepalive'").unwrap(),
	));

	let res = send_request(io.clone(), Method::GET, "http://lo/keepalive").await;
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	let res = send_request(io.clone(), Method::GET, "http://lo/keepalive").await;
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	let mut events = Vec::new();
	while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), trace_rx.recv()).await {
		events.push(serde_json::to_value(msg).unwrap())
	}

	let request_started = events
		.iter()
		.filter(|event| event["message"]["type"] == "requestStarted")
		.count();
	assert_eq!(request_started, 1, "{events:#?}");
}

#[tokio::test]
async fn debug_trace_expression_watchers_match_first_request() {
	let (_mock, _bind, io) = basic_setup().await;
	let mut first_trace_rx = agentgateway::proxy::dtrace::track_expression(Some(
		agentgateway::cel::Expression::new_strict("request.path == '/first'").unwrap(),
	));
	let mut second_trace_rx = agentgateway::proxy::dtrace::track_expression(Some(
		agentgateway::cel::Expression::new_strict("request.path == '/second'").unwrap(),
	));

	let res = send_request(io.clone(), Method::GET, "http://lo/second").await;
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	assert!(
		tokio::time::timeout(Duration::from_millis(50), first_trace_rx.recv())
			.await
			.is_err(),
		"first watcher should remain queued when its expression does not match",
	);
	let second_event = tokio::time::timeout(Duration::from_secs(1), second_trace_rx.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		serde_json::to_value(second_event).unwrap()["message"]["type"],
		"requestStarted"
	);

	let res = send_request(io.clone(), Method::GET, "http://lo/first").await;
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	let first_event = tokio::time::timeout(Duration::from_secs(1), first_trace_rx.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		serde_json::to_value(first_event).unwrap()["message"]["type"],
		"requestStarted"
	);
}

#[tokio::test]
async fn basic_http2() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	let io = t.serve_http2(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://lo")
		.version(Version::HTTP_2)
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);
}

async fn grpc_trailer_backend(status: &'static str) -> std::net::SocketAddr {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	tokio::spawn(async move {
		loop {
			let Ok((stream, _)) = listener.accept().await else {
				return;
			};
			tokio::spawn(async move {
				let svc = service_fn(move |_| async move {
					let mut trailers = HeaderMap::new();
					trailers.insert("grpc-status", status.parse().unwrap());
					let body = StreamBody::new(tokio_stream::iter([
						Ok::<_, Infallible>(Frame::data(bytes::Bytes::new())),
						Ok(Frame::trailers(trailers)),
					]));
					Ok::<_, Infallible>(
						::http::Response::builder()
							.status(200)
							.header(header::CONTENT_TYPE, "application/grpc")
							.body(body)
							.unwrap(),
					)
				});
				let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
					.serve_connection(TokioIo::new(stream), svc)
					.await;
			});
		}
	});
	addr
}

#[tokio::test]
async fn grpc_status_trailer_is_available_to_access_log_cel() {
	let backend = grpc_trailer_backend("13").await;
	let path = format!("/grpc-{}", rand::rng().random::<u128>());
	let t = setup_proxy_test(
		r#"{"config":{"logging":{"fields":{"add":{"cel_grpc_status":"response.grpcStatus"}}}}}"#,
	)
	.unwrap()
	.with_raw_backend(BackendWithPolicies {
		backend: Backend::Opaque(
			ResourceName::new(strng::format!("{}", backend), "".into()),
			Target::Address(backend),
		),
		inline_policies: vec![BackendTrafficPolicy::HTTP(backend::HTTP {
			version: Some(Version::HTTP_2),
			..Default::default()
		})],
	})
	.with_bind(simple_bind())
	.with_route(basic_route(backend));
	let io = t.serve_http2(strng::new("bind"));
	let res = RequestBuilder::new(Method::POST, &format!("http://lo{path}"))
		.version(Version::HTTP_2)
		.header(header::CONTENT_TYPE, "application/grpc")
		.body(Body::empty())
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	assert_eq!(log["grpc.status"].as_u64(), Some(13));
	assert_eq!(log["cel_grpc_status"].as_u64(), Some(13));
}

#[tokio::test]
async fn basic_tcp() {
	let mock = simple_mock().await;
	let (_mock, _bind, io) = setup_tcp_mock(mock);
	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::POST);
}
