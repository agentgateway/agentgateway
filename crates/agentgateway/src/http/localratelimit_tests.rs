use agent_core::strng;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use http_body_util::BodyExt;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::http::{Body, Response};
use crate::llm::custom::ProviderFormat;
use crate::proxy::request_builder::RequestBuilder;
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, basic_route, custom_llm_backend, send_request_headers,
	setup_proxy_test, simple_bind,
};
use crate::types::agent::SimpleBackendReference;

async fn upstream() -> MockServer {
	let server = MockServer::start().await;
	Mock::given(method("GET"))
		.respond_with(ResponseTemplate::new(200).set_body_string("ok"))
		.mount(&server)
		.await;
	server
}

async fn status(resp: Response) -> u16 {
	let status = resp.status().as_u16();
	let _ = resp.into_body().collect().await;
	status
}

async fn get(t: &TestBind, headers: &[(&str, &str)]) -> u16 {
	status(
		send_request_headers(
			t.serve_http(BIND_KEY),
			Method::GET,
			"http://localhost/",
			headers,
		)
		.await,
	)
	.await
}

async fn plain_route(policy: serde_json::Value) -> (MockServer, TestBind) {
	let upstream = upstream().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_backend(*upstream.address())
		.with_bind(simple_bind())
		.with_route(basic_route(*upstream.address()))
		.attach_route_policy_builder(policy)
		.await;
	(upstream, t)
}

#[tokio::test]
async fn requests_are_limited_per_key() {
	let (_upstream, t) = plain_route(json!({
		"localRateLimit": [{
			"maxTokens": 1, "tokensPerFill": 1, "fillInterval": "60s",
			"key": "request.headers[\"x-user\"]",
		}]
	}))
	.await;
	assert_eq!(get(&t, &[("x-user", "alice")]).await, 200);
	assert_eq!(get(&t, &[("x-user", "alice")]).await, 429);
	// Another key has its own bucket.
	assert_eq!(get(&t, &[("x-user", "bob")]).await, 200);
	// Requests without a key share one bucket.
	assert_eq!(get(&t, &[]).await, 200);
	assert_eq!(get(&t, &[]).await, 429);
}

#[tokio::test]
async fn every_rule_must_admit_the_request() {
	let (_upstream, t) = plain_route(json!({
		"localRateLimit": [
			{"maxTokens": 2, "tokensPerFill": 2, "fillInterval": "60s", "key": "request.headers[\"x-team\"]"},
			{"maxTokens": 1, "tokensPerFill": 1, "fillInterval": "60s", "key": "request.headers[\"x-user\"]"},
		]
	}))
	.await;
	let alice = [("x-team", "t1"), ("x-user", "alice")];
	let bob = [("x-team", "t1"), ("x-user", "bob")];
	let carol = [("x-team", "t1"), ("x-user", "carol")];
	let dave = [("x-team", "t2"), ("x-user", "dave")];
	assert_eq!(get(&t, &alice).await, 200);
	// Alice's own bucket is empty.
	assert_eq!(get(&t, &alice).await, 429);
	assert_eq!(get(&t, &bob).await, 200);
	// The team bucket is empty even though carol has not sent anything yet.
	assert_eq!(get(&t, &carol).await, 429);
	assert_eq!(get(&t, &dave).await, 200);
}

/// A chat-completions route with a mock provider that reports the given usage.
async fn llm_route(
	policy: serde_json::Value,
	prompt_tokens: u64,
	completion_tokens: u64,
) -> (MockServer, TestBind) {
	let upstream = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id": "chatcmpl-1",
			"object": "chat.completion",
			"created": 0,
			"model": "mock-model",
			"choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
			"usage": {
				"prompt_tokens": prompt_tokens,
				"completion_tokens": completion_tokens,
				"total_tokens": prompt_tokens + completion_tokens,
			},
		})))
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
		.with_bind(simple_bind())
		.with_raw_backend(backend)
		.with_route(basic_named_route(strng::literal!("/llm")))
		.attach_route_policy_builder(policy)
		.await;
	(upstream, t)
}

async fn chat(t: &TestBind, model: &str, user: &str) -> Response {
	let body = json!({"model": model, "messages": [{"role": "user", "content": "hi"}]}).to_string();
	let headers = HeaderMap::from_iter([(
		HeaderName::from_static("x-user"),
		HeaderValue::from_str(user).unwrap(),
	)]);
	RequestBuilder::new(Method::POST, "http://localhost/v1/chat/completions")
		.headers(headers)
		.body(Body::from(body))
		.send(t.serve_http(BIND_KEY))
		.await
		.unwrap()
}

fn remaining(resp: &Response) -> u64 {
	resp
		.headers()
		.get("x-ratelimit-remaining")
		.expect("rate limit headers are set")
		.to_str()
		.unwrap()
		.parse()
		.unwrap()
}

#[tokio::test]
async fn token_limits_can_key_on_the_requested_model() {
	// A token limit is charged once the request has been parsed, so its key can read the model.
	// Every response costs 25 tokens; each model has 30.
	let (_upstream, t) = llm_route(
		json!({
			"localRateLimit": [{
				"type": "tokens", "maxTokens": 30, "tokensPerFill": 30, "fillInterval": "60s",
				"key": "llm.requestModel",
			}]
		}),
		10,
		15,
	)
	.await;
	status(chat(&t, "qwen3", "alice").await).await;
	let second = chat(&t, "qwen3", "bob").await;
	assert_eq!(second.status(), 200);
	assert_eq!(remaining(&second), 5, "the first response was settled");
	status(second).await;
	assert_eq!(status(chat(&t, "qwen3", "alice").await).await, 429);
	// A different model has its own bucket.
	let other = chat(&t, "llama", "alice").await;
	assert_eq!(other.status(), 200);
	assert_eq!(remaining(&other), 30);
}

#[tokio::test]
async fn token_usage_is_settled_per_key() {
	// Every response costs 25 tokens; each user has 30.
	let (_upstream, t) = llm_route(
		json!({
			"localRateLimit": [{
				"type": "tokens", "maxTokens": 30, "tokensPerFill": 30, "fillInterval": "60s",
				"key": "request.headers[\"x-user\"]",
			}]
		}),
		10,
		15,
	)
	.await;
	let first = chat(&t, "qwen3", "alice").await;
	assert_eq!(first.status(), 200);
	// Input tokens are not counted before the call, so the whole bucket is still available.
	assert_eq!(remaining(&first), 30);
	status(first).await;

	// The first response was settled against alice's bucket once it completed.
	let second = chat(&t, "qwen3", "alice").await;
	assert_eq!(second.status(), 200);
	assert_eq!(remaining(&second), 5);
	status(second).await;

	assert_eq!(status(chat(&t, "qwen3", "alice").await).await, 429);

	// Bob's bucket is untouched.
	let bob = chat(&t, "qwen3", "bob").await;
	assert_eq!(bob.status(), 200);
	assert_eq!(remaining(&bob), 30);
	status(bob).await;
}
