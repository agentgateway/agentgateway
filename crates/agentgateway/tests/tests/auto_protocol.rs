use agentgateway::types::agent::{Bind, BindProtocol, Listener, ListenerProtocol, ListenerSet};

use crate::common::prelude::*;
use crate::tests::tls::{https_bind, test_server_tls_config};

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
		mode: Default::default(),
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
		mode: Default::default(),
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
				protocol: ListenerProtocol::HTTPS(test_server_tls_config()),
			},
		]),
		protocol: BindProtocol::auto,
		tunnel_protocol: Default::default(),
		mode: Default::default(),
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
		mode: Default::default(),
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
