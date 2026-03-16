use ::http::{Method, Version};
use agent_core::strng;
use assert_matches::assert_matches;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use rand::RngExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use x509_parser::nom::AsBytes;

use crate::http::tests_common::*;
use crate::http::{Body, Response};
use crate::llm::{AIProvider, openai};
use crate::proxy::request_builder::RequestBuilder;
use crate::read_body;
use crate::test_helpers::proxymock::*;
use crate::types::agent::{
	Backend, BackendPolicy, BackendWithPolicies, Bind, BindProtocol, Listener, ListenerProtocol,
	ListenerSet, ResourceName, RouteSet, Target,
};
use crate::types::backend;
use crate::*;

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
async fn multiple_requests() {
	let (_mock, _bind, io) = basic_setup().await;
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn basic_http2() {
	let mock = simple_mock().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(simple_bind(basic_route(*mock.address())));
	let io = t.serve_http2(strng::new("bind"));
	let res = RequestBuilder::new(Method::GET, "http://lo")
		.version(Version::HTTP_2)
		.send(io)
		.await
		.unwrap();
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);
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
async fn tls_termination() {
	let mock = simple_mock().await;
	let route = basic_route(*mock.address());
	let bind = Bind {
		key: BIND_KEY,
		// not really used
		address: "127.0.0.1:0".parse().unwrap(),
		listeners: ListenerSet::from_list([Listener {
			key: LISTENER_KEY,
			name: Default::default(),
			hostname: strng::new("*.example.com"),
			protocol: ListenerProtocol::HTTPS(
				types::local::LocalTLSServerConfig {
					cert: "../../examples/tls/certs/cert.pem".into(),
					key: "../../examples/tls/certs/key.pem".into(),
					root: None,
					cipher_suites: None,
					min_tls_version: None,
					max_tls_version: None,
				}
				.try_into()
				.unwrap(),
			),
			tcp_routes: Default::default(),
			routes: RouteSet::from_list(vec![route]),
		}]),
		protocol: BindProtocol::tls,
		tunnel_protocol: Default::default(),
	};

	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*mock.address())
		.with_bind(bind);

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
			inline_policies: vec![BackendPolicy::BackendTLS(backend_tls)],
		})
		.with_bind(simple_bind(basic_route(*mock.address())));

	let res = send_http_version(&t, Version::HTTP_2).await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);

	let res = send_http_version(&t, Version::HTTP_11).await;
	assert_eq!(res.status(), 200);
	assert_eq!(read_body(res.into_body()).await.version, Version::HTTP_2);
}

#[tokio::test]
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
			inline_policies: vec![BackendPolicy::BackendTLS(backend_tls)],
		})
		.with_bind(simple_bind(basic_route(*mock.address())));

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
				BackendPolicy::BackendTLS(backend_tls),
				BackendPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind(basic_route(*mock.address())));

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
				BackendPolicy::BackendTLS(backend_tls),
				BackendPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind(basic_route(*mock.address())));

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
				BackendPolicy::BackendTLS(backend_tls),
				BackendPolicy::HTTP(backend_version),
			],
		})
		.with_bind(simple_bind(basic_route(*mock.address())));

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
		b"Basic my-key"
	);
}

#[tokio::test]
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
					"key": "my-key"
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

/// Helper to set up a DFP test: creates a Dynamic backend and a route pointing to it.
fn setup_dfp() -> (TestBind, Client<MemoryConnector, Body>) {
	let backend_name = ResourceName::new("dynamic".into(), "".into());
	let dynamic_backend = Backend::Dynamic(backend_name, ());

	let route = basic_named_route("/dynamic".into());

	let t = setup_proxy_test("{}").unwrap();
	let pi = t.inputs();
	pi.stores
		.binds
		.write()
		.insert_backend(dynamic_backend.name(), dynamic_backend.into());
	let t = t.with_bind(simple_bind(route));
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
					cert: "../../examples/tls/certs/cert.pem".into(),
					key: "../../examples/tls/certs/key.pem".into(),
					root: None,
					cipher_suites: None,
					min_tls_version: None,
					max_tls_version: None,
				}
				.try_into()
				.unwrap(),
			),
			tcp_routes: Default::default(),
			routes: RouteSet::from_list(vec![route]),
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
	let t = t.with_bind(bind);
	let io = t.serve_https(BIND_KEY, None);
	(t, io)
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
