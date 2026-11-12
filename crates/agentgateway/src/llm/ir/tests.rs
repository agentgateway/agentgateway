//! End-to-end gateway IR requests, replies, fallback, and retry behavior.

use std::collections::HashMap;
use std::fmt::Write;

use ::http::StatusCode;
use agent_llm_ir as llm_ir;
use agent_llm_ir::{ClientProtocol, ConvertedRequest, StateScope, Upstream, UpstreamProtocol};
use http_body_util::BodyExt;
use rstest::rstest;
use serde_json::{Value, json};

use super::identifier::IdentifierHasher;
use super::request::route_type;
use super::test_support::*;
use super::*;
use crate::cel::LLMContext;
use crate::http::{self, Body, Request};
use crate::json::ParsedJson;
use crate::llm::policy::Policy;
use crate::llm::{AIError, AIProvider, InputFormat, LLMLogging, RequestResult, RouteType};
use crate::store::{LLMRequestPolicies, LLMResponsePolicies};

fn messages_request(uri: &str, body: &Value) -> Request {
	::http::Request::builder()
		.method("POST")
		.uri(uri)
		.body(Body::from(body.to_string()))
		.unwrap()
}

fn convert(body: &Value, model: &str) -> ConvertedRequest {
	let identifier_pseudonymizer = IdentifierHasher;
	llm_ir::prepare(
		body.to_string().as_bytes(),
		ClientProtocol::Messages,
		Upstream {
			identifier_pseudonymizer: &identifier_pseudonymizer,
			scope: StateScope {
				backend: "ns/backend",
				provider: "openai",
				model,
			},
			protocol: UpstreamProtocol::Responses {
				safety_identifier: true,
				reasoning_context: false,
			},
			default_max_tokens: None,
		},
	)
	.and_then(|prepared| prepared.render(llm_ir::RenderSettings::ModelDefaults))
	.unwrap()
}

async fn process(
	provider: &AIProvider,
	policies: Option<&Policy>,
	req: Request,
) -> Result<RequestResult, AIError> {
	process_route(provider, RouteType::Messages, policies, req).await
}

async fn process_route(
	provider: &AIProvider,
	route_type: RouteType,
	policies: Option<&Policy>,
	req: Request,
) -> Result<RequestResult, AIError> {
	process_with_catalog(provider, route_type, policies, req, None).await
}

async fn process_with_catalog(
	provider: &AIProvider,
	route_type: RouteType,
	policies: Option<&Policy>,
	req: Request,
	catalog: agent_llm::model_catalog::Catalog<'_>,
) -> Result<RequestResult, AIError> {
	let Selection::Ir(selected) = select(
		route_type,
		&named(provider.clone()),
		&LLMRequestPolicies::default(),
		"ns/backend".into(),
		None,
	)
	.unwrap() else {
		panic!("expected chat IR selection");
	};
	let backend_info = crate::http::auth::BackendInfo {
		target: crate::types::agent::BackendTarget::Invalid,
		call_target: crate::types::agent::Target::from(("api.openai.com", 443)),
		inputs: crate::test_helpers::proxymock::setup_proxy_test("{}")
			.unwrap()
			.pi,
	};
	provider
		.process_request_ir(selected, &backend_info, policies, req, &mut None, catalog)
		.await
}

/// A client's cache mark stays on the system text it marked when a policy adds text before.
#[test]
fn a_prepended_system_prompt_leaves_the_clients_cache_mark_in_place() {
	let identifier_pseudonymizer = IdentifierHasher;
	let body = json!({"model": "gpt-5.6", "max_tokens": 9,
		"system": [{"type": "text", "text": "Be brief", "cache_control": {"type": "ephemeral"}}],
		"messages": [{"role": "user", "content": "hi"}]})
	.to_string();
	let protocol = UpstreamProtocol::Responses {
		safety_identifier: false,
		reasoning_context: false,
	};
	let scope = StateScope {
		backend: "ns/backend",
		provider: "openai",
		model: "gpt-5.6",
	};
	let mut prepared = llm_ir::prepare(
		body.as_bytes(),
		ClientProtocol::Messages,
		Upstream {
			identifier_pseudonymizer: &identifier_pseudonymizer,
			scope,
			protocol,
			default_max_tokens: None,
		},
	)
	.unwrap();
	let prompts: crate::llm::policy::PromptEnrichment =
		from_json(json!({"prepend": [{"role": "system", "content": "Rule one"}]}));
	enrichment::apply(&prompts, prepared.conversation_mut()).unwrap();
	let conversation = prepared.conversation();
	let system: Vec<_> = conversation
		.system
		.iter()
		.flatten()
		.map(|text| text.read().into_owned())
		.collect();
	assert_eq!(system, ["Rule one", "Be brief"]);
	assert_eq!(
		conversation.cache_breakpoints,
		[llm_ir::CacheBreakpoint::System(1)]
	);
}

#[rstest]
#[case::chat(Client::Chat, false)]
#[case::messages_blocks(Client::Messages, false)]
#[case::messages_text(Client::Messages, true)]
#[case::responses_items(Client::Responses, false)]
#[case::responses_text(Client::Responses, true)]
#[case::gemini(Client::Gemini, false)]
#[case::gemini_snake_case(Client::Gemini, true)]
#[tokio::test]
async fn enrichment_preserves_native_fields_on_a_passthrough(
	#[case] client: Client,
	#[case] alternate: bool,
) {
	let policy: Policy = from_json(json!({"prompts": {
		"prepend": [
			{"role": "system", "content": "Before rule"},
			{"role": "user", "content": "Before turn"},
		],
		"append": [
			{"role": "system", "content": "After rule"},
			{"role": "assistant", "content": "After turn"},
		],
	}}));
	let req = client.request("m");
	let uri = req.uri().to_string();
	let mut body: Value =
		serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
	body["provider_extension"] = json!({"nested": [null, "opaque state", 7]});
	let provider = match client {
		Client::Messages => anthropic(),
		Client::Gemini => gemini(),
		Client::Responses | Client::Chat => openai(None),
	};
	let field = match client {
		Client::Responses => "input",
		Client::Gemini => "contents",
		Client::Messages | Client::Chat => "messages",
	};
	body[field][0]["native_state"] = json!({"signature": "provider-only"});
	let system_key = if alternate {
		"system_instruction"
	} else {
		"systemInstruction"
	};
	match client {
		Client::Messages => {
			body["system"] = if alternate {
				json!("Existing rule")
			} else {
				json!([{"type": "text", "text": "Existing rule",
					"cache_control": {"type": "ephemeral", "ttl": "1h"}, "native": true}])
			};
		},
		Client::Gemini => {
			body[system_key] = json!({"role": "system", "native": true,
				"parts": [{"text": "Existing rule", "native": true}]});
		},
		Client::Responses if alternate => body["input"] = json!("hello"),
		_ => {},
	}
	let mut expected = body.clone();
	let existing = if matches!(client, Client::Responses) && alternate {
		json!({"type": "message", "role": "user",
			"content": [{"type": "input_text", "text": "hello"}]})
	} else {
		body[field][0].clone()
	};
	match client {
		Client::Chat => {
			expected[field] = json!([
				{"role": "system", "content": "Before rule"},
				{"role": "user", "content": "Before turn"}, existing,
				{"role": "system", "content": "After rule"},
				{"role": "assistant", "content": "After turn"},
			]);
		},
		Client::Responses => {
			expected[field] = json!([
				{"type": "message", "role": "system", "content": [{"type": "input_text", "text": "Before rule"}]},
				{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Before turn"}]},
				existing,
				{"type": "message", "role": "system", "content": [{"type": "input_text", "text": "After rule"}]},
				{"type": "message", "role": "assistant", "content": "After turn"},
			]);
		},
		Client::Messages => {
			expected[field] = json!([
				{"role": "user", "content": "Before turn"}, existing,
				{"role": "assistant", "content": "After turn"},
			]);
			let existing = if alternate {
				json!({"type": "text", "text": "Existing rule"})
			} else {
				body["system"][0].clone()
			};
			expected["system"] = json!([
				{"type": "text", "text": "Before rule"}, existing,
				{"type": "text", "text": "After rule"},
			]);
		},
		Client::Gemini => {
			expected[field] = json!([
				{"role": "user", "parts": [{"text": "Before turn"}]}, existing,
				{"role": "model", "parts": [{"text": "After turn"}]},
			]);
			expected[system_key]["parts"] = json!([
				{"text": "Before rule"}, body[system_key]["parts"][0],
				{"text": "After rule"},
			]);
		},
	}
	let RequestResult::Success {
		request, ir_reply, ..
	} = process_route(
		&provider,
		client.route_type(),
		Some(&policy),
		messages_request(&uri, &body),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(serde_json::from_slice::<Value>(&sent).unwrap(), expected);
	if !(matches!(client, Client::Responses) && alternate) {
		let native = serde_json::to_string(&body[field][0]).unwrap();
		assert!(std::str::from_utf8(&sent).unwrap().contains(&native));
	}
}

#[tokio::test]
async fn native_enrichment_runs_once_between_body_and_final_mutations() {
	let mut policy: Policy = from_json(json!({"prompts": {
		"prepend": [{"role": "user", "content": "Before"}],
		"append": [{"role": "assistant", "content": "After"}],
	}}));
	policy.overrides = Some(HashMap::from([(
		"messages".into(),
		json!([{"role": "user", "content": "Overridden"}]),
	)]));
	policy.final_transformations = Some(HashMap::from([(
		"seen_by_final".into(),
		expr("llmRequest.messages"),
	)]));
	let RequestResult::Success {
		request, ir_reply, ..
	} = process_route(
		&openai(None),
		RouteType::Completions,
		Some(&policy),
		Client::Chat.request("m"),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	let expected = json!([
		{"role": "user", "content": "Before"},
		{"role": "user", "content": "Overridden"},
		{"role": "assistant", "content": "After"},
	]);
	assert_eq!(sent["messages"], expected);
	assert_eq!(sent["seen_by_final"], expected);
}

#[tokio::test]
async fn empty_native_enrichment_preserves_the_clients_bytes() {
	let policy: Policy = from_json(json!({"prompts": {}}));
	let body = r#"{"model":"m", "max_tokens":9,"system":"unchanged", "messages":[{"role":"user","content":"hi"}],"opaque":{"n":1e+09,"s":"\u0061"}}"#;
	let req = ::http::Request::builder()
		.method("POST")
		.uri("/v1/messages")
		.body(Body::from(body))
		.unwrap();
	let RequestResult::Success {
		request, ir_reply, ..
	} = process_route(&anthropic(), RouteType::Messages, Some(&policy), req)
		.await
		.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(&sent[..], body.as_bytes());
}

/// A passthrough sends a prompt in another role as written, as legacy does. A converted
/// request is refused, since no other role means the same in every protocol.
#[tokio::test]
async fn an_enrichment_role_without_an_equivalent_is_refused_when_converting() {
	let policy: Policy =
		from_json(json!({"prompts": {"append": [{"role": "tool", "content": "x"}]}}));
	let RequestResult::Success {
		request, ir_reply, ..
	} = process_route(
		&openai(None),
		RouteType::Completions,
		Some(&policy),
		Client::Chat.request("gpt-5"),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(sent["messages"][1], json!({"role": "tool", "content": "x"}));

	let result = process(
		&openai(None),
		Some(&policy),
		Client::Messages.request("gpt-5"),
	)
	.await;
	assert!(
		matches!(result, Err(AIError::RequestRefused { .. })),
		"{result:?}"
	);
}

/// Prompt guards on any passthrough and caching on a Messages passthrough still use legacy.
#[rstest::rstest]
#[case::caching_a_messages_passthrough(
	anthropic(),
	"/v1/messages",
	RouteType::Messages,
	json!({"promptCaching": {}})
)]
#[case::guarding_a_passthrough(
	openai(None),
	"/v1/chat/completions",
	RouteType::Completions,
	json!({"promptGuard": {"request": [{"regex": {"rules": [{"pattern": "hi"}]}}]}})
)]
#[tokio::test]
async fn a_passthrough_these_policies_change_stays_on_legacy(
	#[case] provider: AIProvider,
	#[case] uri: &str,
	#[case] route_type: RouteType,
	#[case] policy: Value,
) {
	let policy: Policy = from_json(policy);
	let client = json!({"model": "m", "max_tokens": 9,
		"messages": [{"role": "user", "content": "hi"}]});
	let RequestResult::Success { ir_reply, .. } = process_route(
		&provider,
		route_type,
		Some(&policy),
		messages_request(uri, &client),
	)
	.await
	.unwrap() else {
		panic!("expected a forwarded request");
	};
	assert!(ir_reply.is_none(), "the IR served it");
}

#[rstest]
#[case::openai_alias(openai(None), Some("gpt-5.5"), true)]
#[case::openai_override(openai(Some("gpt-5.5")), None, true)]
#[case::openai_snapshot(openai(Some("gpt-5.5-2026-04-23")), None, true)]
#[case::older_model(openai(Some("gpt-5.4")), None, false)]
#[case::unknown_model(openai(Some("gpt-5.6")), None, false)]
#[case::unverified_variant(openai(Some("gpt-5.5-pro")), None, false)]
#[case::client_name_only(openai(None), None, false)]
#[case::custom_responses(AIProvider::Custom(agent_llm::custom::Provider {
	model_override: Some("gpt-5.5".into()),
	provider_override: Some("openai".into()),
	formats: vec![agent_llm::custom::ProviderFormatConfig {
		format: agent_llm::custom::ProviderFormat::Responses,
		path: None,
	}],
}), None, false)]
#[tokio::test]
async fn claude_thinking_context_uses_the_upstream_provider_and_model(
	#[case] provider: AIProvider,
	#[case] alias: Option<&str>,
	#[case] supported: bool,
) {
	let policy = Policy {
		model_aliases: alias
			.map(|model| HashMap::from([("claude-sonnet-4-6".into(), model.into())]))
			.unwrap_or_default(),
		..Default::default()
	};
	let client = json!({
		"model": "claude-sonnet-4-6", "max_tokens": 16,
		"thinking": {"type": "adaptive", "display": "updates"},
		"output_config": {"effort": "high"},
		"context_management": {"edits": [{"type": "clear_thinking_20251015", "keep": "all"}]},
		"messages": [{"role": "user", "content": "Reply OK."}]
	});
	let result = process(
		&provider,
		Some(&policy),
		messages_request("/v1/messages", &client),
	)
	.await;
	if !supported {
		assert!(
			matches!(result, Err(AIError::RequestRefused { ref problems })
			if problems.join("; ").contains("`context_management` on a model that doesn't accept it"))
		);
		return;
	}
	let RequestResult::Success {
		request,
		upstream_route_type,
		..
	} = result.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert_eq!(upstream_route_type, RouteType::Responses);
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(
		sent["model"],
		alias
			.map(Into::into)
			.or_else(|| provider.override_model())
			.unwrap()
			.as_str()
	);
	assert_eq!(
		sent["reasoning"],
		json!({"effort": "high", "context": "all_turns"})
	);
	assert_eq!(sent["max_output_tokens"], 16);
	for (field, value, problem) in [
		(
			"context_management",
			json!({"edits": []}),
			"`context_management` shape",
		),
		(
			"safeguards",
			json!([{"type": "dangerous_tool_use"}]),
			"field `safeguards`",
		),
	] {
		let mut invalid = client.clone();
		invalid[field] = value;
		let result = process(
			&provider,
			Some(&policy),
			messages_request("/v1/messages", &invalid),
		)
		.await;
		assert!(
			matches!(result, Err(AIError::RequestRefused { ref problems })
			if problems.join("; ").contains(problem))
		);
	}
}

/// The catalog's levels win over a guess from the name, which would send `low` here.
#[tokio::test]
async fn gemini_thinking_comes_from_the_catalog() {
	let catalog = crate::llm::catalog::CatalogSnapshot::parse(
		r#"{"providers":{"gcp.gemini":{"models":{"gemini-x":{
				"reasoning":{"effort":["minimal","high"]}}}}}}"#,
	)
	.unwrap();
	let client = json!({"model": "gemini-x", "max_tokens": 9,
		"output_config": {"effort": "low"},
		"messages": [{"role": "user", "content": "hi"}]});
	assert_eq!(thinking_level(&client, Some(&catalog)).await, "minimal");
	assert_eq!(thinking_level(&client, None).await, "low");
}

async fn thinking_level(client: &Value, catalog: agent_llm::model_catalog::Catalog<'_>) -> Value {
	let RequestResult::Success { request, .. } = process_with_catalog(
		&gemini(),
		RouteType::Messages,
		None,
		messages_request("/v1/messages", client),
		catalog,
	)
	.await
	.unwrap() else {
		panic!("expected a converted request");
	};
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	sent["generationConfig"]["thinkingConfig"]["thinkingLevel"].clone()
}

#[rstest]
#[case::levels_override_the_name("gpt-4o", json!({"reasoning": {"effort": ["low", "high"]}}), json!("high"), Value::Null)]
#[case::reasoning_is_unsupported("gpt-5.4", json!({"capabilities": {"reasoning": false}}), Value::Null, json!("medium"))]
#[case::missing_controls_use_defaults("gpt-5.4", json!({}), json!("medium"), json!("medium"))]
#[case::unknown_levels_use_defaults("gpt-5.4", json!({"reasoning": {"effort": ["future"]}}), json!("medium"), json!("medium"))]
#[tokio::test]
async fn openai_reasoning_comes_from_the_catalog(
	#[values(false, true)] chat: bool,
	#[case] model: &str,
	#[case] entry: Value,
	#[case] expected: Value,
	#[case] fallback: Value,
) {
	let provider = if chat {
		AIProvider::Custom(agent_llm::custom::Provider {
			model_override: Some(model.into()),
			provider_override: Some("test-chat".into()),
			formats: vec![agent_llm::custom::ProviderFormatConfig {
				format: agent_llm::custom::ProviderFormat::Completions,
				path: None,
			}],
		})
	} else {
		openai(Some(model))
	};
	let other_provider = if chat { "openai" } else { "test-chat" };
	let data = json!({"providers": {
		provider.provider().as_str(): {"models": {model: entry}},
		other_provider: {"models": {model: {"capabilities": {"reasoning": false}}}}
	}});
	let catalog = crate::llm::catalog::CatalogSnapshot::parse(&data.to_string()).unwrap();
	let client = json!({"model": "client-alias", "max_tokens": 9,
		"output_config": {"effort": "medium"},
		"messages": [{"role": "user", "content": "hi"}]});
	for (catalog, expected) in [(Some(&catalog), expected), (None, fallback)] {
		let RequestResult::Success {
			request,
			upstream_route_type,
			..
		} = process_with_catalog(
			&provider,
			RouteType::Messages,
			None,
			messages_request("/v1/messages", &client),
			catalog.map(|catalog| catalog as &dyn agent_llm::model_catalog::ModelCatalogHandle),
		)
		.await
		.unwrap()
		else {
			panic!("expected a converted request");
		};
		let sent: Value =
			serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
		let pointer = if chat {
			"/reasoning_effort"
		} else {
			"/reasoning/effort"
		};
		assert_eq!(
			upstream_route_type,
			if chat {
				RouteType::Completions
			} else {
				RouteType::Responses
			}
		);
		assert_eq!(
			sent.pointer(pointer).unwrap_or(&Value::Null),
			&expected,
			"{sent}"
		);
	}
}

#[derive(Debug, Clone, Copy)]
enum Client {
	Messages,
	Responses,
	Chat,
	Gemini,
}

impl Client {
	fn route_type(self) -> RouteType {
		match self {
			Self::Messages => RouteType::Messages,
			Self::Responses => RouteType::Responses,
			Self::Chat => RouteType::Completions,
			Self::Gemini => RouteType::GenerateContent,
		}
	}

	/// A streamed request, without an output limit where the protocol lets it leave one out.
	fn request(self, model: &str) -> Request {
		let (uri, body) = match self {
			Self::Gemini => (
				format!("/v1beta/models/{model}:streamGenerateContent?alt=sse"),
				json!({"contents": [{"role": "user", "parts": [{"text": "hello"}]}]}),
			),
			Self::Messages => (
				"/v1/messages".to_string(),
				json!({"model": model, "max_tokens": 64, "stream": true,
					"messages": [{"role": "user", "content": "hello"}]}),
			),
			Self::Responses => (
				"/v1/responses".to_string(),
				json!({"model": model, "stream": true, "reasoning": {"effort": "low"},
					"input": [{"type": "message", "role": "user",
						"content": [{"type": "input_text", "text": "hello"}]}]}),
			),
			Self::Chat => (
				"/v1/chat/completions".to_string(),
				json!({"model": model, "stream": true,
					"stream_options": {"include_usage": true},
					"messages": [{"role": "user", "content": "hello"}]}),
			),
		};
		messages_request(&uri, &body)
	}
}

fn anthropic() -> AIProvider {
	AIProvider::Anthropic(from_json(json!({})))
}

fn gemini() -> AIProvider {
	AIProvider::Gemini(from_json(json!({})))
}

fn copilot() -> AIProvider {
	AIProvider::Copilot(from_json(json!({})))
}

fn azure(resource_type: crate::llm::azure::AzureResourceType) -> AIProvider {
	AIProvider::Azure(crate::llm::AzureProvider::new(
		crate::llm::azure::Provider {
			model_override: None,
			resource_name: "r".into(),
			resource_type,
			api_version: None,
			project_name: None,
		},
	))
}

/// Each client protocol reaches the provider's native protocol where legacy either has no
/// route or goes through an OpenAI-compatible shim.
#[rstest]
#[case::responses_to_anthropic(
	Client::Responses,
	anthropic(),
	"claude-sonnet-4-5",
	RouteType::Messages
)]
#[case::chat_to_anthropic(Client::Chat, anthropic(), "claude-sonnet-4-5", RouteType::Messages)]
#[case::messages_to_gemini(
	Client::Messages,
	gemini(),
	"gemini-2.5-pro",
	RouteType::GenerateContent
)]
#[case::responses_to_gemini(
	Client::Responses,
	gemini(),
	"gemini-2.5-pro",
	RouteType::GenerateContent
)]
#[case::chat_to_gemini(Client::Chat, gemini(), "gemini-2.5-pro", RouteType::GenerateContent)]
#[case::messages_to_openai(Client::Messages, openai(None), "gpt-5", RouteType::Responses)]
#[case::gemini_to_anthropic(Client::Gemini, anthropic(), "claude-sonnet-4-5", RouteType::Messages)]
#[case::gemini_to_openai(Client::Gemini, openai(None), "gpt-5", RouteType::Responses)]
#[case::responses_to_copilot_claude(
	Client::Responses,
	copilot(),
	"claude-sonnet-4.5",
	RouteType::Messages
)]
#[case::messages_to_copilot_gpt(Client::Messages, copilot(), "gpt-5", RouteType::Responses)]
#[case::chat_to_copilot_gpt(Client::Chat, copilot(), "gpt-5", RouteType::Responses)]
#[case::chat_to_foundry_claude(
	Client::Chat,
	azure(crate::llm::azure::AzureResourceType::Foundry),
	"claude-sonnet-4-5",
	RouteType::Messages
)]
#[case::messages_to_foundry_gpt(
	Client::Messages,
	azure(crate::llm::azure::AzureResourceType::Foundry),
	"gpt-5",
	RouteType::Responses
)]
#[case::messages_to_azure_openai(
	Client::Messages,
	azure(crate::llm::azure::AzureResourceType::OpenAI),
	"gpt-5",
	RouteType::Responses
)]
#[tokio::test]
async fn a_client_reaches_the_providers_native_protocol(
	#[case] client: Client,
	#[case] provider: AIProvider,
	#[case] model: &str,
	#[case] upstream: RouteType,
	#[values(false, true)] cached: bool,
) {
	let mut req = client.request(model);
	if cached {
		let body = std::mem::replace(req.body_mut(), Body::empty());
		let bytes = http::read_body_with_limit(body, 1024 * 1024).await.unwrap();
		let parsed = serde_json::from_slice(&bytes).unwrap();
		req.body_mut().replace_bytes(bytes);
		req.body_mut().insert_extension(ParsedJson(parsed));
	}
	let result = process_route(&provider, client.route_type(), None, req).await;
	let RequestResult::Success {
		ir_reply,
		request,
		llm_request,
		upstream_route_type,
		..
	} = result.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert_eq!(upstream_route_type, upstream);
	let Some(ReplyState::Convert(attempt)) = ir_reply else {
		panic!("expected IR attempt state");
	};
	assert_eq!(route_type(attempt.context.upstream), upstream);
	assert_eq!(
		llm_request.native_gemini(),
		upstream == RouteType::GenerateContent
	);
	assert!(llm_request.streaming);
	if matches!(client, Client::Responses) {
		assert_eq!(
			llm_request.params.reasoning_effort,
			Some(agent_llm::ReasoningEffort::Low)
		);
	}
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	if upstream == RouteType::Messages {
		// Messages requires an output limit; these clients sent none.
		assert_eq!(sent["max_tokens"], MESSAGES_DEFAULT_MAX_TOKENS.get());
	}
	if let UpstreamProtocol::Responses {
		safety_identifier, ..
	} = attempt.context.upstream
	{
		assert_eq!(safety_identifier, matches!(provider, AIProvider::OpenAI(_)));
	}
}

/// A provider that serves the client's own protocol needs no conversion.
#[rstest]
#[case::messages_to_anthropic(Client::Messages, anthropic(), "claude-sonnet-4-5")]
#[case::responses_to_openai(Client::Responses, openai(None), "gpt-5")]
#[case::chat_to_openai(Client::Chat, openai(None), "gpt-5")]
#[case::chat_to_copilot_claude(Client::Chat, copilot(), "claude-sonnet-4.5")]
#[case::responses_to_azure(
	Client::Responses,
	azure(crate::llm::azure::AzureResourceType::OpenAI),
	"gpt-5"
)]
#[case::messages_to_foundry_claude(
	Client::Messages,
	azure(crate::llm::azure::AzureResourceType::Foundry),
	"claude-sonnet-4-5"
)]
#[case::gemini_to_gemini(Client::Gemini, gemini(), "gemini-2.5-pro")]
#[tokio::test]
async fn the_clients_own_protocol_uses_passthrough(
	#[case] client: Client,
	#[case] provider: AIProvider,
	#[case] model: &str,
) {
	let result = process_route(&provider, client.route_type(), None, client.request(model)).await;
	let RequestResult::Success { ir_reply, .. } = result.unwrap() else {
		panic!("expected a forwarded request");
	};
	assert!(
		matches!(ir_reply, Some(ReplyState::Passthrough(_))),
		"{ir_reply:?}"
	);
}

/// Foundry serves Claude only on its Anthropic endpoint, which needs `anthropic-version`. The
/// provider's setup reads both from what the IR returns: the upstream route and served model.
#[rstest]
#[case::converted_through_an_alias(Client::Chat, "fast", RouteType::Messages)]
#[case::passthrough(Client::Messages, "claude-sonnet-4-5", RouteType::Messages)]
#[tokio::test]
async fn foundry_claude_goes_to_the_anthropic_endpoint(
	#[case] client: Client,
	#[case] model: &str,
	#[case] upstream: RouteType,
) {
	let provider = azure(crate::llm::azure::AzureResourceType::Foundry);
	let policy = Policy {
		model_aliases: HashMap::from([("fast".into(), "claude-sonnet-4-5".into())]),
		..Default::default()
	};
	let RequestResult::Success {
		mut request,
		llm_request,
		upstream_route_type,
		..
	} = process_route(
		&provider,
		client.route_type(),
		Some(&policy),
		client.request(model),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert_eq!(upstream_route_type, upstream);
	assert_eq!(llm_request.request_model, "claude-sonnet-4-5");
	// The proxy's request names the gateway it arrived at.
	*request.uri_mut() = format!("http://gateway{}", request.uri()).parse().unwrap();
	provider
		.setup_request(
			&mut request,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
			None,
			None,
		)
		.unwrap();
	assert_eq!(request.uri().path(), "/anthropic/v1/messages");
	assert!(request.headers().contains_key("anthropic-version"));
}

#[tokio::test]
async fn a_gemini_stream_without_sse_is_rejected() {
	let body = json!({"contents": [{"role": "user", "parts": [{"text": "hello"}]}]});
	let req = messages_request("/v1beta/models/gpt-5:streamGenerateContent", &body);
	let result = process_route(&openai(None), RouteType::GenerateContent, None, req).await;
	let Ok(RequestResult::Rejected(response)) = result else {
		panic!("expected a rejection, got {result:?}");
	};
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_eligible_request_is_rendered_by_the_ir() {
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"temperature": 0.5,
		"stream": true,
		"output_config": {"effort": "high"},
		"system": [{"type": "text", "text": "Be brief."}],
		"tools": [{"name": "lookup", "input_schema": {"type": "object"}}],
		"messages": [{"role": "user", "content": "hello"}]
	});
	let RequestResult::Success {
		ir_reply,
		request,
		llm_request,
		upstream_route_type,
		..
	} = process(&openai(None), None, messages_request("/v1/messages", &body))
		.await
		.unwrap()
	else {
		panic!("expected a converted request");
	};

	let sent = request.into_body().collect().await.unwrap().to_bytes();
	let want = convert(&body, "gpt-5");
	assert_eq!(std::str::from_utf8(&sent).unwrap(), want.body);
	assert_eq!(upstream_route_type, RouteType::Responses);
	assert_eq!(llm_request.input_format, InputFormat::Messages);
	assert_eq!(llm_request.request_model, "gpt-5");
	assert_eq!(llm_request.provider, "openai");
	assert!(llm_request.streaming);
	assert_eq!(llm_request.params.temperature, Some(0.5));
	assert_eq!(llm_request.params.max_tokens, Some(64));
	assert_eq!(
		llm_request.params.reasoning_effort,
		Some(agent_llm::ReasoningEffort::High)
	);
	let Some(ReplyState::Convert(attempt)) = ir_reply else {
		panic!("expected IR attempt state");
	};
	assert_eq!(
		attempt.context.scope(),
		StateScope {
			backend: "ns/backend",
			provider: "openai",
			model: "gpt-5",
		}
	);
}

#[tokio::test]
async fn a_provider_error_keeps_request_context_for_response_policies() {
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "hello"}]
	});
	let RequestResult::Success {
		ir_reply,
		llm_request,
		..
	} = process(&openai(None), None, messages_request("/v1/messages", &body))
		.await
		.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Convert(_))));
	let provider_error = ::http::Response::builder()
		.status(429)
		.body(Body::from(
			r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"slow down"}}"#,
		))
		.unwrap();

	let response = openai(None)
		.process_response(
			crate::proxy::httpproxy::PolicyClient::new(
				crate::test_helpers::proxymock::setup_proxy_test("{}")
					.unwrap()
					.pi,
			),
			llm_request,
			ir_reply,
			LLMResponsePolicies::default(),
			None,
			LLMLogging::default(),
			None,
			provider_error,
		)
		.await
		.unwrap();

	let context = response
		.extensions()
		.get::<LLMContext>()
		.expect("provider errors keep the request LLM context");
	assert_eq!(context.provider, "openai");
	assert_eq!(context.request_model, "gpt-5");
	assert!(context.response_model.is_none());
}

/// Each attempt converts the client's body again from its replay, and what it records is
/// what it sends.
#[tokio::test]
async fn a_retry_converts_the_replayed_body_again() {
	use crate::http::retry::ReplayBody;
	let client = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "hello"}]
	});
	let original = bytes::Bytes::from(client.to_string());
	let mut body = Body::from(original.clone());
	body.record(4096);
	let (content, state) = body.into_replay_parts();
	let replay = ReplayBody::try_new(content, 4096).unwrap();
	let retry = replay.clone();
	let want = convert(&client, "gpt-5").body;
	let mut reply_ids = Vec::new();
	for replay in [replay, retry] {
		let body = state.wrap(crate::http::RawBody::new(replay));
		let recording = body.recorded().unwrap().clone();
		let req = ::http::Request::builder()
			.method("POST")
			.uri("/v1/messages")
			.body(body)
			.unwrap();
		let RequestResult::Success {
			request, ir_reply, ..
		} = process(&openai(None), None, req).await.unwrap()
		else {
			panic!("expected a converted request");
		};
		let sent = request.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(std::str::from_utf8(&sent).unwrap(), want);
		assert!(recording.is_complete());
		assert_eq!(recording.bytes(), sent);
		let Some(ReplyState::Convert(attempt)) = ir_reply else {
			panic!("expected IR attempt state");
		};
		assert_ne!(attempt.context.fallback_reply_id, "");
		reply_ids.push(attempt.context.fallback_reply_id);
	}
	assert_ne!(reply_ids[0], reply_ids[1]);
}

#[rstest]
#[case::body_model("/v1/messages", Some("gpt-5"), None, None, "gpt-5", None)]
#[case::provider_override(
	"/v1/messages",
	Some("gpt-5"),
	Some("gpt-5.6"),
	None,
	"gpt-5.6",
	Some("gpt-5")
)]
#[case::alias("/v1/messages", Some("fast"), None, Some(("fast", "gpt-5-mini")), "gpt-5-mini", Some("fast"))]
#[case::alias_of_the_override("/v1/messages", Some("gpt-5"), Some("fast"), Some(("fast", "gpt-5-mini")), "gpt-5-mini", Some("gpt-5"))]
#[case::body_model_over_path_model(
	"/v1/projects/p/locations/l/publishers/openai/models/gpt-5-nano:rawPredict",
	Some("gpt-5"),
	None,
	None,
	"gpt-5",
	None
)]
#[tokio::test]
async fn the_mapped_model_follows_legacy_precedence(
	#[case] uri: &str,
	#[case] body_model: Option<&str>,
	#[case] model_override: Option<&str>,
	#[case] alias: Option<(&str, &str)>,
	#[case] mapped: &str,
	#[case] original: Option<&str>,
) {
	let mut body = json!({"max_tokens": 8, "messages": [{"role": "user", "content": "hi"}]});
	if let Some(model) = body_model {
		body["model"] = json!(model);
	}
	let policy = alias.map(|(from, to)| Policy {
		model_aliases: HashMap::from([(from.into(), to.into())]),
		..Default::default()
	});
	let RequestResult::Success {
		llm_request,
		ir_reply,
		..
	} = process(
		&openai(model_override),
		policy.as_ref(),
		messages_request(uri, &body),
	)
	.await
	.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert_eq!(llm_request.request_model, mapped);
	assert_eq!(llm_request.original_model.as_deref(), original);
	let Some(ReplyState::Convert(attempt)) = ir_reply else {
		panic!("expected IR attempt state");
	};
	let pair = attempt.pair(1024);
	assert_eq!(pair.scope.backend, "ns/backend");
	assert_eq!(pair.scope.provider, "openai");
	assert_eq!(pair.scope.model, mapped);
}

/// As on legacy, a body that names no model is served the provider's override, else a model
/// the path names, and a passthrough names it to the provider. With neither it is refused.
#[tokio::test]
async fn a_body_with_no_model_takes_the_override_or_path_model() {
	let body = json!({"max_tokens": 8, "messages": [{"role": "user", "content": "hi"}]});
	let path_model = "/v1/projects/p/locations/l/publishers/openai/models/gpt-5-nano:rawPredict";
	let RequestResult::Success {
		llm_request,
		ir_reply,
		..
	} = process(&openai(None), None, messages_request(path_model, &body))
		.await
		.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert_eq!(llm_request.request_model, "gpt-5-nano");
	assert!(matches!(ir_reply, Some(ReplyState::Convert(_))));

	let chat = json!({"messages": [{"role": "user", "content": "hi"}]});
	let RequestResult::Success {
		request,
		llm_request,
		ir_reply,
		..
	} = process_route(
		&openai(Some("gpt-5-mini")),
		RouteType::Completions,
		None,
		messages_request("/v1/chat/completions", &chat),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	assert_eq!(llm_request.request_model, "gpt-5-mini");
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(sent["model"], "gpt-5-mini");

	let result = process(&openai(None), None, messages_request("/v1/messages", &body)).await;
	assert!(matches!(result, Err(AIError::MissingField(_))));
}

/// As legacy does, the OpenAI provider's inline moderation goes in each request it sends,
/// converted or not, replacing any the client set.
#[rstest]
#[case::messages_converted(Client::Messages, RouteType::Responses)]
#[case::gemini_converted(Client::Gemini, RouteType::Responses)]
#[case::chat_passthrough(Client::Chat, RouteType::Completions)]
#[case::responses_passthrough(Client::Responses, RouteType::Responses)]
#[tokio::test]
async fn the_providers_inline_moderation_is_sent(
	#[case] client: Client,
	#[case] upstream: RouteType,
) {
	let moderation: crate::llm::openai::ModerationParam = from_json(json!({
		"model": "omni-moderation-latest",
		"policy": {"input": {"mode": "block"}},
	}));
	let provider = AIProvider::OpenAI(crate::llm::openai::Provider {
		model_override: None,
		moderation: Some(moderation.clone()),
	});
	let req = client.request("gpt-5");
	let uri = req.uri().to_string();
	let mut body: Value =
		serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes()).unwrap();
	if matches!(client, Client::Chat | Client::Responses) {
		body["moderation"] = json!({"model": "client-choice"});
	}
	let RequestResult::Success {
		request,
		upstream_route_type,
		..
	} = process_route(
		&provider,
		client.route_type(),
		None,
		messages_request(&uri, &body),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert_eq!(upstream_route_type, upstream);
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(
		sent["moderation"],
		serde_json::to_value(&moderation).unwrap()
	);
}

#[tokio::test]
async fn an_ir_model_without_a_chat_protocol_is_refused_with_its_advertised_formats() {
	let provider = AIProvider::Custom(agent_llm::custom::Provider {
		model_override: Some("backend-model".into()),
		provider_override: Some("embeddings-only".into()),
		formats: vec![agent_llm::custom::ProviderFormatConfig {
			format: agent_llm::custom::ProviderFormat::Embeddings,
			path: None,
		}],
	});
	let policy = Policy {
		model_aliases: HashMap::from([("backend-model".into(), "resolved-model".into())]),
		..Default::default()
	};
	let result = process_route(
		&provider,
		RouteType::Messages,
		Some(&policy),
		Client::Messages.request("client-model"),
	)
	.await;
	let Err(AIError::RequestRefused { problems }) = result else {
		panic!("expected an IR refusal without falling back to legacy");
	};
	assert_eq!(problems.len(), 1);
	for detail in [
		"embeddings-only",
		"resolved-model",
		"Messages",
		"Embeddings",
	] {
		assert!(problems[0].contains(detail), "{}", problems[0]);
	}
}

/// As on legacy, the provider's model is written into the client's JSON, then overrides,
/// transformations and defaults change it before anything reads it.
#[tokio::test]
async fn body_mutations_change_the_request_the_ir_reads() {
	let policy = Policy {
		overrides: Some(HashMap::from([("temperature".into(), json!(0.2))])),
		transformations: Some(HashMap::from([(
			"model".into(),
			expr(r#"llmRequest.model.stripPrefix("gcp/")"#),
		)])),
		defaults: Some(HashMap::from([
			("max_tokens".into(), json!(99)),
			("temperature".into(), json!(0.9)),
		])),
		..Default::default()
	};
	let client = json!({
		"model": "public-model",
		"temperature": 0.7,
		"messages": [{"role": "user", "content": "hello"}]
	});
	let RequestResult::Success {
		request,
		llm_request,
		..
	} = process(
		&openai(Some("gcp/gpt-5")),
		Some(&policy),
		messages_request("/v1/messages", &client),
	)
	.await
	.unwrap()
	else {
		panic!("expected a converted request");
	};

	let sent = request.into_body().collect().await.unwrap().to_bytes();
	let mutated = json!({
		"model": "gpt-5",
		"temperature": 0.2,
		"max_tokens": 99,
		"messages": [{"role": "user", "content": "hello"}]
	});
	assert_eq!(
		std::str::from_utf8(&sent).unwrap(),
		convert(&mutated, "gpt-5").body
	);
	assert_eq!(llm_request.request_model, "gpt-5");
	assert_eq!(llm_request.original_model.as_deref(), Some("public-model"));
	assert_eq!(llm_request.params.temperature, Some(0.2));
	assert_eq!(llm_request.params.max_tokens, Some(99));
}

/// A Gemini client's model is in the path; legacy writes it into the body for mutations.
#[tokio::test]
async fn body_mutations_see_a_gemini_clients_path_model() {
	let policy = Policy {
		transformations: Some(HashMap::from([(
			"model".into(),
			expr(r#"llmRequest.model.stripPrefix("x-")"#),
		)])),
		..Default::default()
	};
	let RequestResult::Success {
		request,
		llm_request,
		..
	} = process_route(
		&anthropic(),
		RouteType::GenerateContent,
		Some(&policy),
		Client::Gemini.request("x-claude-sonnet-4-5"),
	)
	.await
	.unwrap()
	else {
		panic!("expected a converted request");
	};
	assert_eq!(llm_request.request_model, "claude-sonnet-4-5");
	assert_eq!(
		llm_request.original_model.as_deref(),
		Some("x-claude-sonnet-4-5")
	);
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(sent["model"], "claude-sonnet-4-5");
}

/// A passthrough sends the mutated body as it is, so a mutation applies once.
#[tokio::test]
async fn body_mutations_apply_once_on_a_passthrough() {
	let policy = Policy {
		transformations: Some(HashMap::from([(
			"model".into(),
			expr(r#""p-" + llmRequest.model"#),
		)])),
		..Default::default()
	};
	let RequestResult::Success {
		ir_reply, request, ..
	} = process_route(
		&openai(None),
		RouteType::Completions,
		Some(&policy),
		Client::Chat.request("gpt-5"),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(sent["model"], "p-gpt-5");
}

/// What legacy wouldn't change reaches the provider byte for byte, unknown fields included.
#[tokio::test]
async fn a_passthrough_sends_the_clients_bytes() {
	let client = r#"{"model":"claude-sonnet-4-5",  "max_tokens":64,"x_unknown":{"a":[1, 2]},
			"messages":[{"role":"user","content":"hi"}]}"#;
	let req = ::http::Request::builder()
		.method("POST")
		.uri("/v1/messages")
		.body(Body::from(client))
		.unwrap();
	let RequestResult::Success {
		ir_reply,
		request,
		llm_request,
		upstream_route_type,
		..
	} = process_route(&anthropic(), RouteType::Messages, None, req)
		.await
		.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert!(matches!(ir_reply, Some(ReplyState::Passthrough(_))));
	assert_eq!(upstream_route_type, RouteType::Messages);
	assert!(!llm_request.streaming);
	assert_eq!(llm_request.params.max_tokens, Some(64));
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(std::str::from_utf8(&sent).unwrap(), client);
}

/// As legacy does, a passthrough names the served model to the provider and records the
/// client's.
#[tokio::test]
async fn a_passthrough_sends_the_served_model() {
	let client = json!({"model": "gpt-5", "messages": [{"role": "user", "content": "hi"}]});
	let RequestResult::Success {
		request,
		llm_request,
		..
	} = process_route(
		&openai(Some("gpt-5-mini")),
		RouteType::Completions,
		None,
		messages_request("/v1/chat/completions", &client),
	)
	.await
	.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert_eq!(llm_request.request_model, "gpt-5-mini");
	assert_eq!(llm_request.original_model.as_deref(), Some("gpt-5"));
	let sent: Value =
		serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes()).unwrap();
	assert_eq!(sent["model"], "gpt-5-mini");
	assert_eq!(sent["messages"], client["messages"]);
}

/// When the served model is the only edit, every other byte goes as the client sent it.
#[tokio::test]
async fn a_passthrough_replaces_only_the_model() {
	let client = r#"{"model": "claude-sonnet-4-5",  "max_tokens":64,
			"messages":[{"role":"user","content":"hi"}]}"#;
	let provider = AIProvider::Anthropic(from_json(json!({"model": "claude-opus-4-5"})));
	let req = ::http::Request::builder()
		.method("POST")
		.uri("/v1/messages")
		.body(Body::from(client))
		.unwrap();
	let RequestResult::Success { request, .. } =
		process_route(&provider, RouteType::Messages, None, req)
			.await
			.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(
		std::str::from_utf8(&sent).unwrap(),
		client.replace("claude-sonnet-4-5", "claude-opus-4-5")
	);
}

/// A passthrough's settings come from the model router's parse, and its bytes
/// go on as they are. The two differ here only so the test can tell which was read.
#[tokio::test]
async fn a_routed_passthrough_reads_the_routers_parse() {
	let client = json!({"model": "o3", "max_tokens": 1,
		"messages": [{"role": "user", "content": "hi"}]});
	let mut parsed = client.clone();
	parsed["max_tokens"] = json!(9);
	let mut req = messages_request("/v1/chat/completions", &client);
	req.body_mut().insert_extension(ParsedJson(parsed));
	let RequestResult::Success {
		request,
		llm_request,
		..
	} = process_route(&openai(None), RouteType::Completions, None, req)
		.await
		.unwrap()
	else {
		panic!("expected a forwarded request");
	};
	assert_eq!(llm_request.params.max_tokens, Some(9));
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(sent, client.to_string());
}

/// Final transformations see and change the provider's body, after every stage.
#[tokio::test]
async fn final_transformations_change_the_rendered_request() {
	let policy = Policy {
		final_transformations: Some(HashMap::from([
			(
				"max_output_tokens".into(),
				expr("llmRequest.max_output_tokens * 2"),
			),
			("user".into(), expr("'from-policy'")),
		])),
		..Default::default()
	};
	let client = json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "hello"}]
	});
	let RequestResult::Success { request, .. } = process(
		&openai(None),
		Some(&policy),
		messages_request("/v1/messages", &client),
	)
	.await
	.unwrap() else {
		panic!("expected a converted request");
	};

	let sent = request.into_body().collect().await.unwrap().to_bytes();
	let sent: Value = serde_json::from_slice(&sent).unwrap();
	let mut want: Value = serde_json::from_str(&convert(&client, "gpt-5").body).unwrap();
	assert_eq!(want["max_output_tokens"], json!(64));
	want["max_output_tokens"] = json!(128);
	want["user"] = json!("from-policy");
	assert_eq!(sent, want);
}

/// A model before the conversation is read without the rest of the body, so the read of
/// the whole body refuses a repeat after it, whether the request is converted or not.
#[rstest::rstest]
#[case::messages_converted(
	openai(None),
	RouteType::Messages,
	"/v1/messages",
	r#"{"model":"gpt-5","max_tokens":8,"messages":[{"role":"user","content":"hi"}],"model":"o3"}"#
)]
#[case::responses_converted(
	anthropic(),
	RouteType::Responses,
	"/v1/responses",
	r#"{"model":"claude-sonnet-4-5","input":"hi","model":"o3"}"#
)]
#[case::chat_converted(
	anthropic(),
	RouteType::Completions,
	"/v1/chat/completions",
	r#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"hi"}],"model":"o3"}"#
)]
#[case::passthrough(anthropic(), RouteType::Messages, "/v1/messages",
	r#"{"model":"claude-sonnet-4-5","max_tokens":8,"messages":[{"role":"user","content":"hi"}],"model":"o3"}"#)]
#[tokio::test]
async fn a_model_repeated_after_the_conversation_is_refused(
	#[case] provider: AIProvider,
	#[case] route: RouteType,
	#[case] path: &str,
	#[case] body: &'static str,
) {
	let req = ::http::Request::builder()
		.method("POST")
		.uri(path)
		.body(Body::from(body))
		.unwrap();
	let result = process_route(&provider, route, None, req).await;
	let Err(ref refusal @ AIError::RequestRefused { .. }) = result else {
		panic!("expected a refusal, got {result:?}");
	};
	assert!(refusal.to_string().contains("model"), "{refusal}");
}

/// The model comes from the read that converts the request, so a model that isn't a name is
/// refused with the rest of what that read found.
#[tokio::test]
async fn a_model_that_is_not_a_name_is_refused() {
	let body = json!({
		"model": 5,
		"max_tokens": 8,
		"messages": [{"role": "user", "content": "hi"}]
	});
	let result = process(&openai(None), None, messages_request("/v1/messages", &body)).await;
	let Err(ref refusal @ AIError::RequestRefused { .. }) = result else {
		panic!("expected a refusal, got {result:?}");
	};
	assert!(refusal.to_string().contains("model"), "{refusal}");
}

#[tokio::test]
async fn a_request_the_ir_cannot_convert_is_refused() {
	let body = json!({
		"model": "gpt-5",
		"max_tokens": 8,
		"messages": [
			{"role": "user", "content": "hi"},
			{"role": "assistant", "content": [
				{"type": "thinking", "thinking": "", "signature": "not-a-carrier"},
				{"type": "text", "text": "hello"}
			]},
			{"role": "user", "content": "again"}
		]
	});
	let result = process(&openai(None), None, messages_request("/v1/messages", &body)).await;
	let Err(ref refusal @ AIError::RequestRefused { .. }) = result else {
		panic!("expected a refusal, got {result:?}");
	};
	assert!(
		refusal.to_string().contains("unknown carrier format"),
		"{refusal}"
	);
}

fn regex_guard(guard: &Value) -> Policy {
	from_json(json!({"promptGuard": {"request": [{"regex": guard}]}}))
}

/// A Messages request with a text in each place a request guard can look.
fn guarded_body(secret: &str) -> Value {
	json!({
		"model": "gpt-5",
		"max_tokens": 64,
		"system": format!("Never say {secret}."),
		"messages": [
			{"role": "user", "content": format!("What is the {secret}?")},
			{"role": "assistant", "content": [
				{"type": "tool_use", "id": "call_1", "name": "lookup", "input": {"query": secret}}]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "call_1", "content": format!("The {secret} is 42.")}]}
		]
	})
}

/// Processes a request with a log, as the proxy does, and returns the metrics it counted
/// guard checks in.
async fn process_logged(
	policy: &Policy,
	body: &Value,
) -> (
	RequestResult,
	crate::telemetry::log::RequestLog,
	std::sync::Arc<crate::telemetry::metrics::Metrics>,
) {
	let provider = openai(None);
	let Selection::Ir(selected) = select(
		RouteType::Messages,
		&named(provider.clone()),
		&with_policy(policy.clone()),
		"ns/backend".into(),
		None,
	)
	.unwrap() else {
		panic!("expected chat IR selection");
	};
	let inputs = crate::test_helpers::proxymock::setup_proxy_test("{}")
		.unwrap()
		.pi;
	let metrics = inputs.metrics.clone();
	let backend_info = crate::http::auth::BackendInfo {
		target: crate::types::agent::BackendTarget::Invalid,
		call_target: crate::types::agent::Target::from(("api.openai.com", 443)),
		inputs,
	};
	let mut log = crate::test_helpers::make_min_req_log();
	let result = provider
		.process_request_ir(
			selected,
			&backend_info,
			Some(policy),
			messages_request("/v1/messages", body),
			&mut Some(&mut log),
			None,
		)
		.await
		.unwrap();
	(result, log, metrics)
}

fn guard_checks(
	metrics: &crate::telemetry::metrics::Metrics,
	action: crate::telemetry::metrics::GuardrailAction,
) -> u64 {
	metrics
		.guardrail_checks
		.get_or_create(&crate::telemetry::metrics::GuardrailLabels {
			phase: crate::telemetry::metrics::GuardrailPhase::Request,
			action,
		})
		.get()
}

/// The guard's default scope covers the system prompt, messages and tool results, but not
/// tool call arguments.
#[tokio::test]
async fn a_regex_request_guard_masks_the_converted_request() {
	use crate::telemetry::metrics::GuardrailAction;
	let policy = regex_guard(&json!({"action": "mask", "rules": [{"pattern": "secret"}]}));
	let (result, log, metrics) = process_logged(&policy, &guarded_body("secret")).await;
	let RequestResult::Success { request, .. } = result else {
		panic!("expected a converted request, got {result:?}");
	};
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	let mut want = guarded_body("<masked>");
	want["messages"][1]["content"][0]["input"]["query"] = json!("secret");
	assert_eq!(
		std::str::from_utf8(&sent).unwrap(),
		convert(&want, "gpt-5").body
	);
	let entry = &log.guardrails.take().unwrap()[0];
	assert_eq!(
		(entry.phase.as_str(), entry.guard.as_str()),
		("request", "regex")
	);
	assert_eq!(entry.action, GuardrailAction::Mask);
	assert_eq!(guard_checks(&metrics, GuardrailAction::Mask), 1);
}

/// Each string value is masked on its own, so the arguments stay JSON.
#[tokio::test]
async fn a_regex_request_guard_masks_tool_call_arguments_in_scope() {
	let policy = regex_guard(&json!({"action": "mask", "rules": [{"pattern": "secret"}]}));
	let mut policy_value = serde_json::to_value(&policy).unwrap();
	policy_value["promptGuard"]["request"][0]["scope"] = json!(["toolInput"]);
	let policy: Policy = from_json(policy_value);
	let (result, ..) = process_logged(&policy, &guarded_body("secret")).await;
	let RequestResult::Success { request, .. } = result else {
		panic!("expected a converted request, got {result:?}");
	};
	let sent = request.into_body().collect().await.unwrap().to_bytes();
	let mut want = guarded_body("secret");
	want["messages"][1]["content"][0]["input"]["query"] = json!("<masked>");
	assert_eq!(
		std::str::from_utf8(&sent).unwrap(),
		convert(&want, "gpt-5").body
	);
}

#[tokio::test]
async fn a_regex_request_guard_rejects_with_its_response() {
	use crate::telemetry::metrics::GuardrailAction;
	let policy: Policy = from_json(json!({"promptGuard": {"request": [
		{"regex": {"action": "mask", "rules": [{"builtin": "email"}]}},
		{"regex": {"action": "reject", "rules": [{"pattern": "secret"}]},
			"rejection": {"status": 451, "body": "no secrets"}},
	]}}));
	let (result, mut log, metrics) = process_logged(&policy, &guarded_body("secret")).await;
	let RequestResult::GuardrailRejected {
		response,
		guardrail,
	} = result
	else {
		panic!("expected a rejection, got {result:?}");
	};
	assert_eq!(guardrail, "regex");
	assert_eq!(response.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);
	let body = response.into_body().collect().await.unwrap().to_bytes();
	assert_eq!(body, "no secrets");
	let actions: Vec<_> = log
		.guardrails
		.take()
		.unwrap()
		.into_iter()
		.map(|entry| entry.action)
		.collect();
	assert_eq!(actions, [GuardrailAction::Allow, GuardrailAction::Reject]);
	assert_eq!(guard_checks(&metrics, GuardrailAction::Allow), 1);
	assert_eq!(guard_checks(&metrics, GuardrailAction::Reject), 1);
	let recorded = log
		.llm_request
		.take()
		.expect("a rejection records the request");
	assert_eq!(recorded.request_model, "gpt-5");
	assert_eq!(recorded.params.max_tokens, Some(64));
}

/// Response guards run on streams only when streaming guards are enabled, as on legacy.
#[rstest]
#[case::buffered(false, false, 451, "Withheld.")]
#[case::streamed_unguarded(true, false, 200, "Hello there")]
#[case::streamed_guarded(true, true, 451, "Withheld.")]
#[tokio::test]
async fn response_guards_run_where_legacy_runs_them(
	#[case] streams: bool,
	#[case] streaming_guards: bool,
	#[case] status: u16,
	#[case] want: &str,
) {
	let guard = json!({"response": [{"regex": {"action": "reject", "rules": [{"pattern": "there"}]},
		"rejection": {"status": 451, "body": "Withheld."}}]});
	let policy: Policy = from_json(json!({"promptGuard": guard}));
	let body = json!({"model": "gpt-5", "max_tokens": 64, "stream": streams,
		"messages": [{"role": "user", "content": "hi"}]});
	let RequestResult::Success {
		ir_reply,
		llm_request,
		..
	} = process(
		&openai(None),
		Some(&policy),
		messages_request("/v1/messages", &body),
	)
	.await
	.unwrap()
	else {
		panic!("expected a converted request");
	};
	let message = r#"{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}"#;
	let usage = r#"{"input_tokens":3,"output_tokens":2,"total_tokens":5}"#;
	let completed = format!(
		r#"{{"id":"resp_1","object":"response","model":"gpt-5","status":"completed","output":[{message}],"usage":{usage}}}"#
	);
	let provider_body = if streams {
		[
			r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","object":"response","model":"gpt-5","status":"in_progress","output":[]}}"#.to_string(),
			r#"{"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}"#.to_string(),
			r#"{"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#.to_string(),
			r#"{"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello there"}"#.to_string(),
			r#"{"type":"response.output_text.done","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"text":"Hello there"}"#.to_string(),
			r#"{"type":"response.content_part.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]}}"#.to_string(),
			format!(r#"{{"type":"response.output_item.done","sequence_number":6,"output_index":0,"item":{message}}}"#),
			format!(r#"{{"type":"response.completed","sequence_number":7,"response":{completed}}}"#),
		]
		.iter()
		.fold(String::new(), |mut stream, data| {
			write!(stream, "data: {data}\n\n").unwrap();
			stream
		})
	} else {
		completed
	};
	let mut provider = ::http::Response::builder().status(200);
	if streams {
		provider = provider.header("content-type", "text/event-stream");
	}
	let response = openai(None)
		.process_response(
			crate::test_helpers::policy_client(),
			llm_request,
			ir_reply,
			LLMResponsePolicies {
				prompt_guard: policy.prompt_guard.clone().unwrap().response,
				streaming_prompt_guard_enabled: streaming_guards,
				..Default::default()
			},
			None,
			LLMLogging::default(),
			None,
			provider.body(Body::from(provider_body)).unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), status);
	if status == 451 {
		assert!(!response.headers().contains_key("content-type"));
	}
	let sent = response.into_body().collect().await.unwrap().to_bytes();
	let sent = std::str::from_utf8(&sent).unwrap();
	let text: String = if streams && !streaming_guards {
		sent
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.filter_map(|data| serde_json::from_str::<Value>(data).ok())
			.filter_map(|event| event["delta"]["text"].as_str().map(str::to_string))
			.collect()
	} else {
		sent.to_string()
	};
	assert_eq!(text, want, "{sent}");
}

/// What CEL reads as `llm.prompt` follows enrichment and guards: from the conversation when
/// converted, and read as legacy reads the client's format on a passthrough.
#[rstest]
#[case::messages_converted(Client::Messages, json!([
	{"role": "system", "content": "Rule"},
	{"role": "user", "content": "<masked>"},
	{"role": "assistant", "content": "After"},
]))]
#[case::gemini_converted(Client::Gemini, json!([
	{"role": "system", "content": "Rule"},
	{"role": "user", "content": "<masked>"},
	{"role": "model", "content": "After"},
]))]
#[case::chat_passthrough(Client::Chat, json!([
	{"role": "system", "content": "Rule"},
	{"role": "user", "content": "hello"},
	{"role": "assistant", "content": "After"},
]))]
#[tokio::test]
async fn the_prompt_cel_reads_is_recorded(#[case] client: Client, #[case] expected: Value) {
	let mut policy: Policy = from_json(json!({"prompts": {
		"prepend": [{"role": "system", "content": "Rule"}],
		"append": [{"role": "assistant", "content": "After"}],
	}}));
	// A guard would keep a passthrough on legacy.
	if !matches!(client, Client::Chat) {
		policy.prompt_guard =
			regex_guard(&json!({"action": "mask", "rules": [{"pattern": "hello"}]})).prompt_guard;
	}
	let provider = openai(None);
	let Selection::Ir(selected) = select(
		client.route_type(),
		&named(provider.clone()),
		&with_policy(policy.clone()),
		"ns/backend".into(),
		None,
	)
	.unwrap() else {
		panic!("expected chat IR selection");
	};
	let backend_info = crate::http::auth::BackendInfo {
		target: crate::types::agent::BackendTarget::Invalid,
		call_target: crate::types::agent::Target::from(("api.openai.com", 443)),
		inputs: crate::test_helpers::proxymock::setup_proxy_test("{}")
			.unwrap()
			.pi,
	};
	let mut log = crate::test_helpers::make_min_req_log();
	log
		.cel
		.cel_context
		.register_log_expression(&expr("llm.prompt"));
	let result = provider
		.process_request_ir(
			selected,
			&backend_info,
			Some(&policy),
			client.request("gpt-5"),
			&mut Some(&mut log),
			None,
		)
		.await
		.unwrap();
	let RequestResult::Success { llm_request, .. } = result else {
		panic!("expected a forwarded request, got {result:?}");
	};
	assert_eq!(serde_json::to_value(llm_request.prompt).unwrap(), expected);
}
