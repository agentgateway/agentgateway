use ::http::{Method, StatusCode, Version};
use agent_core::strng;
use assert_matches::assert_matches;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use rand::RngExt;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use x509_parser::nom::AsBytes;

use crate::http::cors;
use crate::http::tests_common::*;
use crate::http::transformation_cel::Transformation;
use crate::http::{Body, Response, transformation_cel};
use crate::llm::{AIProvider, openai};
use crate::proxy::request_builder::RequestBuilder;
use crate::test_helpers::proxymock::*;
use crate::types::agent::{
	Backend, BackendPolicy, BackendReference, BackendWithPolicies, Bind, BindProtocol, Listener,
	ListenerProtocol, ListenerSet, PathMatch, PolicyTarget, ResourceName, Route,
	RouteBackendReference, RouteMatch, RouteName, RouteSet, Target, TargetedPolicy, TrafficPolicy,
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
	let (_mock, bind, io) = basic_setup().await;
	let _bind = bind.with_policy(TargetedPolicy {
		key: strng::new("rl"),
		name: None,
		target: PolicyTarget::Route(RouteName {
			name: "route".into(),
			namespace: "".into(),
			rule_name: None,
			kind: None,
		}),
		policy: TrafficPolicy::LocalRateLimit(vec![
			http::localratelimit::RateLimitSpec {
				max_tokens: 1,
				tokens_per_fill: 1,
				fill_interval: Duration::from_secs(1),
				limit_type: Default::default(),
			}
			.try_into()
			.unwrap(),
		])
		.into(),
	});

	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let res = send_request(io.clone(), Method::GET, "http://lo").await;
	assert_eq!(res.status(), 429);
}

/// Helper to build a CORS policy for tests.
fn cors_policy(allow_origins: Vec<&str>, allow_methods: Vec<&str>) -> cors::Cors {
	cors::Cors::try_from(cors::CorsSerde {
		allow_credentials: false,
		allow_headers: vec!["*".to_string()],
		allow_methods: allow_methods.into_iter().map(String::from).collect(),
		allow_origins: allow_origins.into_iter().map(String::from).collect(),
		expose_headers: vec![],
		max_age: None,
	})
	.unwrap()
}

/// Verifies that a CORS preflight (OPTIONS) request returns 200 even when
/// the rate limit is exhausted, because CORS runs before authentication and rate limiting.
#[tokio::test]
async fn cors_preflight_bypasses_ratelimit() {
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	// Attach CORS + rate limit (1 token, essentially immediately exhausted after first real request)
	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("rl"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::LocalRateLimit(vec![
				http::localratelimit::RateLimitSpec {
					max_tokens: 1,
					tokens_per_fill: 1,
					fill_interval: Duration::from_secs(100),
					limit_type: Default::default(),
				}
				.try_into()
				.unwrap(),
			])
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("rl"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::LocalRateLimit(vec![
				http::localratelimit::RateLimitSpec {
					max_tokens: 1,
					tokens_per_fill: 1,
					fill_interval: Duration::from_secs(100),
					limit_type: Default::default(),
				}
				.try_into()
				.unwrap(),
			])
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("apikey"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::APIKey(
				http::apikey::LocalAPIKeys {
					keys: vec![http::apikey::LocalAPIKey {
						key: http::apikey::APIKey::new("sk-123"),
						metadata: None,
					}],
					mode: http::apikey::Mode::Strict,
				}
				.into(),
			)
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("basic"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::BasicAuth(
				http::basicauth::LocalBasicAuth {
					htpasswd: FileOrInline::Inline("user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00".to_string()),
					realm: Some("my-realm".into()),
					mode: http::basicauth::Mode::Strict,
				}
				.try_into()
				.unwrap(),
			)
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("apikey"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::APIKey(
				http::apikey::LocalAPIKeys {
					keys: vec![http::apikey::LocalAPIKey {
						key: http::apikey::APIKey::new("sk-123"),
						metadata: Some(json!({"group": "eng"})),
					}],
					mode: http::apikey::Mode::Strict,
				}
				.into(),
			)
			.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("auth"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::Authorization(deser(json!({
				"rules": ["apiKey.group == 'admin'"]
			})))
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let route_target = PolicyTarget::Route(RouteName {
		name: "route".into(),
		namespace: "".into(),
		rule_name: None,
		kind: None,
	});

	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("cors"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::CORS(cors_policy(vec!["http://example.com"], vec!["GET", "POST"]))
				.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("apikey"),
			name: None,
			target: route_target.clone(),
			policy: TrafficPolicy::APIKey(
				http::apikey::LocalAPIKeys {
					keys: vec![http::apikey::LocalAPIKey {
						key: http::apikey::APIKey::new("sk-123"),
						metadata: Some(json!({"group": "eng"})),
					}],
					mode: http::apikey::Mode::Strict,
				}
				.into(),
			)
			.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("auth"),
			name: None,
			target: route_target,
			policy: TrafficPolicy::Authorization(deser(json!({
				"rules": ["apiKey.group == 'admin'"]
			})))
			.into(),
		});

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
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
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
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
}

#[tokio::test]
async fn llm_openai_tokenize() {
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
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
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
}

#[tokio::test]
async fn llm_log_body() {
	let mock = body_mock(include_bytes!("../llm/tests/response_basic.json")).await;
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
	assert_llm(io, include_bytes!("../llm/tests/request_basic.json"), want).await;
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
	let mock = simple_mock().await;
	let xfm = transformation_cel::LocalTransformationConfig {
		response: Some(transformation_cel::LocalTransform {
			add: vec![("x-xfm".into(), "\"x-xfm-val\"".into())],
			..Default::default()
		}),
		request: None,
	};
	let xfm = Transformation::try_from_local_config(xfm, true).unwrap();
	let bind = base_gateway(&mock).with_route(Route {
		key: "route2".into(),
		name: RouteName {
			name: "route2".into(),
			namespace: Default::default(),
			rule_name: None,
			kind: None,
		},
		hostnames: Default::default(),
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/p".into()),
			method: None,
			query: vec![],
		}],
		inline_policies: vec![
			TrafficPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
				add: vec![("x-filter".into(), "x-filter-val".into())],
				set: vec![],
				remove: vec![],
			}),
			TrafficPolicy::DirectResponse(crate::http::filters::DirectResponse {
				body: Bytes::from_static(b"hello"),
				status: StatusCode::UNPROCESSABLE_ENTITY,
			}),
			TrafficPolicy::Transformation(xfm),
		],
		backends: vec![],
	});
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 422);
	// Each type of response modifier should still run even though its a direct response
	assert_eq!(res.hdr("x-filter"), "x-filter-val");
	assert_eq!(res.hdr("x-xfm"), "x-xfm-val");
	assert_eq!(
		http::read_body_with_limit(res.into_body(), 100)
			.await
			.unwrap()
			.as_bytes(),
		b"hello"
	);
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
	let mock = simple_mock().await;
	let bind = base_gateway(&mock).with_route(Route {
		key: "route2".into(),
		name: RouteName {
			name: "route2".into(),
			namespace: Default::default(),
			rule_name: None,
			kind: None,
		},
		hostnames: Default::default(),
		matches: vec![RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/p".into()),
			method: None,
			query: vec![],
		}],
		inline_policies: vec![
			TrafficPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
				add: vec![("x-route-req".into(), "route-req".into())],
				set: vec![],
				remove: vec![],
			}),
			TrafficPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
				add: vec![("x-route-resp".into(), "route-resp".into())],
				set: vec![],
				remove: vec![],
			}),
		],
		backends: vec![RouteBackendReference {
			weight: 1,
			backend: BackendReference::Backend(strng::format!("/{}", mock.address())),
			inline_policies: vec![
				BackendPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-backend-req".into(), "backend-req".into())],
					set: vec![],
					remove: vec![],
				}),
				BackendPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-backend-resp".into(), "backend-resp".into())],
					set: vec![],
					remove: vec![],
				}),
			],
		}],
	});
	let io = bind.serve_http(BIND_KEY);

	let res = send_request(io.clone(), Method::GET, "http://lo/p").await;
	assert_eq!(res.status(), 200);
	assert_eq!(res.hdr("x-route-resp"), "route-resp");
	assert_eq!(res.hdr("x-backend-resp"), "backend-resp");
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body.headers.get("x-route-req").unwrap().as_bytes(),
		b"route-req"
	);
	assert_eq!(
		body.headers.get("x-backend-req").unwrap().as_bytes(),
		b"backend-req"
	);
}

#[tokio::test]
async fn inline_backend_policies() {
	let mock = simple_mock().await;
	let bind = base_gateway(&mock)
		.with_route(Route {
			key: "route2".into(),
			name: RouteName {
				name: "route2".into(),
				namespace: Default::default(),
				rule_name: None,
				kind: None,
			},
			hostnames: Default::default(),
			matches: vec![RouteMatch {
				headers: vec![],
				path: PathMatch::PathPrefix("/p".into()),
				method: None,
				query: vec![],
			}],
			inline_policies: vec![
				TrafficPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-route-req".into(), "route-req".into())],
					set: vec![],
					remove: vec![],
				}),
				TrafficPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-route-resp".into(), "route-resp".into())],
					set: vec![],
					remove: vec![],
				}),
			],
			backends: vec![RouteBackendReference {
				weight: 1,
				backend: BackendReference::Backend(strng::format!("/{}", mock.address())),
				inline_policies: vec![
					BackendPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
						add: vec![("x-backend-route-req".into(), "backend-route-req".into())],
						set: vec![],
						remove: vec![],
					}),
					BackendPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
						add: vec![("x-backend-route-resp".into(), "backend-route-resp".into())],
						set: vec![],
						remove: vec![],
					}),
				],
			}],
		})
		.with_raw_backend(BackendWithPolicies {
			backend: Backend::Opaque(
				ResourceName::new(strng::format!("{}", mock.address()), "".into()),
				Target::Address(*mock.address()),
			),
			inline_policies: vec![
				BackendPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-backend-req".into(), "backend-req".into())],
					set: vec![],
					remove: vec![],
				}),
				BackendPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
					add: vec![("x-backend-resp".into(), "backend-resp".into())],
					set: vec![],
					remove: vec![],
				}),
			],
		});
	let io = bind.serve_http(BIND_KEY);

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
async fn api_key() {
	let (_mock, bind, io) = basic_setup().await;
	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("apikey"),
			name: Default::default(),
			target: PolicyTarget::Route(RouteName {
				name: "route".into(),
				namespace: "".into(),
				rule_name: None,
				kind: None,
			}),
			policy: TrafficPolicy::APIKey(
				http::apikey::LocalAPIKeys {
					keys: vec![
						http::apikey::LocalAPIKey {
							key: http::apikey::APIKey::new("sk-123"),
							metadata: Some(json!({"group": "eng"})),
						},
						http::apikey::LocalAPIKey {
							key: http::apikey::APIKey::new("sk-456"),
							metadata: Some(json!({"group": "sales"})),
						},
					],
					mode: http::apikey::Mode::Strict,
				}
				.into(),
			)
			.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("auth"),
			name: Default::default(),
			target: PolicyTarget::Route(RouteName {
				name: "route".into(),
				namespace: "".into(),
				rule_name: None,
				kind: None,
			}),
			policy: TrafficPolicy::Authorization(deser(json!({
				"rules": ["apiKey.group == 'eng'"]
			})))
			.into(),
		});

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
	let (_mock, bind, io) = basic_setup().await;
	let _bind = bind
		.with_policy(TargetedPolicy {
			key: strng::new("basic"),
			name: None,
			target: PolicyTarget::Route(RouteName {
				name: "route".into(),
				namespace: "".into(),
				rule_name: None,
				kind: None,
			}),
			policy: TrafficPolicy::BasicAuth(
				http::basicauth::LocalBasicAuth {
					htpasswd: FileOrInline::Inline(
						"user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00
bcrypt_test:$2y$05$nC6nErr9XZJuMJ57WyCob.EuZEjylDt2KaHfbfOtyb.EgL1I2jCVa
sha1_test:{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=
crypt_test:bGVh02xkuGli2"
							.to_string(),
					),
					realm: Some("my-realm".into()),
					mode: http::basicauth::Mode::Strict,
				}
				.try_into()
				.unwrap(),
			)
			.into(),
		})
		.with_policy(TargetedPolicy {
			key: strng::new("auth"),
			name: Default::default(),
			target: PolicyTarget::Route(RouteName {
				name: "route".into(),
				namespace: "".into(),
				rule_name: None,
				kind: None,
			}),
			policy: TrafficPolicy::Authorization(deser(json!({
				"rules": ["basicAuth.username == 'user'"]
			})))
			.into(),
		});

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
					hostname: strng::new("waypoint"),
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

fn deser<T: DeserializeOwned>(v: serde_json::Value) -> T {
	serde_json::from_value(v).unwrap()
}
