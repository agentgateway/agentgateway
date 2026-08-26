use hyper::client::conn::http1;
use rustls_pki_types::ServerName;
use tokio_rustls::TlsConnector;

use crate::common::prelude::*;

pub(in crate::tests) fn test_server_tls_config() -> agentgateway::types::agent::ServerTLSConfig {
	agentgateway::types::agent::ServerTLSConfig::from_pem_with_profile(
		include_bytes!("../../../../examples/mcp-tls/certs/cert.pem").to_vec(),
		include_bytes!("../../../../examples/mcp-tls/certs/key.pem").to_vec(),
		None,
		vec![b"h2".to_vec(), b"http/1.1".to_vec()],
		None,
		None,
		None,
		None,
		false,
	)
	.expect("test server tls config")
}

pub(in crate::tests) fn https_bind() -> BindSnapshot {
	BindSnapshot::new(
		Bind {
			key: BIND_KEY,
			address: "127.0.0.1:0".parse().unwrap(),
			protocol: BindProtocol::tls,
			tunnel_protocol: Default::default(),
			mode: Default::default(),
		},
		ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: strng::new("*.example.com"),
			protocol: ListenerProtocol::HTTPS(test_server_tls_config()),
		}]),
	)
}

async fn serve_https_http1_connection(
	t: &TestBind,
	sni: &str,
) -> (
	http1::SendRequest<Body>,
	tokio::task::JoinHandle<Result<(), hyper::Error>>,
) {
	let io = t.serve(BIND_KEY);
	let tls: agentgateway::http::backendtls::BackendTLS =
		agentgateway::http::backendtls::ResolvedBackendTLS {
			cert: None,
			key: None,
			root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
			hostname: Some(sni.to_string()),
			insecure: false,
			insecure_host: true,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			spiffe: false,
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
			protocol: ListenerProtocol::HTTPS(agentgateway::types::agent::ServerTLSConfig::new_invalid()),
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
#[cfg(feature = "crypto-aws-lc")]
async fn tls_backend_connection() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = agentgateway::http::backendtls::ResolvedBackendTLS {
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
#[cfg(feature = "crypto-aws-lc")]
async fn tls_backend_connection_alpn() {
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
#[cfg(feature = "crypto-aws-lc")]
async fn tls_backend_http2_version() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = agentgateway::http::backendtls::ResolvedBackendTLS {
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
#[cfg(feature = "crypto-aws-lc")]
async fn tls_backend_http1_version() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = agentgateway::http::backendtls::ResolvedBackendTLS {
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
#[cfg(feature = "crypto-aws-lc")]
async fn tls_backend_version_with_alpn() {
	let (mock, certs) = tls_mock().await;
	let backend_tls = agentgateway::http::backendtls::ResolvedBackendTLS {
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

pub fn route_with_prefix(target: std::net::SocketAddr, prefix: &str) -> Route {
	let mut route = basic_route(target);
	route.matches = vec![RouteMatch {
		headers: vec![],
		path: PathMatch::PathPrefix(prefix.into()),
		method: None,
		query: vec![],
	}];
	route
}

fn tls_passthrough_bind() -> BindSnapshot {
	BindSnapshot::new(
		Bind {
			key: BIND_KEY,
			address: "127.0.0.1:0".parse().unwrap(),
			protocol: BindProtocol::tls,
			tunnel_protocol: Default::default(),
			mode: Default::default(),
		},
		ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			// Use the default gateway name so frontend policies attach to this listener.
			name: agentgateway::types::agent::ListenerName {
				gateway_name: strng::literal!("default"),
				gateway_namespace: strng::literal!("default"),
				listener_name: strng::EMPTY,
				listener_set: None,
			},
			hostname: strng::new("*.example.com"),
			// No TLS config: passthrough
			protocol: ListenerProtocol::TLS(None),
		}]),
	)
}

fn hbone_identity(service_account: &str) -> agentgateway::transport::tls::TlsInfo {
	agentgateway::transport::tls::TlsInfo {
		identity: Some(agentgateway::transport::tls::IstioIdentity::new(
			strng::literal!("cluster.local"),
			strng::literal!("default"),
			strng::new(service_account),
		)),
		..Default::default()
	}
}

/// Start a plain TCP backend that captures the first bytes it receives. For TLS passthrough
/// the gateway forwards the raw TLS bytes, so on success the backend sees the ClientHello.
async fn passthrough_capture_backend() -> (std::net::SocketAddr, oneshot::Receiver<Vec<u8>>) {
	let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = backend.local_addr().unwrap();
	let (tx, rx) = oneshot::channel();
	tokio::spawn(async move {
		if let Ok((mut conn, _)) = backend.accept().await {
			let mut buf = vec![0u8; 5];
			if conn.read_exact(&mut buf).await.is_ok() {
				let _ = tx.send(buf);
			}
		}
	});
	(addr, rx)
}

/// Drive a TLS ClientHello from the client side of the duplex. The handshake never completes
/// (the backend is only a byte sink), we just need the gateway to sniff the SNI, run
/// authorization, and (on allow) forward the raw bytes.
fn spawn_client_hello(io: tokio::io::DuplexStream) -> tokio::task::JoinHandle<bool> {
	let tls: agentgateway::http::backendtls::BackendTLS =
		agentgateway::http::backendtls::ResolvedBackendTLS {
			cert: None,
			key: None,
			root: Some(include_bytes!("../../../../examples/mcp-tls/certs/ca-cert.pem").to_vec()),
			hostname: Some("a.example.com".to_string()),
			insecure: false,
			insecure_host: true,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			spiffe: false,
		}
		.try_into()
		.unwrap();
	tokio::spawn(async move {
		TlsConnector::from(tls.base_config().config)
			.connect(ServerName::try_from("a.example.com").unwrap(), io)
			.await
			.is_ok()
	})
}

/// A TLS passthrough listener must preserve the mTLS identity authenticated by an outer HBONE
/// layer, so authorization policies asserting on `source.identity` still apply
/// (https://github.com/agentgateway/agentgateway regression: passthrough dropped the identity).
#[tokio::test]
async fn tls_passthrough_preserves_hbone_identity_for_authz() {
	let (backend_addr, rx) = passthrough_capture_backend().await;

	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(backend_addr)
		.with_bind(tls_passthrough_bind())
		.with_tcp_route(basic_named_tcp_route(strng::format!("/{}", backend_addr)));
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": [r#"source.identity.serviceAccount == "test-sa""#],
		},
	}))
	.await;

	let io = t.serve_with_src_identity(BIND_KEY, hbone_identity("test-sa"));
	let _client = spawn_client_hello(io);

	// The identity matches the policy, so the raw TLS bytes must reach the backend.
	let first = tokio::time::timeout(Duration::from_secs(5), rx)
		.await
		.expect("timed out: passthrough bytes never reached the backend (identity dropped?)")
		.expect("backend closed without receiving data");
	assert_eq!(first[0], 0x16, "expected a TLS handshake record");
}

/// Inverse of the allow test: a non-matching HBONE identity must be denied by the
/// authorization policy on a TLS passthrough listener.
#[tokio::test]
async fn tls_passthrough_hbone_identity_authz_deny() {
	let (backend_addr, rx) = passthrough_capture_backend().await;

	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(backend_addr)
		.with_bind(tls_passthrough_bind())
		.with_tcp_route(basic_named_tcp_route(strng::format!("/{}", backend_addr)));
	t.attach_frontend_policy(json!({
		"networkAuthorization": {
			"rules": [r#"source.identity.serviceAccount == "test-sa""#],
		},
	}))
	.await;

	let io = t.serve_with_src_identity(BIND_KEY, hbone_identity("other-sa"));
	let client = spawn_client_hello(io);

	// The gateway denies the connection, so the client's handshake fails...
	let connected = client.await.unwrap();
	assert!(!connected, "handshake must fail when authorization denies");
	// ...and the backend never receives any bytes.
	assert!(
		tokio::time::timeout(Duration::from_millis(500), rx)
			.await
			.is_err(),
		"backend must not receive bytes for a denied connection"
	);
}
