use ::http::{Method, StatusCode, Version};
use agent_core::strng;
use assert_matches::assert_matches;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use rand::Rng;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use x509_parser::nom::AsBytes;

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

async fn assert_llm(io: Client<MemoryConnector, Body>, body: &[u8], want: Value) {
	let r = rand::rng().random::<u128>();
	let res = send_request_body(io.clone(), Method::POST, &format!("http://lo/{r}"), body).await;

	// Ensure body finishes
	let _ = res.into_body().collect().await.unwrap();
	let logs = check_eventually(
		Duration::from_secs(1),
		|| async {
			agent_core::telemetry::testing::find(&[("scope", "request"), ("http.path", &format!("/{r}"))])
				.to_vec()
		},
		|log| log.len() == 1,
	)
	.await
	.unwrap();
	let log = logs.first().unwrap();
	let valid = is_json_subset(&want, log);
	assert!(valid, "want={want:#?} got={log:#?}");
}

fn deser<T: DeserializeOwned>(v: serde_json::Value) -> T {
	serde_json::from_value(v).unwrap()
}
