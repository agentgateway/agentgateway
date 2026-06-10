use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use ::http::{HeaderMap, Method, StatusCode, Version, header};
use agent_core::strng;
use assert_matches::assert_matches;
use http_body::Frame;
use http_body_util::{BodyExt, StreamBody};
use hyper::client::conn::http1;
use hyper::service::service_fn;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use ppp::v2::{
	Builder as ProxyV2Builder, Command as ProxyV2Command, Protocol as ProxyV2Protocol,
	Version as ProxyV2Version,
};
use rand::RngExt;
use rustls_pki_types::ServerName;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsConnector;
use url::{Position, Url};
use wiremock::{Mock, MockServer, ResponseTemplate};
use x509_parser::nom::AsBytes;

use crate::http::tests_common::*;
use crate::http::{Body, Response, ext_proc};
use crate::llm::{AIProvider, custom, openai};
use crate::proxy::request_builder::RequestBuilder;
use crate::test_helpers::proxymock::*;
use crate::test_helpers::{extauthmock, oteltracemock, ratelimitmock};
use crate::types::agent::{
	Backend, BackendTarget, BackendTrafficPolicy, BackendWithPolicies, Bind, BindProtocol,
	FrontendPolicy, Listener, ListenerProtocol, ListenerSet, ListenerTarget, PathMatch,
	PolicyInheritance, PolicyTarget, ResourceName, Route, RouteMatch, SimpleBackendReference, Target,
	TargetedPolicy,
};
use crate::types::{backend, frontend};
use crate::{read_body, *};

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgltxBTVDLg7C6vE1T
7OtwJIZ/dpm8ygE2MBTjPCY3hgahRANCAARYzu50EeBrT0rELmTGroaGtn0zdjxL
1lOGr9fGw5wOGcXO0+Gn5F5sIxGyTM0FwnUHFNz2SoixZR5dtxhNc+Lo
-----END PRIVATE KEY-----
";
const TEST_KEY_ID: &str = "kid-1";
const TEST_ISSUER: &str = "https://issuer.example.com";
const TEST_CLIENT_ID: &str = "client-id";

#[derive(Serialize)]
struct TestIdTokenClaims<'a> {
	iss: &'a str,
	aud: &'a str,
	exp: u64,
	nonce: &'a str,
	sub: &'a str,
}

fn test_oidc_cookie_encoder() -> crate::http::sessionpersistence::Encoder {
	crate::http::sessionpersistence::Encoder::aes(
		"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
	)
	.expect("aes encoder")
}

fn setup_proxy_test_with_oidc() -> TestBind {
	let mut config = crate::config::parse_config("{}".to_string(), None).expect("config");
	config.oidc_cookie_encoder = Some(test_oidc_cookie_encoder());
	setup_proxy_test_with_config(config)
}

fn test_jwks() -> JwkSet {
	serde_json::from_value(json!({
		"keys": [{
			"use": "sig",
			"kty": "EC",
			"kid": TEST_KEY_ID,
			"crv": "P-256",
			"alg": "ES256",
			"x": "WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk",
			"y": "xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"
		}]
	}))
	.expect("jwks json")
}

fn signed_id_token(nonce: &str) -> String {
	jsonwebtoken::encode(
		&Header {
			alg: Algorithm::ES256,
			kid: Some(TEST_KEY_ID.into()),
			..Header::default()
		},
		&TestIdTokenClaims {
			iss: TEST_ISSUER,
			aud: TEST_CLIENT_ID,
			exp: crate::http::oidc::now_unix() + 300,
			nonce,
			sub: "user-1",
		},
		&EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("encoding key"),
	)
	.expect("signed id token")
}

fn gateway_oidc_policy(token_endpoint: impl Into<String>) -> Value {
	json!({
		"oidc": {
			"issuer": TEST_ISSUER,
			"authorizationEndpoint": format!("{TEST_ISSUER}/authorize"),
			"tokenEndpoint": token_endpoint.into(),
			"jwks": serde_json::to_string(&test_jwks()).expect("jwks"),
			"clientId": TEST_CLIENT_ID,
			"clientSecret": "client-secret",
			"redirectURI": "http://lo/oauth/callback"
		}
	})
}

fn route_with_prefix(target: std::net::SocketAddr, prefix: &str) -> Route {
	let mut route = basic_route(target);
	route.matches = vec![RouteMatch {
		headers: vec![],
		path: PathMatch::PathPrefix(prefix.into()),
		method: None,
		query: vec![],
	}];
	route
}

fn https_bind() -> Bind {
	Bind {
		key: BIND_KEY,
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: strng::new("*.example.com"),
			protocol: ListenerProtocol::HTTPS(
				types::local::LocalTLSServerConfig {
					mode: Default::default(),
					cert: "../../examples/tls/certs/cert.pem".into(),
					key: "../../examples/tls/certs/key.pem".into(),
					root: None,
					cipher_suites: None,
					min_tls_version: None,
					max_tls_version: None,
					key_exchange_groups: None,
				}
				.try_into()
				.unwrap(),
			),
		}]),
		protocol: BindProtocol::tls,
		tunnel_protocol: Default::default(),
	}
}

async fn serve_https_http1_connection(
	t: &TestBind,
	sni: &str,
) -> (
	http1::SendRequest<Body>,
	tokio::task::JoinHandle<Result<(), hyper::Error>>,
) {
	let io = t.serve(BIND_KEY);
	let tls: crate::http::backendtls::BackendTLS = crate::http::backendtls::ResolvedBackendTLS {
		cert: None,
		key: None,
		root: Some(include_bytes!("../../../../examples/tls/certs/ca-cert.pem").to_vec()),
		hostname: Some(sni.to_string()),
		insecure: false,
		insecure_host: true,
		alpn: None,
		subject_alt_names: None,
		key_exchange_groups: None,
	}
	.try_into()
	.unwrap();
	let tls = TlsConnector::from(tls.base_config().config)
		.connect(ServerName::try_from(sni.to_string()).unwrap(), io)
		.await
		.unwrap();
	let (sender, conn) = http1::handshake(TokioIo::new(tls)).await.unwrap();
	let conn = tokio::spawn(conn);
	(sender, conn)
}

fn find_set_cookie_pair(headers: &::http::HeaderMap, prefix: &str) -> String {
	headers
		.get_all(header::SET_COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.find_map(|value| {
			let cookie = cookie::Cookie::parse(value.to_string()).ok()?;
			cookie
				.name()
				.starts_with(prefix)
				.then(|| format!("{}={}", cookie.name(), cookie.value()))
		})
		.unwrap_or_else(|| panic!("missing set-cookie with prefix {prefix}"))
}

fn query_param(uri: &str, name: &str) -> String {
	Url::parse(uri)
		.expect("absolute url")
		.query_pairs()
		.find_map(|(key, value)| (key == name).then(|| value.into_owned()))
		.unwrap_or_else(|| panic!("missing query param {name}"))
}

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

async fn oidc_backend_mock() -> (MockServer, Arc<StdMutex<Option<String>>>) {
	let token_response = Arc::new(StdMutex::new(None));
	let mock = MockServer::start().await;
	let token_response_clone = Arc::clone(&token_response);
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |req: &wiremock::Request| {
			if req.method == Method::POST && req.url.path() == "/token" {
				let id_token = token_response_clone
					.lock()
					.expect("token mutex")
					.clone()
					.expect("token response configured");
				return ResponseTemplate::new(200).set_body_json(json!({
					"id_token": id_token,
				}));
			}

			let request = RequestDump {
				method: req.method.clone(),
				uri: req.url.to_string().parse().expect("request uri"),
				headers: req.headers.clone(),
				body: bytes::Bytes::copy_from_slice(&req.body),
				version: req.version,
			};
			ResponseTemplate::new(200).set_body_json(request)
		})
		.mount(&mock)
		.await;
	(mock, token_response)
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
	let (mut sender, conn) = http1::handshake(TokioIo::new(io)).await.unwrap();
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
	let mut trace_rx = crate::proxy::dtrace::track_expression(None);

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	read_body_raw(res.into_body()).await;

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
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
	let mut first_trace_rx = crate::proxy::dtrace::track_expression(Some(
		crate::cel::Expression::new_strict("request.path == '/first'").unwrap(),
	));
	let mut second_trace_rx = crate::proxy::dtrace::track_expression(Some(
		crate::cel::Expression::new_strict("request.path == '/second'").unwrap(),
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
async fn reserved_oidc_cookies_are_stripped_before_proxying() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));
	let io = t.serve_http(BIND_KEY);

	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo",
		&[(
			"cookie",
			"agw_oidc_s_test=session; app_cookie=keep; agw_oidc_t_test=txn",
		)],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	let cookie = body
		.headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();
	assert!(cookie.contains("app_cookie=keep"));
	assert!(!cookie.contains("agw_oidc_s_test"));
	assert!(!cookie.contains("agw_oidc_t_test"));
}

#[tokio::test]
async fn gateway_phase_oidc_redirects_before_route_selection() {
	let (mock, _token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let io = bind.serve_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://lo/private").await;

	assert_eq!(res.status(), 302);
	let location = res.hdr(header::LOCATION);
	assert!(location.starts_with("https://issuer.example.com/authorize?"));
	assert!(location.contains("redirect_uri=http%3A%2F%2Flo%2Foauth%2Fcallback"));
}

#[tokio::test]
async fn gateway_phase_oidc_callback_authenticates_and_strips_reserved_cookies() {
	let (mock, token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let oidc = bind
		.pi
		.stores
		.read_binds()
		.gateway_policies(&crate::types::agent::ListenerName::default())
		.oidc
		.iter()
		.next()
		.cloned()
		.expect("compiled gateway oidc policy")
		.pol;

	let io = bind.serve_http(BIND_KEY);
	let login = send_request(io.clone(), Method::GET, "http://lo/private").await;
	assert_eq!(login.status(), 302);

	let state = query_param(login.hdr(header::LOCATION), "state");
	let transaction_cookie = login
		.headers()
		.get(header::SET_COOKIE)
		.and_then(|value| value.to_str().ok())
		.expect("transaction set-cookie");
	let transaction_cookie =
		cookie::Cookie::parse(transaction_cookie.to_string()).expect("transaction cookie");
	let transaction = oidc
		.session
		.decode_transaction(transaction_cookie.value())
		.expect("decode transaction cookie");
	*token_response.lock().expect("token mutex") = Some(signed_id_token(&transaction.nonce));

	let callback = send_request_headers(
		io.clone(),
		Method::GET,
		&format!("http://lo/oauth/callback?code=auth-code&state={state}"),
		&[(
			"cookie",
			&format!(
				"{}={}",
				transaction_cookie.name(),
				transaction_cookie.value()
			),
		)],
	)
	.await;
	assert_eq!(callback.status(), 302);
	assert_eq!(callback.hdr(header::LOCATION), "/private");

	let session_cookie = find_set_cookie_pair(callback.headers(), "agw_oidc_s_");
	let res = send_request_headers(
		io,
		Method::GET,
		"http://lo/upstream",
		&[("cookie", &format!("{session_cookie}; app_cookie=keep"))],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	let cookie = body
		.headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();
	assert!(cookie.contains("app_cookie=keep"));
	assert!(!cookie.contains("agw_oidc_s_"));
	assert!(!cookie.contains("agw_oidc_t_"));
}

#[tokio::test]
async fn gateway_phase_oidc_bypasses_cors_preflight_requests() {
	let (mock, _token_response) = oidc_backend_mock().await;
	let mut bind = setup_proxy_test_with_oidc()
		.with_backend(*mock.address())
		.with_bind(simple_bind())
		.with_route(route_with_prefix(*mock.address(), "/upstream"));
	bind
		.attach_gateway_policy(gateway_oidc_policy(format!("{}/token", mock.uri())))
		.await;

	let io = bind.serve_http(BIND_KEY);
	let res = send_request_headers(
		io,
		Method::OPTIONS,
		"http://lo/upstream",
		&[
			("origin", "https://frontend.example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::OPTIONS);
}

#[tokio::test]
async fn gateway_phase_cors_handles_preflight_before_route_selection() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
		}))
		.await;

	let res = send_request_headers(
		io,
		Method::OPTIONS,
		"http://lo/no-route-needed",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;

	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

#[tokio::test]
async fn network_authorization_allow() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_frontend_policy(json!({
			"networkAuthorization": {
				"rules": ["source.port == 12345"], // NOTE: the tests hardcode a dummy src port that matches
			},
		}))
		.await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn network_authorization_deny() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_frontend_policy(json!({
			"networkAuthorization": {
				"rules": ["source.port == 54321"], // NOTE: the tests hardcode a dummy src port that does not match
			},
		}))
		.await;

	RequestBuilder::new(Method::GET, "http://lo")
		.send(io)
		.await
		.expect_err("should be denied");
}

#[tokio::test]
async fn local_ratelimit() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "1s",
			}],
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// the rate limit is exhausted, because CORS runs before authentication and rate limiting.
#[tokio::test]
async fn cors_preflight_bypasses_ratelimit() {
	let (_mock, mut bind, io) = basic_setup().await;

	// Attach CORS + rate limit (1 token, essentially immediately exhausted after first real request)
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "100s",
			}],
		}))
		.await;

	// First real request exhausts the single token
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	// Second real request should be rate limited
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);

	// A CORS preflight should still succeed (200) even though rate limit is exhausted
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that when a cross-origin request is rate limited (429), the response
/// still carries the CORS headers so browsers can read the error.
#[tokio::test]
async fn cors_headers_present_on_ratelimited_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"localRateLimit": [{
				"maxTokens": 1,
				"tokensPerFill": 1,
				"fillInterval": "100s",
			}],
		}))
		.await;

	// Exhaust rate limit with a normal cross-origin GET
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");

	// Second cross-origin request is rate limited, but should still have CORS headers
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 429);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present even on rate-limited responses"
	);
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// API key authentication is required, because CORS runs before authentication
/// and authorization.
#[tokio::test]
async fn cors_preflight_bypasses_api_key_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
				}],
				"mode": "strict",
			},
		}))
		.await;

	// Request without credentials should be rejected
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);

	// CORS preflight should succeed without any credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// basic authentication is required, because CORS runs before authentication
/// and authorization.
#[tokio::test]
async fn cors_preflight_bypasses_basic_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"basicAuth": {
				"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00",
				"realm": "my-realm",
				"mode": "strict",
			},
		}))
		.await;

	// Request without credentials should be rejected
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);

	// CORS preflight should succeed without any credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

#[tokio::test]
async fn mcp_authentication_runs_in_route_policy_path() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
      .attach_route_policy(json!({
			"mcpAuthentication": {
				"issuer": "https://example.com",
				"audiences": ["test-aud"],
				"jwks": "{\"keys\":[{\"use\":\"sig\",\"kty\":\"EC\",\"kid\":\"XhO06x8JjWH1wwkWkyeEUxsooGEWoEdidEpwyd_hmuI\",\"crv\":\"P-256\",\"alg\":\"ES256\",\"x\":\"XZHF8Em5LbpqfgewAalpSEH4Ka2I2xjcxxUt2j6-lCo\",\"y\":\"g3DFz45A7EOUMgmsNXatrXw1t-PG5xsbkxUs851RxSE\"}]}",
				"resourceMetadata": {
					"mcpResourceUri": "mcp://test"
				}
			}
		}))
      .await;

	let res = send_request(
		io,
		Method::GET,
		"http://lo/.well-known/oauth-protected-resource/mcp",
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("content-type"), "application/json");
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// authorization rules would reject the request, because CORS runs before
/// authorization.
#[tokio::test]
async fn cors_preflight_bypasses_authorization() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
					"metadata": {"group": "eng"},
				}],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'admin'"],
			},
		}))
		.await;

	// Authenticated request should be rejected by authorization (403)
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-123")],
	)
	.await;
	assert_eq!(res.status(), 403);

	// CORS preflight should still succeed without credentials
	let res = send_request_headers(
		io.clone(),
		Method::OPTIONS,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("access-control-request-method", "GET"),
		],
	)
	.await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("access-control-allow-origin"), "http://example.com");
}

/// Verifies that when authentication or authorization rejects a cross-origin
/// request, the response still carries CORS headers so browsers can read the
/// error body.
#[tokio::test]
async fn cors_headers_present_on_auth_rejected_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"cors": {
				"allowCredentials": false,
				"allowHeaders": ["*"],
				"allowMethods": ["GET", "POST"],
				"allowOrigins": ["http://example.com"],
				"exposeHeaders": [],
			},
			"apiKey": {
				"keys": [{
					"key": "sk-123",
					"metadata": {"group": "eng"},
				}],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'admin'"],
			},
		}))
		.await;

	// 401: missing credentials, CORS headers should still be present
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("origin", "http://example.com")],
	)
	.await;
	assert_eq!(res.status(), 401);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present on 401 responses"
	);

	// 403: valid key but fails authorization, CORS headers should still be present
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[
			("origin", "http://example.com"),
			("authorization", "bearer sk-123"),
		],
	)
	.await;
	assert_eq!(res.status(), 403);
	assert_eq!(
		res.hdr("access-control-allow-origin"),
		"http://example.com",
		"CORS headers must be present on 403 responses"
	);
}

#[tokio::test]
async fn llm_openai() {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		false,
		"{}",
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert_llm(
		io,
		include_bytes!("../llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
}

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		true,
		"{}",
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23
	});
	assert_llm(
		io,
		include_bytes!("../llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
}

#[tokio::test]
async fn llm_custom_rerank() {
	let mock = body_mock(include_bytes!("../llm/tests/response/cohere/rerank.json")).await;
	let provider = crate::types::local::LocalNamedAIProvider {
		name: "default".into(),
		provider: AIProvider::Custom(custom::Provider {
			model: None,
			formats: vec![custom::ProviderFormatConfig {
				format: custom::ProviderFormat::Rerank,
				path: None,
			}],
		}),
		host_override: Some(Target::Address(*mock.address())),
		path_override: None,
		path_prefix: None,
		tokenize: false,
		policies: serde_json::from_value(json!({
			"ai": {"routes": {"/v1/rerank": "rerank"}}
		}))
		.unwrap(),
	};
	let (mock, _bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/rerank",
		include_bytes!("../llm/tests/requests/rerank/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let body: Value =
		serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(body["results"][0]["index"], 2);
	assert_eq!(body["results"][0]["relevance_score"], 0.91);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(
		upstream_body["query"],
		"What is the capital of the United States?"
	);
	assert_eq!(upstream_body["documents"].as_array().unwrap().len(), 3);
}

fn setup_custom_llm_provider_backend_mock(
	mock: MockServer,
	supported_formats: Vec<custom::ProviderFormat>,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	setup_custom_llm_provider_backend_mock_with_formats(
		mock,
		supported_formats
			.into_iter()
			.map(|format| custom::ProviderFormatConfig { format, path: None })
			.collect(),
	)
}

fn setup_custom_llm_provider_backend_mock_with_formats(
	mock: MockServer,
	formats: Vec<custom::ProviderFormatConfig>,
) -> (MockServer, TestBind, Client<MemoryConnector, Body>) {
	let backend_name = "custom-ai";
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(simple_bind())
		.with_raw_backend(custom_llm_backend_with_formats(
			backend_name,
			SimpleBackendReference::InlineBackend(Target::Address(*mock.address())),
			formats,
		))
		.with_route(basic_named_route(strng::format!("/{backend_name}")));
	let io = t.serve_http(BIND_KEY);
	(mock, t, io)
}

#[tokio::test]
async fn llm_custom_provider_routes_to_provider_backend() {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Completions]);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let _ = res.into_body().collect().await.unwrap();

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/v1/chat/completions"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["model"], "replaceme");
}

#[tokio::test]
async fn llm_custom_provider_uses_native_format_fallback() {
	let mock = body_mock(include_bytes!("../llm/tests/response/anthropic/basic.json")).await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Messages]);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let response_body: Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("response is JSON");
	assert_eq!(response_body["object"], "chat.completion");
	assert_eq!(response_body["usage"]["prompt_tokens"], 15);
	assert_eq!(response_body["usage"]["completion_tokens"], 21);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/v1/messages"
	);
	let upstream_body: Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["system"], "You are a helpful assistant.");
	assert_eq!(upstream_body["messages"][0]["role"], "user");
}

#[tokio::test]
async fn llm_custom_provider_uses_format_path_override() {
	let mock = body_mock(include_bytes!("../llm/tests/response/anthropic/basic.json")).await;
	let (mock, _bind, io) = setup_custom_llm_provider_backend_mock_with_formats(
		mock,
		vec![custom::ProviderFormatConfig {
			format: custom::ProviderFormat::Messages,
			path: Some(strng::literal!("/api/messages")),
		}],
	);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let _ = res.into_body().collect().await.unwrap();

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(
		&requests[0].url[Position::BeforePath..Position::AfterPath],
		"/api/messages"
	);
}

#[tokio::test]
async fn llm_custom_provider_rejects_unsupported_format_before_upstream_call() {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let (mock, _bind, io) =
		setup_custom_llm_provider_backend_mock(mock, vec![custom::ProviderFormat::Embeddings]);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 503);
	let body = res.into_body().collect().await.unwrap().to_bytes();
	assert!(
		String::from_utf8_lossy(&body)
			.contains("unsupported conversion: from Completions to provider custom"),
		"unexpected response body: {}",
		String::from_utf8_lossy(&body)
	);

	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 0);
}

#[derive(Clone)]
struct RecordingRateLimit {
	requests: mpsc::UnboundedSender<crate::http::remoteratelimit::proto::RateLimitRequest>,
}

#[async_trait::async_trait]
impl ratelimitmock::Handler for RecordingRateLimit {
	async fn should_rate_limit(
		&mut self,
		request: &crate::http::remoteratelimit::proto::RateLimitRequest,
	) -> Result<crate::http::remoteratelimit::proto::RateLimitResponse, tonic::Status> {
		self
			.requests
			.send(request.clone())
			.expect("rate limit request receiver should be open");
		ratelimitmock::ok_response()
	}
}

async fn recv_rate_limit_request(
	requests: &mut mpsc::UnboundedReceiver<crate::http::remoteratelimit::proto::RateLimitRequest>,
) -> crate::http::remoteratelimit::proto::RateLimitRequest {
	tokio::time::timeout(Duration::from_secs(1), requests.recv())
		.await
		.expect("timed out waiting for rate limit request")
		.expect("rate limit request sender should be open")
}

fn completions_request_body(streaming: bool) -> Vec<u8> {
	let mut body: Value = serde_json::from_slice(include_bytes!(
		"../llm/tests/requests/completions/basic.json"
	))
	.expect("request fixture should be valid JSON");
	if streaming {
		body["stream"] = json!(true);
	}
	serde_json::to_vec(&body).expect("request fixture should serialize")
}

async fn assert_llm_remote_rate_limit_cost(
	response_body: &[u8],
	request_body: &[u8],
	expected_cost: u64,
) {
	let (rate_limit_tx, mut rate_limit_rx) = mpsc::unbounded_channel();
	let rate_limit = ratelimitmock::RateLimitMock::new({
		let rate_limit_tx = rate_limit_tx.clone();
		move || RecordingRateLimit {
			requests: rate_limit_tx.clone(),
		}
	})
	.spawn()
	.await;

	let mock = body_mock(response_body).await;
	let (_mock, mut bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		false,
		"{}",
	);
	bind
		.attach_route_policy(json!({
			"remoteRateLimit": {
				"domain": "llm",
				"host": rate_limit.address.to_string(),
				"descriptors": [{
					"entries": [{
						"key": "model",
						"value": "\"model\"",
					}],
					"type": "tokens",
					"cost": "llm.outputTokens * uint(1000) + llm.inputTokens",
				}],
			},
		}))
		.await;

	let res = send_request_body(io, Method::POST, "http://lo", request_body).await;
	assert_eq!(res.status(), 200);
	let _ = res.into_body().collect().await.unwrap();

	let initial_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	let amend_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	assert_eq!(initial_request.domain, "llm");
	assert_eq!(amend_request.domain, "llm");

	let initial = initial_request.descriptors.first().unwrap();
	assert_eq!(initial.entries[0].key, "model");
	assert_eq!(initial.entries[0].value, "model");
	assert_eq!(initial.hits_addend, Some(0));

	let amend = amend_request.descriptors.first().unwrap();
	assert_eq!(amend.entries[0].key, "model");
	assert_eq!(amend.entries[0].value, "model");
	assert_eq!(amend.hits_addend, Some(expected_cost));
}

#[tokio::test]
async fn llm_remote_rate_limit_cost_amends_response_tokens() {
	assert_llm_remote_rate_limit_cost(
		include_bytes!("../llm/tests/response/completions/basic.json"),
		&completions_request_body(false),
		23017,
	)
	.await;
}

#[tokio::test]
async fn llm_streaming_remote_rate_limit_cost_amends_response_tokens() {
	assert_llm_remote_rate_limit_cost(
		include_bytes!("../llm/tests/response/completions/stream.json"),
		&completions_request_body(true),
		286018,
	)
	.await;
}

#[rstest::rstest]
#[case::preserves_path(None, None, "/v1/messages?trace=repro")]
#[case::path_override(Some("/custom/chat/completions"), None, "/custom/chat/completions")]
#[case::path_prefix(None, Some("/v1/custom/"), "/v1/custom/chat/completions?trace=repro")]
#[tokio::test]
async fn llm_openai_messages_translation_with_host_override_path_behavior(
	#[case] path_override: Option<&str>,
	#[case] path_prefix: Option<&str>,
	#[case] expected_url: &str,
) {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let provider = crate::test_helpers::proxymock::llm_named_provider(
		&mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		false,
	);
	let provider = crate::types::local::LocalNamedAIProvider {
		path_override: path_override.map(strng::new),
		path_prefix: path_prefix.map(strng::new),
		..provider
	};
	let (mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {
				"routes": {
					"/v1/chat/completions": "completions",
					"/v1/messages": "messages"
				}
			}
		}))
		.await;

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/messages?trace=repro",
		include_bytes!("../llm/tests/requests/messages/basic.json"),
	)
	.await;

	assert_eq!(res.status(), 200);
	let requests = mock
		.received_requests()
		.await
		.expect("request recording should be enabled");
	assert_eq!(requests.len(), 1);
	let upstream = &requests[0];
	assert_eq!(
		&upstream.url[Position::BeforePath..Position::AfterQuery],
		expected_url
	);
}

#[tokio::test]
async fn llm_log_body() {
	let mock = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let x = serde_json::to_string(&json!({
		"config": {
			"logging": {
				"fields": {
					"add": {
						"prompt": "llm.prompt",
						"completion": "llm.completion"
					}
				}
			}
		}
	}))
	.unwrap();
	let (_mock, _bind, io) = setup_llm_mock(
		mock,
		AIProvider::OpenAI(openai::Provider { model: None }),
		true,
		x.as_str(),
	);

	let want = json!({
		"gen_ai.operation.name": "chat",
		"gen_ai.provider.name": "openai",
		"gen_ai.request.model": "replaceme",
		"gen_ai.response.model": "gpt-3.5-turbo-0125",
		"gen_ai.usage.input_tokens": 17,
		"gen_ai.usage.output_tokens": 23,
		"completion": ["Sorry, I couldn't find the name of the LLM provider. Could you please provide more information or context?"],
		"prompt": [
			{"role":"system","content":"You are a helpful assistant."},
			{"role":"user","content":"What is the name of the LLM provider?"},
		]
	});
	assert_llm(
		io,
		include_bytes!("../llm/tests/requests/completions/basic.json"),
		want,
	)
	.await;
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

#[tokio::test]
async fn direct_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"responseHeaderModifier": {
					"add": {
						"x-filter": "x-filter-val"
					},
				},
				"directResponse": {
					"body": "hello",
					"status": 422,
				},
				"transformations": {
					"response": {
						"add": {
							"x-xfm": "'x-xfm-val'",
						},
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 422);
	// Each type of response modifier should still run even though its a direct response
	assert_eq!(res.hdr("x-filter"), "x-filter-val");
	assert_eq!(res.hdr("x-xfm"), "x-xfm-val");
	assert_eq!(read_body!(res).as_bytes(), b"hello");
}

#[tokio::test]
async fn direct_response_expression() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"bodyExpression": "'hello ' + request.path",
					"headers": {
						"x-direct": "'direct-' + request.headers['x-test']",
					},
					"status": 422,
				},
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo/p",
		&[("x-test", "value")],
	)
	.await;
	assert_eq!(res.status(), 422);
	assert_eq!(res.hdr("x-direct"), "direct-value");
	assert_eq!(read_body!(res).as_bytes(), b"hello /p");
}

#[tokio::test]
async fn direct_response_expression_can_read_request_body() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"bodyExpression": "request.body",
					"status": 200,
				},
			},
		}))
		.await;

	let res = send_request_body(io.clone(), Method::POST, "http://lo/p", b"echo me").await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body!(res).as_bytes(), b"echo me");
}

#[tokio::test]
async fn conditional_direct_response() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"directResponse": {
					"conditional": [
						{
							"condition": "request.path == '/a'",
							"body": "hello a",
							"status": 200,
						},
						{
							"body": "hello fallback",
							"status": 202,
						},
					]
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/a").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo/b").await;
	assert_eq!(res.status(), 202);
}

#[tokio::test]
async fn response_policy_short_circuit() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"extAuthz": {
					// Dummy host that should fail
					"host": "127.0.0.1:1",
				},
				"responseHeaderModifier": {
					"add": {
						"x-filter": "x-filter-val"
					},
				},
				"transformations": {
					"response": {
						"add": {
							"x-xfm": "'x-xfm-val'",
						},
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 403);
	// Each type of response modifier should NOT run since the ext_authz short-circuits the req
	assert_eq!(res.hdr("x-filter"), "");
	assert_eq!(res.hdr("x-xfm"), "");
	assert_eq!(read_body!(res).as_bytes(), b"external authorization failed");
}

#[tokio::test]
async fn tls_termination() {
	let mock = simple_mock().await;
	let bind = https_bind();

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(basic_route(*mock.address()));

	let io = t.serve_https(strng::new("bind"), Some("a.example.com"));
	let res = RequestBuilder::new(Method::GET, "http://a.example.com")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);

	// This one should fail since it doesn't match the SNI.
	let io = t.serve_https(strng::new("bind"), Some("not-the-domain"));
	let res = RequestBuilder::new(Method::GET, "http://lo").send(io).await;
	assert_matches!(res, Err(_));
}

#[tokio::test]
async fn tls_connection_reuses_listener_after_route_insert() {
	let existing = body_mock(b"existing-route").await;
	let added = body_mock(b"added-route").await;
	let bind = https_bind();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*existing.address())
		.with_backend(*added.address())
		.with_bind(bind)
		.with_route(route_with_prefix(*existing.address(), "/existing"));

	let (mut sender, conn) = serve_https_http1_connection(&t, "a.example.com").await;

	let res = sender
		.send_request(
			::http::Request::builder()
				.method(Method::GET)
				.uri("/existing")
				.version(Version::HTTP_11)
				.header(header::HOST, "a.example.com")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	assert_eq!(
		res.into_body().collect().await.unwrap().to_bytes().as_ref(),
		b"existing-route"
	);

	t.pi
		.stores
		.binds
		.write()
		.insert_route(route_with_prefix(*added.address(), "/added"), LISTENER_KEY);

	let res = sender
		.send_request(
			::http::Request::builder()
				.method(Method::GET)
				.uri("/added")
				.version(Version::HTTP_11)
				.header(header::HOST, "a.example.com")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	assert_eq!(
		res.into_body().collect().await.unwrap().to_bytes().as_ref(),
		b"added-route"
	);

	drop(sender);
	conn.abort();
}

#[tokio::test]
async fn tls_connection_drains_when_listener_changes() {
	let existing = body_mock(b"existing-route").await;
	let bind = https_bind();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*existing.address())
		.with_bind(bind)
		.with_route(route_with_prefix(*existing.address(), "/existing"));

	let (mut sender, conn) = serve_https_http1_connection(&t, "a.example.com").await;

	let res = sender
		.send_request(
			::http::Request::builder()
				.method(Method::GET)
				.uri("/existing")
				.version(Version::HTTP_11)
				.header(header::HOST, "a.example.com")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);

	t.pi.stores.binds.write().insert_listener(
		Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: strng::new("*.example.com"),
			protocol: ListenerProtocol::HTTPS(crate::types::agent::ServerTLSConfig::new_invalid()),
		},
		BIND_KEY,
	);

	agent_core::test_helpers::check_eventually(
		Duration::from_secs(1),
		|| async { conn.is_finished() },
		|b| *b,
	)
	.await
	.expect("connection should drain after listener change");

	let res = sender
		.send_request(
			::http::Request::builder()
				.method(Method::GET)
				.uri("/existing")
				.version(Version::HTTP_11)
				.header(header::HOST, "a.example.com")
				.body(Body::empty())
				.unwrap(),
		)
		.await;
	assert_matches!(res, Err(_));
	let _ = conn.await;
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tls_backend_connection() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		..Default::default()
	}
	.try_into()
	.unwrap();

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![BackendTrafficPolicy::BackendTLS(backend_tls)],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tls_backend_connection_alpn() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		alpn: Some(vec!["http/1.1".to_string()]),
		..Default::default()
	}
	.try_into()
	.unwrap();

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![BackendTrafficPolicy::BackendTLS(backend_tls)],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	// We should keep HTTP/1.1! We negotiated to ALPN HTTP/1.1 so must send that.
	assert_eq!(
		read_body(res.into_body()).await.version,
		::http::Version::HTTP_11
	);

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	// We should downgrade! We negotiated to ALPN HTTP/1.1 so must send that.
	assert_eq!(
		read_body(res.into_body()).await.version,
		::http::Version::HTTP_11
	);
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tls_backend_http2_version() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		..Default::default()
	}
	.try_into()
	.unwrap();
	let backend_version = backend::HTTP {
		version: Some(Version::HTTP_2),
		..Default::default()
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![
				BackendTrafficPolicy::BackendTLS(backend_tls),
				BackendTrafficPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	// We explicitly set HTTP2, and the ALPN allows it
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	// We explicitly set HTTP2, and the ALPN allows it
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tls_backend_http1_version() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		..Default::default()
	}
	.try_into()
	.unwrap();
	let backend_version = backend::HTTP {
		version: Some(Version::HTTP_11),
		..Default::default()
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![
				BackendTrafficPolicy::BackendTLS(backend_tls),
				BackendTrafficPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	// We explicitly set HTTP_11, and the ALPN allows it. We should downgrade their request!
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_11);

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	// We explicitly set HTTP_11, and the ALPN allows it
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_11);
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tls_backend_version_with_alpn() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		alpn: Some(vec!["http/1.1".to_string()]),
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		..Default::default()
	}
	.try_into()
	.unwrap();
	let backend_version = backend::HTTP {
		version: Some(Version::HTTP_2),
		..Default::default()
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![
				BackendTrafficPolicy::BackendTLS(backend_tls),
				BackendTrafficPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	// Explicit ALPN takes precedence over explicit backend version
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_11);

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	// Explicit ALPN takes precedence over explicit backend version
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_11);
}

async fn send_http_version(t: &TestBind, v: Version) -> Response {
	let io = if v == Version::HTTP_11 {
		t.serve_http(strng::new("bind"))
	} else {
		t.serve_http2(strng::new("bind"))
	};
	RequestBuilder::new(Method::GET, "http://lo")
		.version(v)
		.send(io)
		.await
		.unwrap()
}

#[tokio::test]
async fn header_manipulation() {
	let (mock, mut bind, _io) = basic_setup().await;
	bind
		.attach_route(json!({
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-route-req": "route-req",
					},
				},
				"responseHeaderModifier": {
					"add": {
						"x-route-resp": "route-resp",
					},
				},
			},
			"backends": [{
				"host": mock.address().to_string(),
				"policies": {
					"requestHeaderModifier": {
						"add": {
							"x-backend-req": "backend-req",
						},
					},
					"responseHeaderModifier": {
						"add": {
							"x-backend-resp": "backend-resp",
						},
					},
					"transformations": {
						"request": {
							"set": {
								"x-backend-xfm-req": "'backend-xfm-req'",
							},
						},
						"response": {
							"add": {
								"x-backend-xfm-resp": "'backend-xfm-resp'",
							},
						},
					},
				},
			}],
		}))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-route-resp"), "route-resp");
	assert_eq!(res.hdr("x-backend-resp"), "backend-resp");
	assert_eq!(res.hdr("x-backend-xfm-resp"), "backend-xfm-resp");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-route-req").unwrap().as_bytes(),
		b"route-req"
	);
	assert_eq!(
		body.headers.get("x-backend-req").unwrap().as_bytes(),
		b"backend-req"
	);
	assert_eq!(
		body.headers.get("x-backend-xfm-req").unwrap().as_bytes(),
		b"backend-xfm-req"
	);
}

#[tokio::test]
async fn gateway_ext_authz_response_headers_are_preserved() {
	struct AddResponseHeader;

	#[async_trait::async_trait]
	impl extauthmock::Handler for AddResponseHeader {
		async fn check(
			&mut self,
			_request: &crate::http::ext_authz::proto::CheckRequest,
		) -> Result<crate::http::ext_authz::proto::CheckResponse, tonic::Status> {
			use crate::http::ext_authz::proto::check_response::HttpResponse;
			use crate::http::ext_authz::proto::{HeaderValue, HeaderValueOption, OkHttpResponse};

			extauthmock::allow_response(Some(HttpResponse::OkResponse(OkHttpResponse {
				headers: vec![],
				headers_to_remove: vec![],
				response_headers_to_add: vec![HeaderValueOption {
					header: Some(HeaderValue {
						key: "x-gateway-authz-response".to_string(),
						value: "allowed".to_string(),
						raw_value: vec![],
					}),
					append: Some(false),
					append_action: 0,
				}],
				query_parameters_to_set: vec![],
				query_parameters_to_remove: vec![],
				..Default::default()
			})))
		}
	}

	let (mock, mut bind, io) = basic_setup().await;
	let authz = extauthmock::ExtAuthMock::new(|| AddResponseHeader)
		.spawn()
		.await;
	bind
		.attach_gateway_policy(json!({
			"extAuthz": {
				"host": authz.address,
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-gateway-authz-response"), "allowed");
	assert_eq!(read_body(res.into_body()).await.method, Method::GET);
	drop(mock);
}

#[tokio::test]
async fn gateway_transformation_response_headers_are_applied() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_gateway_policy(json!({
			"transformations": {
				"request": {
					"set": {
						"x-gateway-xfm-req": "'gateway-request'",
					},
				},
				"response": {
					"add": {
						"x-gateway-xfm-resp": "'gateway-response'",
					},
				},
			},
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-gateway-xfm-resp"), "gateway-response");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-gateway-xfm-req").unwrap().as_bytes(),
		b"gateway-request"
	);
}

#[tokio::test]
async fn inline_backend_policies() {
	let (mock, mut bind, io) = basic_setup().await;
	bind
		.attach_backend(json!({
			"name": "backend1",
			"host": mock.address(),
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-backend-req": "backend-req",
					}
				},
				"responseHeaderModifier": {
					"add": {
						"x-backend-resp": "backend-resp",
					}
				}
			}
		}))
		.await;
	bind
		.attach_route(json!({
			"policies": {
				"requestHeaderModifier": {
					"add": {
						"x-route-req": "route-req",
					},
				},
				"responseHeaderModifier": {
					"add": {
						"x-route-resp": "route-resp",
					},
				},
			},
			"backends": [{
				"backend": "/backend1",
				"policies": {
					"requestHeaderModifier": {
						"add": {
							"x-backend-route-req": "backend-route-req",
						},
					},
					"responseHeaderModifier": {
						"add": {
							"x-backend-route-resp": "backend-route-resp",
						},
					},
				},
			}],
		}))
		.await;

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	// We should get the route rule, and the inline backend rule. The Backend rule takes precedence
	// over the HTTPRoute.backendRef.filters though, so that one is ignored (no deep merging, either).
	assert_eq!(res.hdr("x-route-resp"), "route-resp");
	assert_eq!(res.hdr("x-backend-route-resp"), "backend-route-resp");
	assert_eq!(res.hdr("x-backend-resp"), "");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-route-req").unwrap().as_bytes(),
		b"route-req"
	);
	assert!(body.headers.get("x-backend-req").is_none(),);
	assert_eq!(
		body.headers.get("x-backend-route-req").unwrap().as_bytes(),
		b"backend-route-req"
	);
}

#[tokio::test]
async fn tunnel_absolute_form() {
	let mock = simple_mock().await;
	let tunnel_mock = simple_mock().await;
	let mut bind = base_gateway(&mock).with_backend(*tunnel_mock.address());
	bind
		.attached_backend_policy(
			mock.address(),
			json!({
				"backendTunnel": {
					"proxy": {
						"host": tunnel_mock.address(),
					}
				}
			}),
		)
		.await;
	bind
		.attached_backend_policy(
			tunnel_mock.address(),
			json!({
				"backendAuth": {
					"key": "my-key"
				}
			}),
		)
		.await;
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io.clone(), Method::GET, "http://lo/foo").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	// Unfortunately, wiremock obscures whether it is an absolute form or not and makes the typical case hardcoded
	// to "http://localhost". But our assertion here is good enough.
	assert_eq!(&body.uri.to_string(), "http://lo/foo");
	assert_eq!(
		body.headers.get("proxy-authorization").unwrap().as_bytes(),
		b"Bearer my-key"
	);
}

#[tokio::test]
#[cfg(feature = "tls-aws-lc")]
async fn tunnel_connect() {
	let (mock, _certs) = tls_mock().await;
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let tunnel_addr = listener.local_addr().unwrap();
	let upstream_addr = *mock.address();
	let (connect_tx, connect_rx) = oneshot::channel();
	let tunnel = tokio::spawn(async move {
		let (mut downstream, _) = listener.accept().await.unwrap();
		let mut buf = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let n = downstream.read(&mut chunk).await.unwrap();
			assert!(n > 0, "CONNECT request unexpectedly closed");
			buf.extend_from_slice(&chunk[..n]);
			if buf.windows(4).any(|w| w == b"\r\n\r\n") {
				break;
			}
		}
		let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
		connect_tx
			.send(String::from_utf8(buf[..header_end].to_vec()).unwrap())
			.unwrap();

		let mut upstream = TcpStream::connect(upstream_addr).await.unwrap();
		downstream
			.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
			.await
			.unwrap();
		tokio::io::copy_bidirectional(&mut downstream, &mut upstream)
			.await
			.unwrap();
	});

	let mut bind = base_gateway(&mock).with_backend(tunnel_addr);
	bind
		.attached_backend_policy(
			mock.address(),
			json!({
				"backendTunnel": {
					"proxy": {
						"host": tunnel_addr,
					}
				},
				"backendTLS": {
					"insecure": true
				}
			}),
		)
		.await;
	bind
		.attached_backend_policy(
			&tunnel_addr,
			json!({
				"backendAuth": {
					"key": {
						"value": "my-key",
						"location": {
							"header": {
								"name":"authorization",
								"prefix": "Basic "
							},
						}
					}
				}
			}),
		)
		.await;
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io, Method::GET, "http://lo/foo").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::GET);
	assert_eq!(&body.uri.to_string(), "https://lo/foo");

	let connect_req = connect_rx.await.unwrap();
	assert!(connect_req.starts_with(&format!("CONNECT {} HTTP/1.1\r\n", mock.address())));
	assert!(connect_req.contains(&format!("Host: {}\r\n", mock.address())));
	assert!(connect_req.contains("Proxy-Authorization: Basic my-key\r\n"));

	tunnel.abort();
}

#[tokio::test]
async fn incoming_connect_dynamic_forward_proxy() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let target_addr = listener.local_addr().unwrap();
	let upstream = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let mut buf = [0; 4];
		stream.read_exact(&mut buf).await.unwrap();
		assert_eq!(&buf, b"ping");
		stream.write_all(b"pong").await.unwrap();
	});

	let t = setup_dfp_bind().with_connect_enabled();
	let mut io = t.serve(BIND_KEY);
	let req = format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n\r\n");
	io.write_all(req.as_bytes()).await.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	io.write_all(b"ping").await.unwrap();
	let mut tunneled = [0; 4];
	io.read_exact(&mut tunneled).await.unwrap();
	assert_eq!(&tunneled, b"pong");
	upstream.await.unwrap();
}

#[tokio::test]
async fn incoming_connect_requires_frontend_connect_policy() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let target_addr = listener.local_addr().unwrap();

	let t = setup_dfp_bind();
	let mut io = t.serve(BIND_KEY);
	let req = format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n\r\n");
	io.write_all(req.as_bytes()).await.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);
}

#[tokio::test]
async fn incoming_connect_tunnel_reenters_bind_flow() {
	let mock = simple_mock().await;
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15008".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = "127.0.0.1:18080".parse().unwrap();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*mock.address()))
		.with_connect_mode_on_port(frontend::ConnectMode::Tunnel, 15008);
	let mut io = t.serve_tunnel(strng::literal!("outer"));
	io.write_all(b"CONNECT httpbingo.org:18080 HTTP/1.1\r\nHost: httpbingo.org:18080\r\n\r\n")
		.await
		.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	io.write_all(b"GET /foo HTTP/1.1\r\nHost: lo\r\nConnection: close\r\n\r\n")
		.await
		.unwrap();
	let mut tunneled = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut tunneled))
		.await
		.expect("timed out waiting for tunneled HTTP response")
		.unwrap();
	assert!(
		String::from_utf8_lossy(&tunneled).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected tunneled response: {}",
		String::from_utf8_lossy(&tunneled),
	);
}

#[tokio::test]
async fn incoming_connect_applies_backend_tls() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = http::backendtls::ResolvedBackendTLS {
		root: Some(certs.root_cert.pem().into_bytes()),
		hostname: Some("localhost".to_string()),
		alpn: Some(vec!["http/1.1".to_string()]),
		..Default::default()
	}
	.try_into()
	.unwrap();

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_connect_enabled()
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![BackendTrafficPolicy::BackendTLS(backend_tls)],
		})
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()));

	let mut io = t.serve(BIND_KEY);
	let authority = mock.address().to_string();
	io.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
		.await
		.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	io.write_all(
		format!("GET /foo HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n").as_bytes(),
	)
	.await
	.unwrap();
	let mut tunneled = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut tunneled))
		.await
		.expect("timed out waiting for tunneled TLS backend response")
		.unwrap();
	assert!(
		String::from_utf8_lossy(&tunneled).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected tunneled response: {}",
		String::from_utf8_lossy(&tunneled),
	);
}

#[tokio::test]
async fn incoming_connect_requires_authority_port() {
	let t = setup_dfp_bind().with_connect_enabled();
	let mut io = t.serve(BIND_KEY);
	io.write_all(b"CONNECT example.com HTTP/1.1\r\nHost: example.com\r\n\r\n")
		.await
		.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400 Bad Request\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);
}

#[tokio::test]
async fn incoming_connect_uses_backend_tunnel_proxy() {
	let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let target_addr = target_listener.local_addr().unwrap();
	let target = tokio::spawn(async move {
		let (mut stream, _) = target_listener.accept().await.unwrap();
		let mut buf = [0; 4];
		stream.read_exact(&mut buf).await.unwrap();
		assert_eq!(&buf, b"ping");
		stream.write_all(b"pong").await.unwrap();
	});

	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let proxy_addr = listener.local_addr().unwrap();
	let (connect_tx, connect_rx) = oneshot::channel();
	let proxy = tokio::spawn(async move {
		let (mut downstream, _) = listener.accept().await.unwrap();
		let mut buf = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let n = downstream.read(&mut chunk).await.unwrap();
			assert!(n > 0, "CONNECT request unexpectedly closed");
			buf.extend_from_slice(&chunk[..n]);
			if buf.windows(4).any(|w| w == b"\r\n\r\n") {
				break;
			}
		}
		let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
		connect_tx
			.send(String::from_utf8(buf[..header_end].to_vec()).unwrap())
			.unwrap();
		downstream
			.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
			.await
			.unwrap();
		let mut upstream = TcpStream::connect(target_addr).await.unwrap();
		let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
	});

	let mut t = setup_dfp_bind().with_connect_enabled();
	t.with_policy(TargetedPolicy {
		key: strng::literal!("pol/backend-tunnel"),
		name: None,
		inheritance: PolicyInheritance::default(),
		target: PolicyTarget::Backend(BackendTarget::Backend {
			name: strng::literal!("dynamic"),
			namespace: Default::default(),
			section: None,
		}),
		policy: BackendTrafficPolicy::Tunnel(backend::Tunnel {
			proxy: Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
				proxy_addr,
			))),
		})
		.into(),
	});
	let mut io = t.serve(BIND_KEY);
	let req = format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n\r\n");
	io.write_all(req.as_bytes()).await.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);

	let connect_req = connect_rx.await.unwrap();
	assert!(connect_req.starts_with(&format!("CONNECT {target_addr} HTTP/1.1\r\n")));
	assert!(connect_req.contains(&format!("Host: {target_addr}\r\n")));

	io.write_all(b"ping").await.unwrap();
	let mut tunneled = [0; 4];
	io.read_exact(&mut tunneled).await.unwrap();
	assert_eq!(&tunneled, b"pong");
	drop(io);
	target.await.unwrap();
	proxy.await.unwrap();
}

#[tokio::test]
async fn incoming_connect_snapshots_request_for_cel_logging() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let target_addr = listener.local_addr().unwrap();
	let upstream = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let _ = stream.read(&mut [0; 1]).await;
	});

	let config = serde_json::to_string(&json!({
		"config": {
			"logging": {
				"fields": {
					"add": {
						"request": "request",
						"backend": "backend",
					},
				},
			},
		},
	}))
	.unwrap();
	let t = setup_dfp_bind_with_config(&config).with_connect_enabled();
	let mut io = t.serve(BIND_KEY);
	let req = format!("CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\n\r\n");
	io.write_all(req.as_bytes()).await.unwrap();

	let mut response = Vec::new();
	loop {
		let mut chunk = [0; 1024];
		let n = io.read(&mut chunk).await.unwrap();
		assert!(n > 0, "CONNECT response unexpectedly closed");
		response.extend_from_slice(&chunk[..n]);
		if response.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}
	assert!(
		String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected CONNECT response: {}",
		String::from_utf8_lossy(&response),
	);
	drop(io);
	upstream.await.unwrap();

	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("endpoint", &target_addr.to_string()),
	])
	.await
	.unwrap();
	assert_eq!(log["http.path"].as_str(), Some("/"));
	assert_eq!(log["request"]["method"].as_str(), Some("CONNECT"));
	assert_eq!(log["request"]["path"].as_str(), Some("/"));
	assert_eq!(
		log["request"]["host"].as_str(),
		Some(target_addr.to_string().as_str())
	);
	assert_eq!(log["request"]["scheme"].as_str(), Some("http"));
	assert!(
		log["backend"].is_object(),
		"backend CEL context should be populated"
	);
}

#[tokio::test]
async fn api_key() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
		.attach_route_policy(json!({
			"apiKey": {
				"keys": [
					{
						"key": "sk-123",
						"metadata": {"group": "eng"},
					},
					{
						"key": "sk-456",
						"metadata": {"group": "sales"},
					}
				],
				"mode": "strict",
			},
			"authorization": {
				"rules": ["apiKey.group == 'eng'"],
			},
		}))
		.await;

	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-123")],
	)
	.await;
	assert_eq!(res.status(), 200);
	// Match but fails authz
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-456")],
	)
	.await;
	assert_eq!(res.status(), 403);
	// No match
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", "bearer sk-789")],
	)
	.await;
	assert_eq!(res.status(), 401);
	// No match
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn basic_auth() {
	let (_mock, mut bind, io) = basic_setup().await;
	bind
      .attach_route_policy(json!({
			"basicAuth": {
				"htpasswd": "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00\nbcrypt_test:$2y$05$nC6nErr9XZJuMJ57WyCob.EuZEjylDt2KaHfbfOtyb.EgL1I2jCVa\nsha1_test:{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=\ncrypt_test:bGVh02xkuGli2",
				"realm": "my-realm",
				"mode": "strict",
			},
			"authorization": {
				"rules": ["basicAuth.username == 'user'"],
			},
		}))
      .await;

	use base64::Engine;
	let md5 = base64::prelude::BASE64_STANDARD.encode(b"user:password");
	let sha1 = base64::prelude::BASE64_STANDARD.encode(b"sha1_test:password");
	let bcrypt = base64::prelude::BASE64_STANDARD.encode(b"bcrypt_test:password");
	let crypt = base64::prelude::BASE64_STANDARD.encode(b"crypt_test:password");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {md5}"))],
	)
	.await;
	assert_eq!(res.status(), 200);
	// Match but fails authz
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {sha1}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {crypt}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {bcrypt}"))],
	)
	.await;
	assert_eq!(res.status(), 403);
	// No match
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 401);
	let md5_wrong = base64::prelude::BASE64_STANDARD.encode(b"user:not-password");
	let res = send_request_headers(
		io.clone(),
		Method::GET,
		"http://lo",
		&[("authorization", &format!("basic {md5_wrong}"))],
	)
	.await;
	assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn test_hbone_address_parsing() {
	// Test parsing IP:port
	let uri = "127.0.0.1:8080".parse::<http::Uri>().unwrap();
	let addr = super::HboneAddress::try_from(&uri).unwrap();
	assert_matches!(addr, super::HboneAddress::SocketAddr(_));

	// Test parsing hostname:port
	let uri = "example.com:443".parse::<http::Uri>().unwrap();
	let addr = super::HboneAddress::try_from(&uri).unwrap();
	assert_matches!(addr, super::HboneAddress::SvcHostname(host, port) => {
		assert_eq!(host.as_ref(), "example.com");
		assert_eq!(port, 443);
	});

	// Test parsing invalid URI (this will panic on parse, so we skip it)
	// let uri = "invalid-uri".parse::<http::Uri>().unwrap(); // This would panic

	// Test URI with no host
	let uri_no_host = "/path".parse::<http::Uri>().unwrap();
	let result_no_host = super::HboneAddress::try_from(&uri_no_host);
	assert!(result_no_host.is_err());

	// Test URI with host but no port (should fail for CONNECT)
	let uri_no_port = "http://example.com".parse::<http::Uri>().unwrap();
	let result_no_port = super::HboneAddress::try_from(&uri_no_port);
	assert!(result_no_port.is_err());
}

#[tokio::test]
async fn test_hostname_resolution_logic() {
	use crate::types::discovery::{NetworkAddress, Service};

	// Create a mock service store with a service that has a hostname
	let mut stores = crate::store::DiscoveryStore::new();

	let service = Service {
		name: strng::new("waypoint-service"),
		namespace: strng::new("default"),
		hostname: strng::new("my-app.example.com"),
		vips: vec![NetworkAddress {
			network: strng::new("default"),
			address: "10.0.0.100".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, 8080)]),
		app_protocols: Default::default(),
		endpoints: Default::default(),
		subject_alt_names: Default::default(),
		waypoint: Some(crate::types::discovery::GatewayAddress {
			destination: crate::types::discovery::gatewayaddress::Destination::Hostname(
				crate::types::discovery::NamespacedHostname {
					namespace: strng::new("istio-system"),
					hostname: strng::new("waypoint.istio-system.svc.cluster.local"),
				},
			),
			hbone_mtls_port: 15008,
		}),
		load_balancer: None,
		ip_families: None,
		ingress_use_waypoint: false,
	};

	stores.insert_service_internal(service);

	// Test URI parsing for hostname:port
	let uri = "my-app.example.com:80".parse::<http::Uri>().unwrap();
	let parsed_addr = super::HboneAddress::try_from(&uri).unwrap();

	// Should parse as SvcHostname
	assert_matches!(parsed_addr, super::HboneAddress::SvcHostname(host, port) => {
		assert_eq!(host.as_ref(), "my-app.example.com");
		assert_eq!(port, 80);
	});

	// Test service lookup by hostname
	let hostname_str = "my-app.example.com";
	let found_service = super::find_service_by_hostname(&stores, hostname_str);
	assert!(found_service.is_some());

	let svc = found_service.unwrap();
	assert_eq!(svc.hostname.as_str(), "my-app.example.com");
	assert_eq!(svc.namespace.as_str(), "default");
	assert!(!svc.vips.is_empty());

	// Verify we can get the VIP
	let network = strng::new("default");
	let vip = svc.vips.iter().find(|v| v.network == network);
	assert!(vip.is_some());
	assert_eq!(vip.unwrap().address.to_string(), "10.0.0.100");

	// Test hostname that doesn't exist as a service
	let nonexistent_hostname = "nonexistent.example.com";
	let not_found = super::find_service_by_hostname(&stores, nonexistent_hostname);
	assert!(not_found.is_none());

	// Test service exists but has no VIPs
	let service_no_vips = Service {
		name: strng::new("service-no-vips"),
		namespace: strng::new("default"),
		hostname: strng::new("no-vips.example.com"),
		vips: vec![], // No VIPs
		ports: Default::default(),
		app_protocols: Default::default(),
		endpoints: Default::default(),
		subject_alt_names: Default::default(),
		waypoint: None,
		load_balancer: None,
		ip_families: None,
		ingress_use_waypoint: false,
	};
	stores.insert_service_internal(service_no_vips);

	let no_vips_found = super::find_service_by_hostname(&stores, "no-vips.example.com");
	assert!(no_vips_found.is_none()); // Should return None because service has no VIPs
}

async fn assert_llm(io: Client<MemoryConnector, Body>, body: &[u8], want: Value) {
	let r = rand::rng().random::<u128>();
	let res = send_request_body(io.clone(), Method::POST, &format!("http://lo/{r}"), body).await;

	// Ensure body finishes
	let _ = res.into_body().collect().await.unwrap();
	let log = agent_core::telemetry::testing::eventually_find(&[
		("scope", "request"),
		("http.path", &format!("/{r}")),
	])
	.await
	.unwrap();
	let valid = is_json_subset(&want, &log);
	assert!(valid, "want={want:#?} got={log:#?}");
}

// --- Dynamic Forward Proxy (DFP) tests ---

impl TestBind {
	fn with_connect_enabled(self) -> Self {
		self.with_connect_mode(frontend::ConnectMode::Route)
	}

	fn with_connect_mode(self, mode: frontend::ConnectMode) -> Self {
		self.with_connect_policy(mode, None)
	}

	fn with_connect_mode_on_port(self, mode: frontend::ConnectMode, port: u16) -> Self {
		self.with_connect_policy(mode, Some(port))
	}

	fn with_connect_policy(mut self, mode: frontend::ConnectMode, port: Option<u16>) -> Self {
		self.with_policy(TargetedPolicy {
			key: strng::literal!("pol/frontend-connect"),
			name: None,
			inheritance: PolicyInheritance::default(),
			target: PolicyTarget::Gateway(ListenerTarget {
				gateway_name: strng::literal!("default"),
				gateway_namespace: strng::literal!("default"),
				listener_name: None,
				port,
			}),
			policy: FrontendPolicy::Connect(frontend::Connect { mode }).into(),
		});
		self
	}
}

/// Helper to set up a DFP test: creates a Dynamic backend and a route pointing to it.
fn setup_dfp_bind() -> TestBind {
	setup_dfp_bind_with_config("{}")
}

fn setup_dfp_bind_with_config(config: &str) -> TestBind {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = Backend::Dynamic(backend_name, ());

	let route = basic_named_route("/dynamic".into());

	let t = setup_proxy_test(config).unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	t.with_bind(simple_bind()).with_route(route)
}

fn setup_dfp() -> (TestBind, Client<MemoryConnector, Body>) {
	let t = setup_dfp_bind();
	let io = t.serve_http(BIND_KEY);
	(t, io)
}

/// Helper to set up a DFP test behind an HTTPS listener.
fn setup_dfp_https() -> (TestBind, Client<MemoryConnector, Body>) {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = Backend::Dynamic(backend_name, ());

	let route = basic_named_route("/dynamic".into());

	let bind = Bind {
		key: BIND_KEY,
		// not really used
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTPS(
				types::local::LocalTLSServerConfig {
					mode: Default::default(),
					cert: "../../examples/tls/certs/cert.pem".into(),
					key: "../../examples/tls/certs/key.pem".into(),
					root: None,
					cipher_suites: None,
					min_tls_version: None,
					max_tls_version: None,
					key_exchange_groups: None,
				}
				.try_into()
				.unwrap(),
			),
		}]),
		protocol: BindProtocol::tls,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}").unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	let t = t.with_bind(bind).with_route(route);
	let io = t.serve_https(BIND_KEY, None);
	(t, io)
}

/// DFP and inference routing are orthogonal: DFP chooses the upstream from the request authority,
/// while inference routing expects an endpoint picker to choose the upstream endpoint.
#[tokio::test]
async fn dfp_rejects_inference_routing() {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = BackendWithPolicies {
		backend: Backend::Dynamic(backend_name, ()),
		inline_policies: vec![BackendTrafficPolicy::InferenceRouting(
			ext_proc::InferenceRouting {
				target: Arc::new(SimpleBackendReference::InlineBackend(Target::from((
					"127.0.0.1",
					9002,
				)))),
				destination_mode: ext_proc::InferenceRoutingDestinationMode::Passthrough,
				failure_mode: ext_proc::FailureMode::FailClosed,
			},
		)],
	};

	let route = basic_named_route("/dynamic".into());
	let t = setup_proxy_test("{}").unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.backend.name(), dynamic_backend);
	let t = t.with_bind(simple_bind()).with_route(route);
	let io = t.serve_http(BIND_KEY);

	let res = send_request(io, Method::GET, "http://example.com/dynamic").await;
	assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
	let body = res.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(
		String::from_utf8_lossy(&body),
		"processing failed: inferenceRouting is not supported with dynamic backends"
	);
}

/// DFP resolves the destination from the request's Host/URI authority, including the port.
#[tokio::test]
async fn dfp_uses_host_port() {
	let mock = simple_mock().await;
	let mock_addr = *mock.address();
	let (_bind, io) = setup_dfp();

	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-explicit-port-{r}");
	let url = format!("http://{mock_addr}{path}");
	let res = send_request(io, Method::GET, &url).await;

	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.uri.path(), path);

	// Also verify telemetry recorded the expected upstream endpoint with the explicit authority port.
	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	let expected_endpoint = mock_addr.to_string();
	assert_eq!(log["endpoint"].as_str(), Some(expected_endpoint.as_str()));
}

/// DFP defaults to port 80 when the URI has no explicit port and scheme is HTTP.
#[tokio::test]
async fn dfp_defaults_to_port_80_for_http() {
	let (_bind, io) = setup_dfp();
	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-http-default-{r}");

	// No port in URI — should default to 80 per HTTP scheme
	let _res = send_request(io, Method::GET, &format!("http://127.0.0.1{path}")).await;

	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	assert_eq!(log["endpoint"].as_str(), Some("127.0.0.1:80"));
}

/// DFP defaults to port 443 when the URI has no explicit port and scheme is HTTPS.
#[tokio::test]
async fn dfp_defaults_to_port_443_for_https() {
	let (_bind, io) = setup_dfp_https();
	let r = rand::rng().random::<u128>();
	let path = format!("/dfp-https-default-{r}");

	// No port in URI over HTTPS listener — should default to 443 per HTTPS scheme
	let _res = send_request(io, Method::GET, &format!("http://127.0.0.1{path}")).await;

	let log =
		agent_core::telemetry::testing::eventually_find(&[("scope", "request"), ("http.path", &path)])
			.await
			.unwrap();
	assert_eq!(log["endpoint"].as_str(), Some("127.0.0.1:443"));
}

#[test]
fn accept_error_classification() {
	use std::io::{Error, ErrorKind};

	use super::{is_accept_error_per_connection, is_accept_error_permanent};

	// Fatal errors: socket is permanently broken
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EBADF
	)));
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::ENOTSOCK
	)));
	// EINVAL is permanent on Linux (socket not listening), but transient on macOS
	#[cfg(target_os = "linux")]
	assert!(is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EINVAL
	)));
	#[cfg(not(target_os = "linux"))]
	assert!(!is_accept_error_permanent(&Error::from_raw_os_error(
		libc::EINVAL
	)));

	// Per-connection errors: harmless, no backoff needed
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::ECONNABORTED
	)));
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::ECONNRESET
	)));
	assert!(is_accept_error_per_connection(&Error::from_raw_os_error(
		libc::EPERM
	)));

	// Resource pressure errors: need backoff
	let pressure = Error::from_raw_os_error(libc::EMFILE);
	assert!(!is_accept_error_permanent(&pressure));
	assert!(!is_accept_error_per_connection(&pressure));

	let pressure = Error::from_raw_os_error(libc::ENOMEM);
	assert!(!is_accept_error_permanent(&pressure));
	assert!(!is_accept_error_per_connection(&pressure));

	// Generic errors: not permanent, not per-connection
	assert!(!is_accept_error_permanent(&Error::new(
		ErrorKind::WouldBlock,
		"again"
	)));
	assert!(!is_accept_error_per_connection(&Error::new(
		ErrorKind::WouldBlock,
		"again"
	)));
}

/// BindProtocol::auto should detect plaintext HTTP and proxy it successfully.
#[tokio::test]
async fn auto_protocol_plaintext_http() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let bind = Bind {
		key: BIND_KEY,
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTP,
		}]),
		protocol: BindProtocol::auto,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);
	let io = t.serve_http(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://lo")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::GET);
}

/// BindProtocol::auto should detect a TLS ClientHello (first byte 0x16) and
/// dispatch through TLS termination, just like BindProtocol::tls.
#[tokio::test]
async fn auto_protocol_tls_detection() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let mut bind = https_bind();
	bind.protocol = BindProtocol::auto;

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);
	let io = t.serve_https(strng::new("bind"), Some("a.example.com"));
	let res = RequestBuilder::new(Method::GET, "http://a.example.com")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
}

/// BindProtocol::auto with TLS should reject connections that don't match the SNI,
/// just like BindProtocol::tls does.
#[tokio::test]
async fn auto_protocol_tls_wrong_sni() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let mut bind = https_bind();
	bind.protocol = BindProtocol::auto;

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);
	let io = t.serve_https(strng::new("bind"), Some("not-the-domain"));
	let res = RequestBuilder::new(Method::GET, "http://lo").send(io).await;
	assert_matches!(res, Err(_));
}

/// Plaintext HTTP on a bind with only an HTTPS listener must be rejected.
/// This prevents a protocol downgrade where plaintext bypasses TLS.
#[tokio::test]
async fn auto_protocol_plaintext_rejected_for_https_only() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let mut bind = https_bind();
	bind.protocol = BindProtocol::auto;

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);
	// Send plaintext HTTP — should fail because only HTTPS listeners exist
	let io = t.serve_http(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://a.example.com")
		.send(io)
		.await
		.unwrap();
	// No HTTP listener matches, so we get a 404 (listener not found)
	assert_eq!(res.status(), 404);
}

/// TLS to a bind with only an HTTP listener must be rejected.
#[tokio::test]
async fn auto_protocol_tls_rejected_for_http_only() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let bind = Bind {
		key: BIND_KEY,
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTP,
		}]),
		protocol: BindProtocol::auto,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);
	// Send TLS — should fail because only HTTP listeners exist (no TLS listener match)
	let io = t.serve_https(strng::new("bind"), Some("example.com"));
	let res = RequestBuilder::new(Method::GET, "http://example.com")
		.send(io)
		.await;
	assert_matches!(res, Err(_));
}

/// Mixed listeners: a bind with both HTTP and HTTPS listeners should route
/// plaintext to the HTTP listener and TLS to the HTTPS listener.
/// The HTTP listener uses a specific hostname (not catch-all) so that if TLS
/// traffic were accidentally routed to the HTTP path, it would fail to match.
#[tokio::test]
async fn auto_protocol_mixed_listeners() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let route2 = basic_route(*mock.address());
	let bind = Bind {
		key: BIND_KEY,
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([
			Listener {
				key: strng::new("http-listener"),
				name: Default::default(),
				hostname: strng::new("http.local"),
				protocol: ListenerProtocol::HTTP,
			},
			Listener {
				key: strng::new("https-listener"),
				name: Default::default(),
				hostname: strng::new("*.example.com"),
				protocol: ListenerProtocol::HTTPS(
					types::local::LocalTLSServerConfig {
						mode: Default::default(),
						cert: "../../examples/tls/certs/cert.pem".into(),
						key: "../../examples/tls/certs/key.pem".into(),
						root: None,
						cipher_suites: None,
						min_tls_version: None,
						max_tls_version: None,
						key_exchange_groups: None,
					}
					.try_into()
					.unwrap(),
				),
			},
		]),
		protocol: BindProtocol::auto,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route_for_listener(strng::new("http-listener"), route)
		.with_route_for_listener(strng::new("https-listener"), route2);

	// Plaintext HTTP to http.local should route to the HTTP listener
	let io = t.serve_http(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://http.local")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);

	// Plaintext HTTP to a.example.com should fail (only HTTPS listener matches that host)
	let io = t.serve_http(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://a.example.com")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 404);

	// TLS to a.example.com should route to the HTTPS listener
	let io = t.serve_https(strng::new("bind"), Some("a.example.com"));
	let res = RequestBuilder::new(Method::GET, "http://a.example.com")
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);

	// TLS to http.local should fail (only HTTP listener matches that host, no TLS listener)
	let io = t.serve_https(strng::new("bind"), Some("http.local"));
	let res = RequestBuilder::new(Method::GET, "http://http.local")
		.send(io)
		.await;
	assert_matches!(res, Err(_));
}

/// Connections that send no data should time out instead of hanging forever.
#[tokio::test(start_paused = true)]
async fn auto_protocol_peek_timeout() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let bind = Bind {
		key: BIND_KEY,
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTP,
		}]),
		protocol: BindProtocol::auto,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind)
		.with_route(route);

	// Get raw duplex stream but don't send any data
	let _client = t.serve(strng::new("bind"));
	// With start_paused = true, the tokio runtime auto-advances time.
	// The proxy_bind future should complete within the timeout (5s) rather than hanging.
	tokio::time::sleep(std::time::Duration::from_secs(10)).await;
	// If we reach here, the timeout worked (auto-advance means no real wait).
}

#[tokio::test]
async fn waypoint_http_basic() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::GET);
}

#[tokio::test]
async fn waypoint_http_port_selects_distinct_backend() {
	// Two service routes on the same (service, path "/") differ only by their referenced
	// service_port. A request to :443 must reach the :443 backend, :80 the :80 backend.
	async fn id_mock(id: &'static str) -> MockServer {
		let mock = MockServer::start().await;
		Mock::given(wiremock::matchers::path_regex("/.*"))
			.respond_with(ResponseTemplate::new(200).insert_header("x-backend", id))
			.mount(&mock)
			.await;
		mock
	}
	let b80 = id_mock("p80").await;
	let b443 = id_mock("p443").await;

	let svc_nh = crate::types::discovery::NamespacedHostname {
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
	};
	let route = |key: &'static str, port: u16| Route {
		key: strng::new(key),
		service_key: Some(svc_nh.clone()),
		service_port: port,
		name: crate::types::agent::RouteName {
			name: strng::new(key),
			namespace: strng::literal!("default"),
			rule_name: None,
			kind: None,
		},
		hostnames: vec![],
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/".into()),
			method: None,
			query: vec![],
		}],
		inline_policies: vec![],
		backends: vec![crate::types::agent::RouteBackendReference {
			weight: 1,
			target: crate::types::agent::BackendReference::Service {
				name: svc_nh.clone(),
				port,
			}
			.into(),
			inline_policies: vec![],
		}],
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service_with_ports(&[(80, *b80.address()), (443, *b443.address())])
		.with_service_route(route("r80", 80))
		.with_service_route(route("r443", 443));

	// Destination :443 -> the 443-scoped route -> the :443 backend.
	let io = t.serve_waypoint_http_port(BIND_KEY, 443);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-backend"), "p443");

	// Destination :80 -> the 80-scoped route -> the :80 backend.
	let io = t.serve_waypoint_http_port(BIND_KEY, 80);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-backend"), "p80");
}

#[tokio::test]
async fn waypoint_http_fallback() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HTTP))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::POST, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.method, Method::POST);
}

#[tokio::test]
async fn waypoint_tcp_basic() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	let io = t.serve_waypoint_tcp(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn waypoint_service_policy_header_modifier() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_service_policy(json!({
		"requestHeaderModifier": {
			"add": { "x-svc-req": "from-service" },
		},
		"responseHeaderModifier": {
			"add": { "x-svc-resp": "from-service" },
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-svc-resp"), "from-service");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-svc-req").unwrap().as_bytes(),
		b"from-service"
	);
}

#[tokio::test]
async fn waypoint_service_policy_direct_response() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_service_policy(json!({
		"directResponse": {
			"status": 418,
			"body": "teapot",
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 418);
	assert_eq!(read_body!(res).as_bytes(), b"teapot");
}

#[tokio::test]
async fn waypoint_gateway_policy_authz_allow() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 12345"],
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	let res = send_request(io, Method::GET, "http://my-svc.default.svc.cluster.local").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn waypoint_gateway_policy_authz_deny() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 54321"],
		},
	}))
	.await;
	let io = t.serve_waypoint_http(BIND_KEY);
	RequestBuilder::new(Method::GET, "http://my-svc.default.svc.cluster.local")
		.send(io)
		.await
		.expect_err("should be denied by network authorization");
}

/// Gateway-targeted network authorization applies to TCP waypoint path.
#[tokio::test]
async fn waypoint_tcp_gateway_policy_authz_deny() {
	let mock = simple_mock().await;
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(waypoint_bind(ListenerProtocol::HBONE))
		.with_waypoint_service(*mock.address());
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": ["source.port == 54321"],
		},
	}))
	.await;
	let io = t.serve_waypoint_tcp(BIND_KEY);
	RequestBuilder::new(Method::GET, "http://my-svc.default.svc.cluster.local")
		.send(io)
		.await
		.expect_err("should be denied by network authorization");
}

// --- Ingress → Waypoint tests ---
// These test that when a service has ingress_use_waypoint=true,
// build_service_call correctly populates the BackendCall with
// a WaypointTarget and hostname-based target.

#[tokio::test]
async fn ingress_use_waypoint_sets_waypoint_target() {
	use crate::proxy::httpproxy;
	use crate::types::discovery::NamespacedHostname;

	let mock = simple_mock().await;
	let waypoint_addr: std::net::SocketAddr = "10.0.0.50:15008".parse().unwrap();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_ingress_use_waypoint_service(*mock.address(), waypoint_addr);

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	assert!(svc.ingress_use_waypoint, "ingress_use_waypoint must be set");
	assert!(svc.waypoint.is_some(), "waypoint must be configured");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// Waypoint target should be populated
	let wp = backend_call
		.waypoint
		.expect("waypoint target must be set when ingress_use_waypoint is true");
	assert_eq!(
		wp.address.ip(),
		waypoint_addr.ip(),
		"waypoint address should be the waypoint VIP"
	);
	assert_eq!(
		wp.address.port(),
		waypoint_addr.port(),
		"waypoint port should be the hbone_mtls_port"
	);

	// Target must be the service hostname (used as the HBONE CONNECT authority for the waypoint)
	assert_matches!(backend_call.target, Target::Hostname(host, port) => {
		assert_eq!(host.as_str(), "my-svc.default.svc.cluster.local");
		assert_eq!(port, 80);
	});
}

#[tokio::test]
async fn ingress_use_waypoint_false_no_waypoint() {
	use crate::proxy::httpproxy;
	use crate::types::discovery::NamespacedHostname;

	let mock = simple_mock().await;
	// Use the standard waypoint service helper which has ingress_use_waypoint: false
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_waypoint_service(*mock.address());

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	assert!(
		!svc.ingress_use_waypoint,
		"ingress_use_waypoint should be false"
	);

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// Waypoint should NOT be set
	assert!(
		backend_call.waypoint.is_none(),
		"waypoint should not be set when ingress_use_waypoint is false"
	);

	// Target should be a direct workload address, not hostname
	assert_matches!(backend_call.target, Target::Address(_));
}

#[tokio::test]
async fn ingress_use_waypoint_remote_waypoint_uses_network_gateway() {
	use crate::proxy::httpproxy;
	use crate::store::LocalWorkload;
	use crate::types::discovery::gatewayaddress::Destination;
	use crate::types::discovery::{
		GatewayAddress, Identity, InboundProtocol, NamespacedHostname, NetworkAddress, Service,
		Workload,
	};

	let mock = simple_mock().await;
	let waypoint_vip: std::net::IpAddr = "240.240.0.5".parse().unwrap();
	let waypoint_ip: std::net::IpAddr = "10.20.0.12".parse().unwrap();
	let gateway_ip: std::net::IpAddr = "172.18.7.110".parse().unwrap();
	let remote_network = strng::literal!("network-2");
	let t = setup_proxy_test("{}").unwrap();

	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: Some(GatewayAddress {
			destination: Destination::Hostname(NamespacedHostname {
				namespace: strng::literal!("default"),
				hostname: strng::literal!("waypoint.default.svc.cluster.local"),
			}),
			hbone_mtls_port: 15008,
		}),
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	let wp_svc = Service {
		name: strng::literal!("waypoint"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("waypoint.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: waypoint_vip,
		}],
		ports: std::collections::HashMap::from([(15008, 15008)]),
		subject_alt_names: vec![Identity::Spiffe {
			trust_domain: strng::literal!("td2"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint-san"),
		}],
		..Default::default()
	};
	let wp_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-waypoint-wl-uid"),
			name: strng::literal!("test-waypoint-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint"),
			network: remote_network.clone(),
			workload_ips: vec![waypoint_ip],
			network_gateway: Some(GatewayAddress {
				destination: Destination::Address(NetworkAddress {
					network: remote_network.clone(),
					address: gateway_ip,
				}),
				hbone_mtls_port: 15008,
			}),
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/waypoint.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(15008, 15008)]),
		)]),
	};
	let gw_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-gateway-wl-uid"),
			name: strng::literal!("test-gateway-wl"),
			namespace: strng::literal!("istio-gateways"),
			service_account: strng::literal!("istio-eastwest"),
			network: remote_network.clone(),
			workload_ips: vec![gateway_ip],
			..Default::default()
		},
		services: Default::default(),
	};

	t.pi
		.stores
		.discovery
		.sync_local(
			vec![svc, wp_svc],
			vec![wl, wp_wl, gw_wl],
			Default::default(),
		)
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	assert!(
		backend_call.waypoint.is_none(),
		"remote waypoint should be reached through double HBONE, not direct waypoint transport"
	);
	let (resolved_gw, gw_identities) = backend_call
		.network_gateway
		.expect("remote waypoint should resolve a network gateway");
	assert_matches!(resolved_gw.destination, Destination::Address(addr) => {
		assert_eq!(addr.address, gateway_ip);
		assert_eq!(addr.network, remote_network);
	});
	assert_eq!(resolved_gw.hbone_mtls_port, 15008);
	// Outer tunnel: gateway workload id (the gateway is referenced by address, so no SANs).
	assert_eq!(
		gw_identities,
		vec![Identity::Spiffe {
			trust_domain: strng::EMPTY,
			namespace: strng::literal!("istio-gateways"),
			service_account: strng::literal!("istio-eastwest"),
		}]
	);
	// Inner tunnel: waypoint workload id + waypoint service SANs.
	assert_matches!(backend_call.transport_override, Some((InboundProtocol::HBONE, identities)) => {
		assert_eq!(identities, vec![
			Identity::Spiffe {
				trust_domain: strng::EMPTY,
				namespace: strng::literal!("default"),
				service_account: strng::literal!("waypoint"),
			},
			Identity::Spiffe {
				trust_domain: strng::literal!("td2"),
				namespace: strng::literal!("default"),
				service_account: strng::literal!("waypoint-san"),
			},
		]);
	});
	assert_matches!(backend_call.target, Target::Hostname(host, port) => {
		assert_eq!(host.as_str(), "my-svc.default.svc.cluster.local");
		assert_eq!(port, 80);
	});
}

#[tokio::test]
async fn ingress_use_waypoint_ip_based_waypoint() {
	use crate::proxy::httpproxy;
	use crate::store::LocalWorkload;
	use crate::types::discovery::gatewayaddress::Destination;
	use crate::types::discovery::{
		GatewayAddress, NamespacedHostname, NetworkAddress, Service, Workload,
	};

	let mock = simple_mock().await;
	let waypoint_ip: std::net::IpAddr = "10.0.0.99".parse().unwrap();

	let t = setup_proxy_test("{}").unwrap();

	// Create a service with an IP-based waypoint (not hostname)
	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: Some(GatewayAddress {
			destination: Destination::Address(NetworkAddress {
				network: strng::EMPTY,
				address: waypoint_ip,
			}),
			hbone_mtls_port: 15008,
		}),
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	// Waypoint workload at the IP-based waypoint address, so its SPIFFE identity
	// can be resolved for mTLS verification.
	let wp_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-waypoint-wl-uid"),
			name: strng::literal!("test-waypoint-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("waypoint"),
			workload_ips: vec![waypoint_ip],
			..Default::default()
		},
		services: Default::default(),
	};
	t.pi
		.stores
		.discovery
		.sync_local(vec![svc], vec![wl, wp_wl], Default::default())
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	let wp = backend_call
		.waypoint
		.expect("waypoint target must be set for IP-based waypoint");
	assert_eq!(wp.address.ip(), waypoint_ip);
	assert_eq!(wp.address.port(), 15008);
}

#[tokio::test]
async fn ingress_use_waypoint_no_waypoint_field_no_routing() {
	use crate::proxy::httpproxy;
	use crate::store::LocalWorkload;
	use crate::types::discovery::{NamespacedHostname, NetworkAddress, Service, Workload};

	let mock = simple_mock().await;
	let t = setup_proxy_test("{}").unwrap();

	// Service with ingress_use_waypoint=true but NO waypoint configured
	let svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		waypoint: None, // No waypoint
		ingress_use_waypoint: true,
		..Default::default()
	};
	let wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("test-wl-uid"),
			name: strng::literal!("test-wl"),
			namespace: strng::literal!("default"),
			workload_ips: vec![mock.address().ip()],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};
	t.pi
		.stores
		.discovery
		.sync_local(vec![svc], vec![wl], Default::default())
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	// No waypoint configured, so it should fall back to direct routing
	assert!(
		backend_call.waypoint.is_none(),
		"waypoint should not be set when no waypoint is configured on the service"
	);
	assert_matches!(backend_call.target, Target::Address(_));
}

#[tokio::test]
async fn ingress_use_waypoint_build_transport_falls_back_without_ca() {
	use crate::proxy::httpproxy;
	use crate::types::discovery::NamespacedHostname;

	let mock = simple_mock().await;
	let waypoint_addr: std::net::SocketAddr = "10.0.0.50:15008".parse().unwrap();
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_ingress_use_waypoint_service(*mock.address(), waypoint_addr);

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	assert!(backend_call.waypoint.is_some());

	// build_transport with no CA should fall back to plain transport
	let transport = httpproxy::build_transport(&t.pi, &backend_call, None, None, None, None)
		.await
		.expect("build_transport should succeed");
	// Without CA, it falls back to Plain
	assert_eq!(transport.name(), "plaintext");
}

#[tokio::test]
async fn network_gateway_hostname_resolves_via_service_endpoint() {
	use crate::proxy::httpproxy;
	use crate::store::LocalWorkload;
	use crate::types::discovery::gatewayaddress::Destination;
	use crate::types::discovery::{
		GatewayAddress, Identity, NamespacedHostname, NetworkAddress, Service, Workload,
	};

	let mock = simple_mock().await;
	let gw_ip: std::net::IpAddr = "192.168.1.10".parse().unwrap();
	let remote_network = strng::literal!("network-3");
	let gateway_namespace = strng::literal!("gateway-ns");
	let gateway_hostname = strng::literal!("gateway.example.internal");
	let svc_port: u16 = 15008;
	let gw_target_port: u16 = 31234;

	let t = setup_proxy_test("{}").unwrap();

	let app_svc = Service {
		name: strng::literal!("my-svc"),
		namespace: strng::literal!("default"),
		hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "10.0.0.1".parse().unwrap(),
		}],
		ports: std::collections::HashMap::from([(80, mock.address().port())]),
		..Default::default()
	};
	let remote_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("remote-wl-uid"),
			name: strng::literal!("remote-wl"),
			namespace: strng::literal!("default"),
			service_account: strng::literal!("remote-sa"),
			network: remote_network.clone(),
			workload_ips: vec!["10.244.0.5".parse().unwrap()],
			network_gateway: Some(GatewayAddress {
				destination: Destination::Hostname(NamespacedHostname {
					namespace: gateway_namespace.clone(),
					hostname: gateway_hostname.clone(),
				}),
				hbone_mtls_port: svc_port,
			}),
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			"default/my-svc.default.svc.cluster.local".to_string(),
			std::collections::HashMap::from([(80, mock.address().port())]),
		)]),
	};

	// gateway service with port mapping
	let gw_svc = Service {
		name: strng::literal!("gateway-svc"),
		namespace: gateway_namespace.clone(),
		hostname: gateway_hostname.clone(),
		vips: vec![],
		ports: std::collections::HashMap::from([(svc_port, gw_target_port)]),
		subject_alt_names: vec![Identity::Spiffe {
			trust_domain: strng::literal!("td-gw"),
			namespace: gateway_namespace.clone(),
			service_account: strng::literal!("gateway-san"),
		}],
		..Default::default()
	};
	let gw_wl = LocalWorkload {
		workload: Workload {
			uid: strng::literal!("gw-wl-uid"),
			name: strng::literal!("gw-1"),
			namespace: gateway_namespace.clone(),
			service_account: strng::literal!("gateway-sa"),
			network: remote_network.clone(),
			workload_ips: vec![gw_ip],
			..Default::default()
		},
		services: std::collections::HashMap::from([(
			format!("{}/{}", gateway_namespace, gateway_hostname),
			std::collections::HashMap::from([(svc_port, gw_target_port)]),
		)]),
	};

	t.pi
		.stores
		.discovery
		.sync_local(
			vec![app_svc, gw_svc],
			vec![remote_wl, gw_wl],
			Default::default(),
		)
		.unwrap();

	let svc = t
		.pi
		.stores
		.read_discovery()
		.services
		.get_by_namespaced_host(&NamespacedHostname {
			namespace: strng::literal!("default"),
			hostname: strng::literal!("my-svc.default.svc.cluster.local"),
		})
		.expect("app service must exist");

	let backend_call = httpproxy::build_service_call(
		&t.pi,
		Default::default(),
		&mut None,
		Default::default(),
		&svc,
		&80,
		None,
		None,
	)
	.expect("build_service_call should succeed");

	let (resolved_gw, gw_identities) = backend_call
		.network_gateway
		.expect("network_gateway must be resolved for hostname-form destination");

	assert_matches!(resolved_gw.destination, Destination::Address(addr) => {
		assert_eq!(addr.address, gw_ip, "should resolve to the gateway endpoint IP");
		assert_eq!(addr.network, remote_network, "network should be the gateway workload's network");
	});
	assert_eq!(
		resolved_gw.hbone_mtls_port, gw_target_port,
		"port should be the endpoint target port, not the service port"
	);
	// Outer-tunnel identities match ztunnel: gateway workload id + gateway service SANs.
	assert_eq!(
		gw_identities,
		vec![
			Identity::Spiffe {
				trust_domain: strng::EMPTY,
				namespace: gateway_namespace.clone(),
				service_account: strng::literal!("gateway-sa"),
			},
			Identity::Spiffe {
				trust_domain: strng::literal!("td-gw"),
				namespace: gateway_namespace.clone(),
				service_account: strng::literal!("gateway-san"),
			},
		]
	);
}
