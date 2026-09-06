use std::time::Duration;

use agent_core::strng;
use http::Method;
use http_body_util::BodyExt;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::http::Response;
use crate::llm::custom::ProviderFormat;
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, basic_route, custom_llm_backend, send_request_body,
	send_request_headers, setup_proxy_test, simple_bind,
};
use crate::types::agent::SimpleBackendReference;

const DELAY: Duration = Duration::from_millis(400);

async fn slow_upstream() -> MockServer {
	let server = MockServer::start().await;
	Mock::given(method("GET"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_string("ok")
				.set_delay(DELAY),
		)
		.mount(&server)
		.await;
	server
}

async fn drain(resp: Response) -> u16 {
	let status = resp.status().as_u16();
	let _ = resp.into_body().collect().await;
	status
}

/// Retry until the slot held by a finished response has been released by the proxy.
async fn eventually_admitted(t: &TestBind, headers: &[(&str, &str)]) -> u16 {
	for _ in 0..20 {
		let resp = send_request_headers(
			t.serve_http(BIND_KEY),
			Method::GET,
			"http://localhost/",
			headers,
		)
		.await;
		if resp.status() != 429 {
			return drain(resp).await;
		}
		tokio::time::sleep(Duration::from_millis(25)).await;
	}
	429
}

#[tokio::test]
async fn in_flight_requests_are_limited_per_key() {
	let upstream = slow_upstream().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*upstream.address()))
		.attach_route_policy_builder(json!({
			"concurrencyLimit": [{"maxConcurrent": 1, "key": "request.headers[\"x-user\"]"}]
		}))
		.await;

	let alice = t.serve_http(BIND_KEY);
	let first = tokio::spawn(async move {
		send_request_headers(
			alice,
			Method::GET,
			"http://localhost/",
			&[("x-user", "alice")],
		)
		.await
	});
	tokio::time::sleep(Duration::from_millis(100)).await;

	// The same key is full while the first request is in flight.
	let second = send_request_headers(
		t.serve_http(BIND_KEY),
		Method::GET,
		"http://localhost/",
		&[("x-user", "alice")],
	)
	.await;
	assert_eq!(second.status(), 429);
	// Another key has its own counter.
	let bob = send_request_headers(
		t.serve_http(BIND_KEY),
		Method::GET,
		"http://localhost/",
		&[("x-user", "bob")],
	)
	.await;
	assert_eq!(bob.status(), 200);
	drain(bob).await;

	// Once the first response is complete, the slot is free again.
	assert_eq!(drain(first.await.unwrap()).await, 200);
	assert_eq!(eventually_admitted(&t, &[("x-user", "alice")]).await, 200);
}

#[tokio::test]
async fn every_rule_must_admit_the_request() {
	let upstream = slow_upstream().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*upstream.address()))
		.attach_route_policy_builder(json!({
			"concurrencyLimit": [
				{"maxConcurrent": 2},
				{"maxConcurrent": 1, "key": "request.headers[\"x-user\"]"},
			]
		}))
		.await;

	let (alice, bob) = (t.serve_http(BIND_KEY), t.serve_http(BIND_KEY));
	let first = tokio::spawn(async move {
		send_request_headers(
			alice,
			Method::GET,
			"http://localhost/",
			&[("x-user", "alice")],
		)
		.await
	});
	let second = tokio::spawn(async move {
		send_request_headers(bob, Method::GET, "http://localhost/", &[("x-user", "bob")]).await
	});
	tokio::time::sleep(Duration::from_millis(100)).await;

	// The shared rule is full even though carol has her own per-user counter.
	let carol = send_request_headers(
		t.serve_http(BIND_KEY),
		Method::GET,
		"http://localhost/",
		&[("x-user", "carol")],
	)
	.await;
	assert_eq!(carol.status(), 429);

	assert_eq!(drain(first.await.unwrap()).await, 200);
	assert_eq!(drain(second.await.unwrap()).await, 200);
	assert_eq!(eventually_admitted(&t, &[("x-user", "carol")]).await, 200);
}

#[tokio::test]
async fn llm_model_keys_are_evaluated_after_parsing() {
	let upstream = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_json(json!({
					"id": "chatcmpl-1",
					"object": "chat.completion",
					"created": 0,
					"model": "mock-model",
					"choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
					"usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
				}))
				.set_delay(DELAY),
		)
		.mount(&upstream)
		.await;
	let backend = custom_llm_backend(
		"llm",
		SimpleBackendReference::Backend(strng::format!("/{}", upstream.address())),
		vec![ProviderFormat::Completions],
	);
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_raw_backend(backend)
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/llm")))
		.attach_route_policy_builder(json!({
			"concurrencyLimit": [{"maxConcurrent": 1, "key": "llm.requestModel"}]
		}))
		.await;

	let body = |model: &str| {
		json!({"model": model, "messages": [{"role": "user", "content": "hi"}]})
			.to_string()
			.into_bytes()
	};
	let send = |t: &TestBind, model: &str| {
		let io = t.serve_http(BIND_KEY);
		let body = body(model);
		async move {
			send_request_body(
				io,
				Method::POST,
				"http://localhost/v1/chat/completions",
				&body,
			)
			.await
		}
	};

	let first = tokio::spawn(send(&t, "qwen3"));
	tokio::time::sleep(Duration::from_millis(100)).await;
	assert_eq!(send(&t, "qwen3").await.status(), 429);
	// A different model has its own counter.
	let other = send(&t, "llama").await;
	assert_eq!(other.status(), 200);
	drain(other).await;
	assert_eq!(drain(first.await.unwrap()).await, 200);

	// The slot is released after the response completes.
	let mut admitted = false;
	for _ in 0..20 {
		let resp = send(&t, "qwen3").await;
		if resp.status() == 200 {
			admitted = true;
			drain(resp).await;
			break;
		}
		tokio::time::sleep(Duration::from_millis(25)).await;
	}
	assert!(admitted);
}

#[tokio::test]
async fn rejection_is_a_429_with_a_body() {
	let upstream = slow_upstream().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*upstream.address()))
		.attach_route_policy_builder(json!({"concurrencyLimit": [{"maxConcurrent": 0}]}))
		.await;
	let resp = send_request_headers(
		t.serve_http(BIND_KEY),
		Method::GET,
		"http://localhost/",
		&[],
	)
	.await;
	assert_eq!(resp.status(), 429);
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	assert!(
		String::from_utf8_lossy(&body).contains("concurrency limit exceeded"),
		"{}",
		String::from_utf8_lossy(&body)
	);
}

/// Set `AGENTGATEWAY_TEST_REDIS_URL` to run this against a Redis.
#[tokio::test]
async fn shared_slots_are_counted_through_the_store() {
	let Some(url) = std::env::var("AGENTGATEWAY_TEST_REDIS_URL").ok() else {
		return;
	};
	let upstream = slow_upstream().await;
	let unique = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos();
	let policy = json!({
		"concurrencyLimit": [{
			"maxConcurrent": 1,
			"key": "request.headers[\"x-user\"]",
			"shared": {"redis": {"url": url}, "keyPrefix": format!("test:{unique}")},
		}]
	});
	// Two proxies with the same rule stand in for two instances behind one store.
	let mut proxies = Vec::new();
	for _ in 0..2 {
		proxies.push(
			setup_proxy_test("{}")
				.unwrap()
				.with_backend(*upstream.address())
				.with_bind(simple_bind())
				.with_route(basic_route(*upstream.address()))
				.attach_route_policy_builder(policy.clone())
				.await,
		);
	}
	let (a, b) = (&proxies[0], &proxies[1]);

	let io = a.serve_http(BIND_KEY);
	let first = tokio::spawn(async move {
		drain(send_request_headers(io, Method::GET, "http://localhost/", &[("x-user", "alice")]).await)
			.await
	});
	tokio::time::sleep(Duration::from_millis(100)).await;
	// The other instance sees alice's slot, and only hers.
	assert_eq!(
		drain(
			send_request_headers(
				b.serve_http(BIND_KEY),
				Method::GET,
				"http://localhost/",
				&[("x-user", "alice")],
			)
			.await
		)
		.await,
		429
	);
	assert_eq!(
		drain(
			send_request_headers(
				b.serve_http(BIND_KEY),
				Method::GET,
				"http://localhost/",
				&[("x-user", "bob")],
			)
			.await
		)
		.await,
		200
	);
	assert_eq!(first.await.unwrap(), 200);
	assert_eq!(eventually_admitted(b, &[("x-user", "alice")]).await, 200);
}
