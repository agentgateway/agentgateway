use agentgateway::types::agent::{
	BackendTarget, BindMode, PolicyInheritance, PolicyTarget, TargetedPolicy, TunnelProtocol,
};
use agentgateway::types::frontend;
use hyper::client::conn::http1;
use rustls_pki_types::ServerName;
use tokio_rustls::TlsConnector;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::prelude::*;
use crate::tests::dfp::{setup_dfp_bind, setup_dfp_bind_with_config};
use crate::tests::tls::test_server_tls_config;

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

/// An internal bind (`mode: internal`) never opens an OS socket, yet remains reachable
/// via CONNECT tunnel re-entry: the outer bind reads `CONNECT host:18080`, looks the inner
/// bind up by port, and re-enters it in-process. This mirrors
/// `incoming_connect_tunnel_reenters_bind_flow`, but asserts that (a) a direct TCP connection
/// to the inner port is refused (no socket was bound) while (b) re-entry still succeeds.
#[tokio::test]
async fn incoming_connect_tunnel_reenters_internal_bind() {
	let mock = simple_mock().await;
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let outer_addr = listener.local_addr().unwrap();
	drop(listener);
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let inner_addr = listener.local_addr().unwrap();
	drop(listener);

	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = outer_addr;
	let mut inner = simple_bind();
	inner.address = inner_addr;
	// The inner bind is routing-only: it must not open a listener socket.
	inner.mode = BindMode::Internal;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*mock.address()))
		.with_connect_mode_on_port(frontend::ConnectMode::Tunnel, outer_addr.port());

	// The internal bind did not open a socket, so a direct connection to its port is refused.
	let direct = TcpStream::connect(inner_addr).await;
	assert!(
		direct.is_err(),
		"internal bind must not open a socket, but a direct connection to {inner_addr} succeeded",
	);

	let mut io = t.serve_tunnel(strng::literal!("outer"));
	let connect = format!(
		"CONNECT httpbingo.org:{} HTTP/1.1\r\nHost: httpbingo.org:{}\r\n\r\n",
		inner_addr.port(),
		inner_addr.port()
	);
	io.write_all(connect.as_bytes()).await.unwrap();

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

/// A wildcard internal bind (`mode: internal` with no port) is the catch-all for CONNECT
/// re-entry: when no other bind matches the requested destination port, the tunnel falls back
/// to it via `find_wildcard_bind`. Here the only inner bind is the wildcard (port 0), and a
/// `CONNECT host:18085` (a port no bind listens on) is served by re-entering it.
#[tokio::test]
async fn incoming_connect_tunnel_reenters_wildcard_bind() {
	let mock = simple_mock().await;
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15012".parse().unwrap();
	// Wildcard internal bind: internal mode + port 0 (address from simple_bind is 127.0.0.1:0).
	let mut inner = simple_bind();
	inner.key = strng::literal!("bind/wildcard");
	inner.mode = BindMode::Internal;
	assert!(
		inner.is_wildcard(),
		"inner bind should be the wildcard fallback"
	);
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*mock.address()))
		.with_connect_mode_on_port(frontend::ConnectMode::Tunnel, 15012);

	let mut io = t.serve_tunnel(strng::literal!("outer"));
	// No bind listens on 18085; the wildcard bind serves it.
	io.write_all(b"CONNECT httpbingo.org:18085 HTTP/1.1\r\nHost: httpbingo.org:18085\r\n\r\n")
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

/// Headers carried on a CONNECT request are captured and surfaced to CEL as
/// `source.connectHeaders` on the re-entered (tunneled) request, so an
/// authorization policy on the inner bind can match against them.
#[tokio::test]
async fn incoming_connect_tunnel_exposes_connect_headers_to_cel() {
	async fn tunnel_inner_request(custom_header: Option<&str>) -> String {
		let mock = simple_mock().await;
		let mut outer = simple_bind();
		outer.key = strng::literal!("outer");
		outer.address = "127.0.0.1:15008".parse().unwrap();
		let mut inner = simple_bind();
		inner.address = "127.0.0.1:18080".parse().unwrap();
		let mut t = setup_proxy_test("{}")
			.unwrap()
			.with_backend(*mock.address())
			.with_bind(outer)
			.with_bind(inner)
			.with_route(basic_route(*mock.address()))
			.with_connect_mode_on_port(frontend::ConnectMode::Tunnel, 15008);
		// Authorization on the re-entered request only allows when the custom header
		// carried on the CONNECT request is present and matches.
		t.attach_route_policy(json!({
			"authorization": {
				"rules": ["source.connectHeaders[\"x-custom-header\"] == \"custom-value\""],
			},
		}))
		.await;

		let mut io = t.serve_tunnel(strng::literal!("outer"));
		let connect = match custom_header {
			Some(v) => format!(
				"CONNECT httpbingo.org:18080 HTTP/1.1\r\nHost: httpbingo.org:18080\r\nx-custom-header: {v}\r\n\r\n"
			),
			None => {
				"CONNECT httpbingo.org:18080 HTTP/1.1\r\nHost: httpbingo.org:18080\r\n\r\n".to_string()
			},
		};
		io.write_all(connect.as_bytes()).await.unwrap();

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
		String::from_utf8_lossy(&tunneled).to_string()
	}

	// Header present and matching: the inner request is authorized (200).
	let allowed = tunnel_inner_request(Some("custom-value")).await;
	assert!(
		allowed.starts_with("HTTP/1.1 200 OK\r\n"),
		"expected inner request to be authorized, got: {allowed}",
	);

	// Header absent: `source.connectHeaders` is empty, the rule does not match,
	// and the inner request is denied (403).
	let denied = tunnel_inner_request(None).await;
	assert!(
		denied.starts_with("HTTP/1.1 403 Forbidden\r\n"),
		"expected inner request to be denied, got: {denied}",
	);
}

/// A `tunnelProtocol: Connect` bind with an HTTPS listener terminates the OUTER
/// TLS BEFORE serving CONNECT, so the CONNECT request and its headers are
/// encrypted on the wire.
///
/// The client completes a TLS handshake to the outer bind (which only
/// succeeds if the gateway terminated the outer TLS rather than serving plaintext
/// CONNECT on the raw socket), sends CONNECT inside that TLS carrying a custom
/// header, and the re-entered inner request is authorized iff the header survived
/// (`source.connectHeaders`).
#[tokio::test]
async fn connect_tunnel_terminates_outer_tls() {
	// SNI must match the `*.example.com` static cert at examples/mcp-tls/certs.
	const SNI: &str = "gw.example.com";

	async fn tls_connect_inner(custom_header: Option<&str>) -> String {
		let mock = simple_mock().await;

		// OUTER bind: HTTPS Static cert + tunnelProtocol Connect. The fix terminates
		// this outer TLS before serving CONNECT.
		let outer = Bind {
			key: strng::literal!("outer"),
			address: "127.0.0.1:15011".parse().unwrap(),
			listeners: ListenerSet::from_list([Listener {
				key: LISTENER_KEY,
				name: Default::default(),
				hostname: strng::new("*.example.com"),
				protocol: ListenerProtocol::HTTPS(test_server_tls_config()),
			}]),
			protocol: BindProtocol::tls,
			tunnel_protocol: TunnelProtocol::Connect,
			mode: Default::default(),
		};

		// INNER plain bind, re-entered by the CONNECT authority port.
		let mut inner = simple_bind();
		inner.address = "127.0.0.1:18083".parse().unwrap();

		let mut t = setup_proxy_test("{}")
			.unwrap()
			.with_backend(*mock.address())
			.with_bind(outer)
			.with_bind(inner)
			.with_route(basic_route(*mock.address()));
		// Authorize the inner request only when the custom header carried on the
		// (TLS-encrypted) CONNECT survived termination + re-entry.
		t.attach_route_policy(json!({
			"authorization": {
				"rules": ["source.connectHeaders[\"x-custom-header\"] == \"custom-value\""],
			},
		}))
		.await;

		// Complete the OUTER TLS handshake to the Connect bind. This is the core
		// assertion: a plaintext CONNECT bind would fail this handshake.
		let raw = t.serve_tunnel(strng::literal!("outer"));
		let client_tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
				hostname: Some(SNI.to_string()),
				insecure_host: true,
				..Default::default()
			}
			.try_into()
			.unwrap();
		let mut io = TlsConnector::from(client_tls.base_config().config)
			.connect(ServerName::try_from(SNI.to_string()).unwrap(), raw)
			.await
			.expect("outer TLS handshake should succeed (gateway terminates outer TLS before CONNECT)");

		// CONNECT inside the outer TLS, optionally carrying the custom header.
		let connect = match custom_header {
			Some(v) => format!(
				"CONNECT inner.local:18083 HTTP/1.1\r\nHost: inner.local:18083\r\nx-custom-header: {v}\r\n\r\n"
			),
			None => "CONNECT inner.local:18083 HTTP/1.1\r\nHost: inner.local:18083\r\n\r\n".to_string(),
		};
		io.write_all(connect.as_bytes()).await.unwrap();

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

		// Inner request over the tunnel (plaintext within the outer TLS).
		io.write_all(b"GET /foo HTTP/1.1\r\nHost: lo\r\nConnection: close\r\n\r\n")
			.await
			.unwrap();
		let mut tunneled = Vec::new();
		tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut tunneled))
			.await
			.expect("timed out waiting for tunneled HTTP response")
			.unwrap();
		String::from_utf8_lossy(&tunneled).to_string()
	}

	// Header present + matching: authorized (200) — proves the outer TLS was
	// terminated, the CONNECT was read inside it, and the header reached CEL.
	let allowed = tls_connect_inner(Some("custom-value")).await;
	assert!(
		allowed.starts_with("HTTP/1.1 200 OK\r\n"),
		"expected authorized inner request, got: {allowed}",
	);
	// Wrong / absent header: denied (403).
	let wrong = tls_connect_inner(Some("other-value")).await;
	assert!(
		wrong.starts_with("HTTP/1.1 403 Forbidden\r\n"),
		"expected denied inner request for wrong header, got: {wrong}",
	);
	let absent = tls_connect_inner(None).await;
	assert!(
		absent.starts_with("HTTP/1.1 403 Forbidden\r\n"),
		"expected denied inner request without the header, got: {absent}",
	);
}

/// End-to-end on-behalf-of (OBO) token exchange over a CONNECT tunnel.
///
/// CONNECT (carrying `x-actor-token`, the actor token) -> Tunnel re-entry ->
/// ext-authz builds an RFC 8693 token-exchange body from
/// `source.connectHeaders["x-actor-token"]` (actor) and
/// `request.headers["authorization"]` (subject) -> a mock STS returns an OBO JWT
/// -> `transformations` injects `Authorization: Bearer <obo>` -> the route
/// backend forwards it upstream.
///
/// Proves that `source.connectHeaders`, the ext-authz `http.body` CEL, the
/// ext-authz cache, transformations, and the re-entered request pipeline compose
/// into a working OBO injection. The STS is mocked, so this proves the gateway
/// wiring rather than STS semantics.
///
/// Note: this uses a fixed route backend rather than `dynamic: {}`; the OBO
/// injection (decrypted inner request) is orthogonal to how the destination is
/// resolved, so the simpler proven CONNECT-re-entry fixture is used here.
#[tokio::test]
async fn connect_tunnel_obo_exchange_injects_token() {
	use std::collections::HashMap;

	// An unsigned JWT (`header.payload.signature`) carrying a near-future `exp`,
	// so `unvalidatedJwtPayload(...).exp` resolves for the cache TTL. The gateway
	// never validates the signature.
	fn unsigned_jwt(payload: serde_json::Value) -> String {
		use base64::Engine;
		let b64 = base64::prelude::BASE64_URL_SAFE_NO_PAD;
		let header = b64.encode(br#"{"alg":"none","typ":"JWT"}"#);
		let body = b64.encode(serde_json::to_vec(&payload).unwrap());
		format!("{header}.{body}.")
	}

	fn parse_form(body: &[u8]) -> HashMap<String, String> {
		url::form_urlencoded::parse(body).into_owned().collect()
	}

	let exp = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
		+ 3600;
	let obo_jwt = unsigned_jwt(json!({"sub": "obo-subject", "exp": exp}));
	let obo_bearer = format!("Bearer {obo_jwt}");

	// Mock STS: records each token-exchange request and returns the OBO JWT as an
	// RFC 8693 token-exchange response.
	let sts = MockServer::start().await;
	let sts_bodies = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
	let sts_paths = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
	let sts_methods = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
	{
		let sts_bodies = sts_bodies.clone();
		let sts_paths = sts_paths.clone();
		let sts_methods = sts_methods.clone();
		let token = obo_jwt.clone();
		Mock::given(wiremock::matchers::any())
			.respond_with(move |req: &wiremock::Request| {
				sts_bodies.lock().unwrap().push(req.body.clone());
				sts_paths.lock().unwrap().push(req.url.path().to_string());
				sts_methods.lock().unwrap().push(req.method.to_string());
				ResponseTemplate::new(200).set_body_json(json!({
					"access_token": token,
					"issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
					"token_type": "Bearer",
				}))
			})
			.mount(&sts)
			.await;
	}

	// Mock upstream: records the `authorization` header it receives so we can
	// assert the OBO token was injected.
	let upstream = MockServer::start().await;
	let upstream_auth = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
	{
		let upstream_auth = upstream_auth.clone();
		Mock::given(wiremock::matchers::any())
			.respond_with(move |req: &wiremock::Request| {
				let auth = req
					.headers
					.get("authorization")
					.and_then(|v| v.to_str().ok())
					.map(str::to_string);
				upstream_auth.lock().unwrap().push(auth);
				ResponseTemplate::new(200)
			})
			.mount(&upstream)
			.await;
	}

	// Outer bind terminates the CONNECT tunnel; the inner (plain HTTP) bind is
	// re-entered by authority port and carries the OBO policies.
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15008".parse().unwrap();
	let mut inner = simple_bind();
	inner.address = "127.0.0.1:18080".parse().unwrap();
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_route(*upstream.address()))
		.with_connect_mode_on_port(frontend::ConnectMode::Tunnel, 15008);

	// ext-authz performs the RFC 8693 token exchange (actor from the CONNECT
	// header, subject from the inner request's authorization header), caches the
	// result keyed by subject+actor, and exposes the OBO token under `extauthz`.
	// The transformation then injects it as the outbound `Authorization` header.
	// Both token types are JWT: the enterprise STS `validateTokenType` accepts only
	// `urn:ietf:params:oauth:token-type:jwt` and rejects `access_token`/`id_token`.
	let body_expr = r#"form.encode({
		"grant_type": "urn:ietf:params:oauth:grant-type:token-exchange",
		"subject_token": request.headers["authorization"].regexReplace("^Bearer ", ""),
		"subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
		"actor_token": source.connectHeaders["x-actor-token"],
		"actor_token_type": "urn:ietf:params:oauth:token-type:jwt",
		"audience": "https://api.example.test"
	})"#;
	t.attach_route_policy(json!({
		"extAuthz": {
			"host": sts.address().to_string(),
			"cache": {
				"key": [
					"request.headers[\"authorization\"]",
					"source.connectHeaders[\"x-actor-token\"]"
				],
				"ttl": "extauthz.expires - 5",
				"maxEntries": 1024
			},
			"protocol": {
				"http": {
					"path": "\"/token\"",
					"addRequestHeaders": {
						"content-type": "\"application/x-www-form-urlencoded\"",
						":method": "\"POST\""
					},
					"body": body_expr,
					"metadata": {
						"token": "json(response.body).access_token",
						"expires": "unvalidatedJwtPayload(json(response.body).access_token).exp"
					}
				}
			}
		},
		"transformations": {
			"request": {
				"set": {
					"authorization": "\"Bearer \" + extauthz.token"
				}
			}
		}
	}))
	.await;

	// Open a fresh CONNECT tunnel, optionally carrying the actor token, and send
	// the inner request with the subject token. Returns the raw inner response.
	async fn tunnel_request(t: &TestBind, actor: Option<&str>, subject: &str) -> String {
		let mut io = t.serve_tunnel(strng::literal!("outer"));
		let connect = match actor {
			Some(a) => format!(
				"CONNECT api.example.test:18080 HTTP/1.1\r\nHost: api.example.test:18080\r\nx-actor-token: {a}\r\n\r\n"
			),
			None => "CONNECT api.example.test:18080 HTTP/1.1\r\nHost: api.example.test:18080\r\n\r\n"
				.to_string(),
		};
		io.write_all(connect.as_bytes()).await.unwrap();

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

		let inner = format!(
			"GET /foo HTTP/1.1\r\nHost: lo\r\nauthorization: Bearer {subject}\r\nConnection: close\r\n\r\n"
		);
		io.write_all(inner.as_bytes()).await.unwrap();
		let mut tunneled = Vec::new();
		tokio::time::timeout(Duration::from_secs(5), io.read_to_end(&mut tunneled))
			.await
			.expect("timed out waiting for tunneled HTTP response")
			.unwrap();
		String::from_utf8_lossy(&tunneled).to_string()
	}

	// 1. First request: the OBO token is exchanged and injected upstream.
	let resp1 = tunnel_request(&t, Some("actor-token-A"), "subject-token-S").await;
	assert!(
		resp1.starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected inner response: {resp1}",
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		1,
		"STS should be called once"
	);
	// Upstream sees the OBO bearer, not the original subject token.
	assert_eq!(
		upstream_auth.lock().unwrap().last().unwrap().as_deref(),
		Some(obo_bearer.as_str()),
	);
	// The STS request body is a well-formed RFC 8693 exchange built from both the
	// actor (CONNECT header) and subject (inner authorization header) tokens.
	let form = parse_form(&sts_bodies.lock().unwrap()[0]);
	assert_eq!(
		form.get("grant_type").map(String::as_str),
		Some("urn:ietf:params:oauth:grant-type:token-exchange"),
	);
	assert_eq!(
		form.get("subject_token").map(String::as_str),
		Some("subject-token-S"),
	);
	assert_eq!(
		form.get("actor_token").map(String::as_str),
		Some("actor-token-A"),
	);
	assert_eq!(
		form.get("audience").map(String::as_str),
		Some("https://api.example.test"),
	);
	assert_eq!(
		form.get("subject_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:jwt"),
	);
	assert_eq!(
		form.get("actor_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:jwt"),
	);
	assert_eq!(sts_paths.lock().unwrap()[0], "/token");
	assert_eq!(sts_methods.lock().unwrap()[0], "POST");

	// 2. Cache hit: same subject+actor on a fresh tunnel reuses the cached OBO
	// token without calling the STS again.
	let resp2 = tunnel_request(&t, Some("actor-token-A"), "subject-token-S").await;
	assert!(
		resp2.starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected inner response: {resp2}",
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		1,
		"cache hit: STS should not be called again",
	);
	assert_eq!(
		upstream_auth.lock().unwrap().last().unwrap().as_deref(),
		Some(obo_bearer.as_str()),
	);

	// 3. Cache miss: a different actor token (the cache key includes the actor)
	// triggers a new exchange.
	let resp3 = tunnel_request(&t, Some("actor-token-B"), "subject-token-S").await;
	assert!(
		resp3.starts_with("HTTP/1.1 200 OK\r\n"),
		"unexpected inner response: {resp3}",
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		2,
		"different actor should miss the cache",
	);
	let form_b = parse_form(&sts_bodies.lock().unwrap()[1]);
	assert_eq!(
		form_b.get("actor_token").map(String::as_str),
		Some("actor-token-B"),
	);

	// 4. Negative: without the actor token, the body expression fails to evaluate
	// (indexing the missing CONNECT header errors), so the request is rejected and
	// the STS is never called. This documents the dependency on the actor token.
	let resp4 = tunnel_request(&t, None, "subject-token-S").await;
	assert!(
		resp4.starts_with("HTTP/1.1 403 Forbidden\r\n"),
		"expected 403 without the actor token, got: {resp4}",
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		2,
		"rejected request must not call the STS",
	);
}

/// Full assembled MITM + OBO chain in one test:
///
/// CONNECT (carrying `x-actor-token`) -> Tunnel re-entry by authority port into a
/// dynamic-CA HTTPS bind -> inner TLS terminated with a minted per-SNI cert ->
/// ext-authz RFC 8693 exchange (actor from `source.connectHeaders`, subject from
/// the decrypted `authorization`, both JWT token types) -> mock STS returns the
/// OBO -> `transformations` injects `Authorization: Bearer <obo>` -> a
/// `dynamic: {}` (DFP) backend forwards to the upstream named by the inner Host.
///
/// This exercises the joints the OBO test and a plain authz test only prove
/// separately: (1) re-entry by port into a TLS-terminating bind, (2) dynamic-CA
/// minting a trusted per-SNI cert, (3) `ConnectHeaders` surviving
/// `maybe_terminate_tls` into the OBO body, and (4) the decrypted request flowing
/// into the `dynamic: {}` backend path.
#[cfg(feature = "tls-aws-lc")]
#[tokio::test]
async fn connect_tunnel_dynamic_ca_obo_dynamic_backend() {
	use std::collections::HashMap;

	// SNI drives dynamic-CA cert minting; it must be a hostname (rustls sends no SNI
	// for an IP). The DFP destination is taken from the inner request's `Host` and
	// uses the upstream mock's address, so the two legitimately differ in the test.
	const SNI: &str = "api.example.test";

	fn unsigned_jwt(payload: serde_json::Value) -> String {
		use base64::Engine;
		let b64 = base64::prelude::BASE64_URL_SAFE_NO_PAD;
		let header = b64.encode(br#"{"alg":"none","typ":"JWT"}"#);
		let body = b64.encode(serde_json::to_vec(&payload).unwrap());
		format!("{header}.{body}.")
	}
	fn parse_form(body: &[u8]) -> HashMap<String, String> {
		url::form_urlencoded::parse(body).into_owned().collect()
	}

	let _ = rustls::crypto::CryptoProvider::install_default(Arc::unwrap_or_clone(
		agentgateway::transport::tls::provider(),
	));

	let exp = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
		+ 3600;
	let obo_jwt = unsigned_jwt(json!({"sub": "obo-subject", "exp": exp}));
	let obo_bearer = format!("Bearer {obo_jwt}");

	// Mock STS: records the token-exchange request body and returns the OBO JWT.
	let sts = MockServer::start().await;
	let sts_bodies = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
	{
		let sts_bodies = sts_bodies.clone();
		let token = obo_jwt.clone();
		Mock::given(wiremock::matchers::any())
			.respond_with(move |req: &wiremock::Request| {
				sts_bodies.lock().unwrap().push(req.body.clone());
				ResponseTemplate::new(200).set_body_json(json!({
					"access_token": token,
					"issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
					"token_type": "Bearer",
				}))
			})
			.mount(&sts)
			.await;
	}

	// Mock upstream: the `dynamic: {}` destination. Records the `authorization`
	// header it receives so we can confirm the OBO was injected.
	let upstream = MockServer::start().await;
	let upstream_addr = upstream.address().to_string();
	let upstream_auth = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
	{
		let upstream_auth = upstream_auth.clone();
		Mock::given(wiremock::matchers::any())
			.respond_with(move |req: &wiremock::Request| {
				let auth = req
					.headers
					.get("authorization")
					.and_then(|v| v.to_str().ok())
					.map(str::to_string);
				upstream_auth.lock().unwrap().push(auth);
				ResponseTemplate::new(200)
			})
			.mount(&upstream)
			.await;
	}

	// Generate a self-signed CA; the dynamic-CA inner bind mints a leaf per SNI, and
	// the inner TLS client trusts it as its root.
	let ca_key = rcgen::KeyPair::generate().expect("generate CA key");
	let mut ca_params = rcgen::CertificateParams::default();
	ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
	let ca_cert = ca_params.self_signed(&ca_key).expect("generate CA cert");
	let ca_cert_pem = ca_cert.pem().into_bytes();

	let tls_config = agentgateway::types::agent::ServerTLSConfig::dynamic_ca_with_profile(
		ca_cert_pem.clone(),
		ca_key.serialize_pem().into_bytes(),
		vec![b"http/1.1".to_vec()],
		None,
		None,
		None,
		None,
		Default::default(),
	)
	.expect("build dynamic CA TLS config");

	// Outer bind terminates the CONNECT tunnel via the bind-level `tunnelProtocol: Connect`
	let mut outer = simple_bind();
	outer.key = strng::literal!("outer");
	outer.address = "127.0.0.1:15010".parse().unwrap();
	outer.tunnel_protocol = TunnelProtocol::Connect;
	// Inner bind terminates TLS with the dynamic CA (empty hostname = match any SNI).
	let inner = Bind {
		key: BIND_KEY,
		address: "127.0.0.1:18082".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: Default::default(),
			protocol: ListenerProtocol::HTTPS(tls_config),
		}]),
		protocol: BindProtocol::tls,
		tunnel_protocol: Default::default(),
		mode: Default::default(),
	};

	// `dynamic: {}` backend: the upstream is resolved from the decrypted request's
	// Host/authority (DFP), proven on direct requests by `dfp_uses_host_port`.
	let dynamic_backend = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), ());
	let t = setup_proxy_test("{}").unwrap();
	t.inputs()
		.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	let mut t = t
		.with_bind(outer)
		.with_bind(inner)
		.with_route(basic_named_route("/dynamic".into()));

	// Both token types are JWT (the enterprise STS rejects `access_token`); the
	// actor is the surviving CONNECT header, the subject the decrypted authorization.
	let body_expr = r#"form.encode({
		"grant_type": "urn:ietf:params:oauth:grant-type:token-exchange",
		"subject_token": request.headers["authorization"].regexReplace("^Bearer ", ""),
		"subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
		"actor_token": source.connectHeaders["x-actor-token"],
		"actor_token_type": "urn:ietf:params:oauth:token-type:jwt",
		"audience": request.host
	})"#;
	t.attach_route_policy(json!({
		"extAuthz": {
			"host": sts.address().to_string(),
			"protocol": {
				"http": {
					"path": "\"/token\"",
					"addRequestHeaders": {
						"content-type": "\"application/x-www-form-urlencoded\"",
						":method": "\"POST\""
					},
					"body": body_expr,
					"metadata": {
						"token": "json(response.body).access_token"
					}
				}
			}
		},
		"transformations": {
			"request": {
				"set": {
					"authorization": "\"Bearer \" + extauthz.token"
				}
			}
		}
	}))
	.await;

	// Open a CONNECT tunnel (optionally carrying the actor token), run an inner TLS
	// handshake over the tunnel trusting the generated CA, then send an HTTPS request
	// whose Host names the DFP upstream. The handshake is expected to succeed in all
	// cases; the status reflects whether the OBO exchange (and thus the actor header)
	// completed.
	async fn tls_tunnel_request(
		t: &TestBind,
		ca_pem: &[u8],
		upstream_host: &str,
		actor: Option<&str>,
	) -> StatusCode {
		let mut io = t.serve_tunnel(strng::literal!("outer"));
		let connect = match actor {
			Some(a) => {
				format!("CONNECT {SNI}:18082 HTTP/1.1\r\nHost: {SNI}:18082\r\nx-actor-token: {a}\r\n\r\n")
			},
			None => format!("CONNECT {SNI}:18082 HTTP/1.1\r\nHost: {SNI}:18082\r\n\r\n"),
		};
		io.write_all(connect.as_bytes()).await.unwrap();

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

		// Inner TLS handshake as a client trusting the generated CA. Success proves
		// re-entry-by-port reached the dynamic-CA bind and it minted a trusted cert
		// for the SNI.
		let client_tls: agentgateway::http::backendtls::BackendTLS =
			agentgateway::http::backendtls::ResolvedBackendTLS {
				root: Some(ca_pem.to_vec()),
				hostname: Some(SNI.to_string()),
				alpn: Some(vec!["http/1.1".to_string()]),
				..Default::default()
			}
			.try_into()
			.unwrap();
		let tls = TlsConnector::from(client_tls.base_config().config)
			.connect(ServerName::try_from(SNI.to_string()).unwrap(), io)
			.await
			.expect("inner TLS handshake should succeed (dynamic CA mints a trusted cert)");

		let (mut sender, conn) = http1::handshake(TokioIo::new(tls)).await.unwrap();
		let conn = tokio::spawn(conn);
		let res = sender
			.send_request(
				::http::Request::builder()
					.method(Method::GET)
					.uri("/foo")
					.header(header::HOST, upstream_host)
					.header(header::AUTHORIZATION, "Bearer subject-token-S")
					.header(header::CONNECTION, "close")
					.body(Body::empty())
					.unwrap(),
			)
			.await
			.unwrap();
		let status = res.status();
		conn.abort();
		status
	}

	// Actor token present: the handshake succeeds, the OBO is exchanged (JWT types)
	// and injected, and the DFP upstream receives `Bearer <obo>` (200).
	assert_eq!(
		tls_tunnel_request(&t, &ca_cert_pem, &upstream_addr, Some("actor-token-A")).await,
		StatusCode::OK,
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		1,
		"STS should be called once"
	);
	assert_eq!(
		upstream_auth.lock().unwrap().last().unwrap().as_deref(),
		Some(obo_bearer.as_str()),
		"DFP upstream must receive the injected OBO token",
	);
	// The exchange body is well-formed: actor from the surviving CONNECT header,
	// subject from the decrypted request, JWT token types, audience = inner Host.
	let form = parse_form(&sts_bodies.lock().unwrap()[0]);
	assert_eq!(
		form.get("grant_type").map(String::as_str),
		Some("urn:ietf:params:oauth:grant-type:token-exchange"),
	);
	assert_eq!(
		form.get("subject_token").map(String::as_str),
		Some("subject-token-S"),
	);
	assert_eq!(
		form.get("actor_token").map(String::as_str),
		Some("actor-token-A"),
	);
	assert_eq!(
		form.get("subject_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:jwt"),
	);
	assert_eq!(
		form.get("actor_token_type").map(String::as_str),
		Some("urn:ietf:params:oauth:token-type:jwt"),
	);
	assert_eq!(
		form.get("audience").map(String::as_str),
		Some(upstream_addr.as_str())
	);

	// Actor token absent: the handshake still succeeds (TLS termination is
	// independent of the actor), but the OBO body fails to evaluate, so the request
	// is rejected (403) and neither the STS nor the upstream sees a second call.
	assert_eq!(
		tls_tunnel_request(&t, &ca_cert_pem, &upstream_addr, None).await,
		StatusCode::FORBIDDEN,
	);
	assert_eq!(
		sts_bodies.lock().unwrap().len(),
		1,
		"rejected request must not call the STS",
	);
	assert_eq!(
		upstream_auth.lock().unwrap().len(),
		1,
		"rejected request must not reach the upstream",
	);
}

#[tokio::test]
async fn incoming_connect_applies_backend_tls() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = agentgateway::http::backendtls::ResolvedBackendTLS {
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
