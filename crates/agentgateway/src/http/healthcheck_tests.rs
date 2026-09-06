use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use agent_core::strng;
use http::Method;
use http_body_util::BodyExt;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::HealthChecker;
use crate::http::health::{self, ActiveHealthCheck, Eviction};
use crate::llm::custom::{Provider as CustomProvider, ProviderFormat, ProviderFormatConfig};
use crate::llm::{AIBackend, AIProvider, NamedAIProvider};
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, send_request_body, setup_proxy_test, simple_bind,
};
use crate::types::agent::{Backend, BackendTrafficPolicy, ResourceName, SimpleBackendReference};
use crate::types::loadbalancer::EndpointSet;

const INTERVAL: Duration = Duration::from_millis(20);

/// Answers `/health` with 200 while the flag is set and 503 otherwise.
struct Toggle(Arc<AtomicBool>);

impl Respond for Toggle {
	fn respond(&self, _: &Request) -> ResponseTemplate {
		if self.0.load(Ordering::SeqCst) {
			ResponseTemplate::new(200)
		} else {
			ResponseTemplate::new(503)
		}
	}
}

/// A mock provider whose chat completions answer with its own name.
async fn provider_server(name: &str, healthy: Arc<AtomicBool>) -> MockServer {
	let server = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/health"))
		.respond_with(Toggle(healthy))
		.mount(&server)
		.await;
	Mock::given(method("POST"))
		.respond_with(ResponseTemplate::new(200).set_body_json(json!({
			"id": "chatcmpl-1",
			"object": "chat.completion",
			"created": 0,
			"model": "mock-model",
			"choices": [{"index": 0, "message": {"role": "assistant", "content": name}, "finish_reason": "stop"}],
			"usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
		})))
		.mount(&server)
		.await;
	server
}

fn check() -> ActiveHealthCheck {
	ActiveHealthCheck {
		interval: INTERVAL,
		timeout: Duration::from_millis(200),
		healthy_threshold: 1,
		unhealthy_threshold: 2,
		..Default::default()
	}
}

fn provider(name: &str, server: &MockServer, check: ActiveHealthCheck) -> NamedAIProvider {
	NamedAIProvider {
		name: name.into(),
		provider: AIProvider::Custom(CustomProvider {
			model: None,
			provider_override: None,
			formats: vec![ProviderFormatConfig {
				format: ProviderFormat::Completions,
				path: None,
			}],
		}),
		provider_backend: Some(SimpleBackendReference::Backend(strng::format!(
			"/{}",
			server.address()
		))),
		host_override: None,
		path_override: None,
		path_prefix: None,
		tokenize: false,
		inline_policies: vec![BackendTrafficPolicy::Health(health::Policy {
			unhealthy_expression: None,
			eviction: Some(Eviction {
				restore_health: Some(1.0),
				..Default::default()
			}),
			active: Some(check),
		})],
	}
}

fn setup(providers: Vec<(NamedAIProvider, &MockServer)>) -> TestBind {
	let mut t = setup_proxy_test("{}").unwrap();
	let mut endpoints = Vec::new();
	for (provider, server) in providers {
		t = t.with_backend(*server.address());
		endpoints.push((provider.name.clone(), provider));
	}
	let backend = Backend::AI(
		ResourceName::new("llm".into(), "".into()),
		AIBackend {
			providers: EndpointSet::new(vec![endpoints]),
		},
	);
	t.with_raw_backend(backend.into())
		.with_bind(simple_bind())
		.with_route(basic_named_route(strng::literal!("/llm")))
}

fn is_evicted(t: &TestBind, provider: &str) -> bool {
	let backend = t
		.inputs()
		.stores
		.read_binds()
		.backend(&strng::literal!("/llm"))
		.expect("backend exists");
	let Backend::AI(_, ai) = &backend.backend else {
		panic!("expected an AI backend");
	};
	ai.providers
		.find_endpoint(|ep, info| (ep.name.as_str() == provider).then(|| info.is_evicted()))
		.expect("provider exists")
}

/// Whether the provider is in the set the load balancer currently picks from.
fn is_selectable(t: &TestBind, provider: &str) -> bool {
	let backend = t
		.inputs()
		.stores
		.read_binds()
		.backend(&strng::literal!("/llm"))
		.expect("backend exists");
	let Backend::AI(_, ai) = &backend.backend else {
		panic!("expected an AI backend");
	};
	ai.providers
		.iter()
		.iter()
		.any(|(ep, _)| ep.name.as_str() == provider)
}

/// Wait for the eviction worker to apply a decision.
async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
	for _ in 0..200 {
		if cond() {
			return;
		}
		tokio::time::sleep(Duration::from_millis(10)).await;
	}
	panic!("timed out waiting for {what}");
}

/// Which provider answered a chat completion.
async fn served_by(t: &TestBind) -> String {
	let body = json!({"model": "any", "messages": [{"role": "user", "content": "hi"}]})
		.to_string()
		.into_bytes();
	let resp = send_request_body(
		t.serve_http(BIND_KEY),
		Method::POST,
		"http://localhost/v1/chat/completions",
		&body,
	)
	.await;
	assert_eq!(resp.status(), 200);
	let body = resp.into_body().collect().await.unwrap().to_bytes();
	let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
	json["choices"][0]["message"]["content"]
		.as_str()
		.unwrap()
		.to_string()
}

/// Run the checker until every provider has been probed `times` more times.
async fn probe_rounds(checker: &mut HealthChecker, times: usize) {
	for _ in 0..times {
		tokio::time::sleep(INTERVAL + Duration::from_millis(5)).await;
		checker.run_once().await;
	}
}

#[tokio::test]
async fn failing_probes_evict_and_recovery_restores() {
	let healthy_a = Arc::new(AtomicBool::new(true));
	let healthy_b = Arc::new(AtomicBool::new(false));
	let a = provider_server("a", healthy_a.clone()).await;
	let b = provider_server("b", healthy_b.clone()).await;
	let t = setup(vec![
		(provider("a", &a, check()), &a),
		(provider("b", &b, check()), &b),
	]);
	let mut checker = HealthChecker::new(t.inputs());

	// Both providers are due on the first pass, and nothing is due again until the interval passes.
	// A loaded machine can stretch the first pass past the interval, which makes them due again.
	let first_pass = Instant::now();
	assert_eq!(checker.run_once().await, 2);
	if first_pass.elapsed() < INTERVAL / 2 {
		assert_eq!(checker.run_once().await, 0);
		// One failure is below the threshold of two.
		assert!(!is_evicted(&t, "b"));
	}

	probe_rounds(&mut checker, 1).await;
	assert!(is_evicted(&t, "b"), "b failed twice and should be evicted");
	assert!(!is_evicted(&t, "a"));
	wait_until("b to leave the active set", || !is_selectable(&t, "b")).await;

	// Every request goes to the healthy provider while b is evicted.
	for _ in 0..10 {
		assert_eq!(served_by(&t).await, "a");
	}

	// b keeps failing: it stays evicted across further rounds.
	probe_rounds(&mut checker, 3).await;
	assert!(is_evicted(&t, "b"));
	assert!(!is_selectable(&t, "b"));

	// Once b answers again it is restored after one healthy probe.
	healthy_b.store(true, Ordering::SeqCst);
	probe_rounds(&mut checker, 1).await;
	wait_until("b to rejoin the active set", || {
		!is_evicted(&t, "b") && is_selectable(&t, "b")
	})
	.await;

	let mut seen = std::collections::HashSet::new();
	for _ in 0..60 {
		seen.insert(served_by(&t).await);
	}
	assert!(seen.contains("a") && seen.contains("b"), "{seen:?}");
}

#[tokio::test]
async fn healthy_probes_do_not_lift_a_passive_eviction() {
	let a = provider_server("a", Arc::new(AtomicBool::new(true))).await;
	let t = setup(vec![(provider("a", &a, check()), &a)]);
	let mut checker = HealthChecker::new(t.inputs());
	checker.run_once().await;
	assert!(!is_evicted(&t, "a"));

	// Evict the provider the way a failed request would.
	{
		let backend = t
			.inputs()
			.stores
			.read_binds()
			.backend(&strng::literal!("/llm"))
			.unwrap();
		let Backend::AI(_, ai) = &backend.backend else {
			unreachable!()
		};
		ai.providers.evict(
			strng::literal!("a"),
			std::time::Instant::now() + Duration::from_secs(30),
		);
	}
	assert!(is_evicted(&t, "a"));

	// Passing probes leave the passive eviction in place.
	probe_rounds(&mut checker, 3).await;
	assert!(is_evicted(&t, "a"));
}

#[tokio::test]
async fn probe_timeout_counts_as_failure() {
	let a = provider_server("a", Arc::new(AtomicBool::new(true))).await;
	let slow = MockServer::start().await;
	Mock::given(method("GET"))
		.and(path("/health"))
		.respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
		.mount(&slow)
		.await;
	let t = setup(vec![
		(provider("a", &a, check()), &a),
		(provider("slow", &slow, check()), &slow),
	]);
	let mut checker = HealthChecker::new(t.inputs());
	checker.run_once().await;
	probe_rounds(&mut checker, 1).await;
	assert!(is_evicted(&t, "slow"));
	assert!(!is_evicted(&t, "a"));
}

#[tokio::test]
async fn expected_statuses_replace_the_2xx_default() {
	let healthy = Arc::new(AtomicBool::new(false));
	let a = provider_server("a", healthy.clone()).await;
	let accepts_503 = ActiveHealthCheck {
		expected_statuses: vec![503],
		..check()
	};
	let t = setup(vec![(provider("a", &a, accepts_503), &a)]);
	let mut checker = HealthChecker::new(t.inputs());
	checker.run_once().await;
	probe_rounds(&mut checker, 2).await;
	// 503 is the only accepted status, so the provider stays in.
	assert!(!is_evicted(&t, "a"));

	// ... and 200 is now a failure.
	healthy.store(true, Ordering::SeqCst);
	probe_rounds(&mut checker, 2).await;
	assert!(is_evicted(&t, "a"));
}

#[tokio::test]
async fn providers_without_an_active_check_are_not_probed() {
	let a = provider_server("a", Arc::new(AtomicBool::new(false))).await;
	let mut plain = provider("a", &a, check());
	plain.inline_policies.clear();
	let t = setup(vec![(plain, &a)]);
	let mut checker = HealthChecker::new(t.inputs());
	assert_eq!(checker.run_once().await, 0);
	probe_rounds(&mut checker, 2).await;
	assert!(!is_evicted(&t, "a"));
	assert!(a.received_requests().await.unwrap().is_empty());
}
