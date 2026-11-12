//! Runs legacy goldens through the LLM IR for a Chat Completions client: its requests rendered
//! for a Messages or Gemini provider, and those providers' replies written back for it. Every
//! input's IR result is snapshotted beside the legacy snapshots: `<case>.llm-ir-messages.snap` or `<case>.llm-ir-gemini.snap` for
//! requests, and `<case>.llm-ir-completions.snap` for replies.
//!
//! Only inputs legacy's goldens send on a route are compared there; the rest are outside it.
//! Legacy has no route from a Chat Completions client to a Responses provider, so that pair has
//! no goldens here. Replies are compared as what the client can read from them: text,
//! reasoning, calls, how the reply ended, and the usage accounting read.

use std::collections::BTreeSet;
use std::fs;
use std::num::NonZeroU64;
use std::sync::Arc;

use agent_core::strng;
use agent_llm::conversion::{messages, vertex_gemini};
use agent_llm::model_catalog::{ModelCatalogHandle, Reasoning, tags};
use agent_llm::types::completions;
use agent_llm::{CacheTokenConvention, InputFormat, LLMResponse, LogContentFields};
use agent_llm_ir::{ClientProtocol, ReplyContext, StateScope, Upstream, UpstreamProtocol};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{Value, json};

use super::Disposition::*;
use super::{
	Disposition, FixedIdentifier, IrRequest, assert_every_input_listed, assert_reply_refusal,
	assert_request_refusal, describe, drive_buffered_reply, drive_streamed_reply, ir_request,
	ir_usage, legacy_stream_usage, legacy_usage, model_of, reply_snapshot, snapshot,
};

#[derive(Debug, Clone, Copy)]
enum Provider {
	Messages,
	Gemini,
}

const REQUESTS: &str = "src/tests/requests/completions";

const SUMMARIZED: &str =
	"a client that sets `reasoning_effort` gets summarized thinking, to show as `reasoning_content`";
const RESULTS_JOINED: &str =
	"the results of parallel calls are one user turn, where legacy sends a turn for each";
const REASONING_SIGNATURE: &str = "`reasoning_signature`, legacy's field for a provider's raw thinking signature; the IR replays reasoning sealed into call ids";
const FILE_PARTS: &str = "`file` parts, which the IR has no form for";
const REMOTE_IMAGE: &str = "an image by URL, which the provider would fetch";
const JSON_TOOL_RESULTS: &str = "complete JSON tool results are structured output values, where legacy embeds their JSON text in content";

const ON_MESSAGES: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	("cache_control_blank_text", OutsideRoute),
	("file-inline", Unsupported(FILE_PARTS)),
	("file-uri", OutsideRoute),
	(
		"full",
		Unsupported(
			"`n`, penalties, `logit_bias`, `seed`, the `priority` tier, `functions`, participant names, and `json_object`, which Messages has no form for",
		),
	),
	("generation-config", OutsideRoute),
	("image-file", OutsideRoute),
	("image-inline", OutsideRoute),
	("image-url", Unsupported(REMOTE_IMAGE)),
	("multi-turn-tools", OutsideRoute),
	("parallel-tool-call", OutsideRoute),
	(
		"prompt-cache-breakpoint",
		Unsupported("`prompt_cache_breakpoint`, which the IR refuses from Responses clients too"),
	),
	(
		"reasoning",
		Changed(
			"`reasoning_effort` is adaptive thinking at that effort, where legacy gives this model a token budget; and a client that sets it gets summarized thinking",
		),
	),
	("reasoning-adaptive", Changed(SUMMARIZED)),
	("reasoning_max", Changed(SUMMARIZED)),
	("reasoning_replay", Unsupported(REASONING_SIGNATURE)),
	("reasoning_replay_unsigned", Equivalent),
	("structured-output", OutsideRoute),
	("tool-call", Changed(RESULTS_JOINED)),
];

const ON_GEMINI: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	("cache_control_blank_text", OutsideRoute),
	("file-inline", Unsupported(FILE_PARTS)),
	("file-uri", Unsupported(FILE_PARTS)),
	("full", OutsideRoute),
	(
		"generation-config",
		Unsupported(
			"`top_k`, which isn't a Chat Completions field, and `n`, penalties and `seed`, which the IR has no settings for",
		),
	),
	("image-file", Unsupported(REMOTE_IMAGE)),
	(
		"image-inline",
		Unsupported(
			"the media type `image/jpg`, which isn't registered; legacy sends it as `image/jpeg`",
		),
	),
	("image-url", OutsideRoute),
	("multi-turn-tools", Changed(JSON_TOOL_RESULTS)),
	("parallel-tool-call", Changed(JSON_TOOL_RESULTS)),
	("prompt-cache-breakpoint", OutsideRoute),
	("reasoning", Equivalent),
	("reasoning-adaptive", OutsideRoute),
	("reasoning_max", Equivalent),
	("reasoning_replay", OutsideRoute),
	("reasoning_replay_unsigned", OutsideRoute),
	("structured-output", Equivalent),
	("tool-call", Changed(JSON_TOOL_RESULTS)),
];

#[test]
fn every_chat_request_golden_has_a_disposition() {
	assert_every_input_listed(REQUESTS, ON_MESSAGES.iter().map(|(name, _)| *name));
	assert_every_input_listed(REQUESTS, ON_GEMINI.iter().map(|(name, _)| *name));
}

#[test]
fn chat_requests_on_a_messages_provider() {
	for (name, disposition) in ON_MESSAGES {
		compare_request(name, *disposition, Provider::Messages);
	}
}

#[test]
fn chat_requests_on_a_gemini_provider() {
	for (name, disposition) in ON_GEMINI {
		compare_request(name, *disposition, Provider::Gemini);
	}
}

fn compare_request(name: &str, disposition: Disposition, provider: Provider) {
	if matches!(disposition, OutsideRoute) {
		return;
	}
	let body = input(name, provider);
	let legacy = legacy_request(&body, provider);
	let ir = ir_request(
		&body,
		ClientProtocol::ChatCompletions,
		upstream(&body, provider),
	);
	let suffix = match provider {
		Provider::Messages => "messages",
		Provider::Gemini => "gemini",
	};
	snapshot(
		REQUESTS,
		&format!("{name}.llm-ir-{suffix}"),
		&describe(disposition),
		&body,
		&ir,
	);
	match (disposition, &legacy, &ir) {
		(Equivalent, Ok(legacy), Ok(ir)) => assert_eq!(
			expected_client_request(legacy, provider),
			without_representation(&ir.request),
			"{name} on {provider:?}"
		),
		(Changed(_) | LegacyBug(_), Ok(legacy), Ok(ir)) => {
			assert_request_change(name, provider, legacy, ir);
		},
		(Unsupported(_), Ok(_), Err(error)) => {
			assert_request_refusal(
				name,
				ClientProtocol::ChatCompletions,
				upstream(&body, provider).protocol,
				error,
			);
		},
		(disposition, legacy, ir) => panic!(
			"{name} on {provider:?}: {disposition:?}, but legacy gave {:?} and the IR {:?}",
			legacy.as_ref().map(|_| "a request"),
			ir.as_ref().map(|_| "a request"),
		),
	}
}

fn assert_request_change(name: &str, provider: Provider, legacy: &Value, ir: &IrRequest) {
	if matches!(provider, Provider::Gemini) {
		let outputs = match name {
			"multi-turn-tools" => vec![json!({"temp": 15}), json!({"temp": 9})],
			"parallel-tool-call" => vec![
				json!({"temperature": 15, "condition": "Cloudy"}),
				json!({"temperature": 22, "condition": "Sunny"}),
			],
			"tool-call" => vec![
				json!({"temperature": 15, "condition": "Cloudy"}),
				json!({"time": "2026-08-12T12:00:00Z"}),
			],
			_ => panic!("{name}: intentional change needs specific assertions"),
		};
		let mut expected = expected_client_request(legacy, provider);
		let responses: Vec<_> = expected["contents"]
			.as_array_mut()
			.unwrap()
			.iter_mut()
			.flat_map(|message| message["parts"].as_array_mut().unwrap())
			.filter_map(|part| part.get_mut("functionResponse"))
			.collect();
		assert_eq!(responses.len(), outputs.len());
		for (response, output) in responses.into_iter().zip(outputs) {
			let text = response["response"]["content"].as_str().unwrap();
			assert_eq!(response["response"], json!({"content": text}));
			assert_eq!(serde_json::from_str::<Value>(text).unwrap(), output);
			response["response"] = json!({"output": [output]});
		}
		assert_eq!(without_representation(&ir.request), expected, "{name}");
		let dropped: &[&str] = if name == "parallel-tool-call" {
			&[]
		} else {
			&["tool result name"]
		};
		assert_eq!(ir.dropped, dropped, "{name}: dropped fields");
		assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
		return;
	}
	assert!(
		matches!(provider, Provider::Messages),
		"{name}: unexpected changed route"
	);
	let mut expected = expected_client_request(legacy, provider);
	let expected_dropped: &[&str] = match name {
		"reasoning" => {
			assert_eq!(
				expected["thinking"],
				json!({"type": "enabled", "budget_tokens": 2048})
			);
			assert!(expected.get("output_config").is_none());
			expected["thinking"] = json!({"type": "adaptive", "display": "summarized"});
			expected["output_config"] = json!({"effort": "medium"});
			&[]
		},
		"reasoning-adaptive" | "reasoning_max" => {
			let effort = if name == "reasoning-adaptive" {
				"high"
			} else {
				"xhigh"
			};
			assert_eq!(expected["thinking"], json!({"type": "adaptive"}));
			assert_eq!(expected["output_config"], json!({"effort": effort}));
			expected["thinking"]["display"] = json!("summarized");
			&[]
		},
		"tool-call" => {
			let messages = expected["messages"].as_array_mut().unwrap();
			assert_eq!(messages.len(), 4);
			assert_eq!(messages[2]["role"], "user");
			assert_eq!(messages[3]["role"], "user");
			for (result_index, call_index) in [(2, 0), (3, 1)] {
				let content = messages[result_index]["content"].as_array().unwrap();
				assert_eq!(content.len(), 1);
				assert_eq!(content[0]["type"], "tool_result");
				assert_eq!(
					content[0]["tool_use_id"],
					messages[1]["content"][call_index]["id"]
				);
			}
			let mut last = messages.pop().unwrap();
			messages[2]["content"]
				.as_array_mut()
				.unwrap()
				.append(last["content"].as_array_mut().unwrap());
			&["tool result name"]
		},
		_ => panic!("{name}: intentional change needs specific assertions"),
	};
	assert_eq!(
		without_representation(&ir.request),
		expected,
		"{name}: changes beyond those declared"
	);
	assert_eq!(ir.dropped, expected_dropped, "{name}: dropped fields");
	assert!(ir.emulated_stop_sequences.is_empty(), "{name}");
}

/// Legacy's goldens send each request to Gemini as `gemini-2.5-pro`, and the model decides how
/// effort is written.
fn input(name: &str, provider: Provider) -> String {
	let body =
		fs::read_to_string(super::legacy_fixture(REQUESTS).join(format!("{name}.json"))).unwrap();
	match provider {
		Provider::Messages => body,
		Provider::Gemini => {
			let mut body: Value = serde_json::from_str(&body).unwrap();
			body["model"] = "gemini-2.5-pro".into();
			body.to_string()
		},
	}
}

fn legacy_request(body: &str, provider: Provider) -> Result<Value, String> {
	let request: completions::Request = serde_json::from_str(body).map_err(|err| err.to_string())?;
	let request = match provider {
		Provider::Messages => {
			let catalog = TestCatalog;
			messages::from_completions::translate(&request, None, Some(&catalog))
		},
		Provider::Gemini => vertex_gemini::from_completions::translate(&request, true),
	};
	let request = request.map_err(|err| err.to_string())?;
	Ok(serde_json::from_slice(&request).unwrap())
}

struct TestCatalog;

impl ModelCatalogHandle for TestCatalog {
	fn model_has_tag(&self, model: &str, tag: &str) -> bool {
		self
			.get_model_tags(model)
			.is_some_and(|tags| tags.contains(tag))
	}

	fn get_model_tags(&self, model: &str) -> Option<Arc<BTreeSet<String>>> {
		let tags: &[&str] = match model {
			"custom-adaptive-model" => &[tags::ADAPTIVE_THINKING],
			"claude-opus-4-6" => &[tags::ADAPTIVE_THINKING, tags::LEGACY_THINKING],
			_ => return None,
		};
		Some(Arc::new(
			tags.iter().map(|tag| (*tag).to_string()).collect(),
		))
	}

	fn reasoning(&self, _: &str, _: &str) -> Option<Reasoning> {
		None
	}
}

/// A Messages provider needs an output limit, which the gateway supplies from its model
/// catalog when a request has none. Legacy sends 4096, and sends Gemini none.
fn upstream(body: &str, provider: Provider) -> Upstream<'static> {
	let (name, protocol, default_max_tokens) = match provider {
		Provider::Messages => (
			"anthropic",
			UpstreamProtocol::Messages,
			NonZeroU64::new(4096),
		),
		Provider::Gemini => ("gemini", UpstreamProtocol::Gemini, None),
	};
	Upstream {
		identifier_pseudonymizer: &FixedIdentifier,
		scope: StateScope {
			backend: "golden",
			provider: name,
			model: model_of(body),
		},
		protocol,
		default_max_tokens,
	}
}

/// The default Messages cache policy marks the last system block and the last
/// block of the last user turn. Compare the complete request with those exact marks;
/// extra marks, wrong values, and missing marks must all fail.
fn expected_client_request(legacy: &Value, provider: Provider) -> Value {
	let mut expected = without_representation(legacy);
	if matches!(provider, Provider::Messages) {
		if let Some(system) = expected.get_mut("system").and_then(Value::as_array_mut)
			&& let Some(last) = system.last_mut()
		{
			assert!(last.get("cache_control").is_none());
			last["cache_control"] = json!({"type": "ephemeral"});
		}
		if let Some(last) = expected["messages"]
			.as_array_mut()
			.unwrap()
			.iter_mut()
			.rev()
			.find(|message| message["role"] == "user")
			.and_then(|message| message["content"].as_array_mut().unwrap().last_mut())
		{
			assert!(last.get("cache_control").is_none());
			last["cache_control"] = json!({"type": "ephemeral"});
		}
		if expected.get("tools").is_some() && expected.get("tool_choice").is_none() {
			expected["tool_choice"] = json!({"type": "auto"});
		}
	}
	expected
}

/// Empty tool lists/config objects mean no settings. A system prompt and a tool
/// result's string text have the same meaning as one text block. Cache marks and
/// tool choice remain in the comparison.
fn without_representation(request: &Value) -> Value {
	let mut request = request.clone();
	let object = request.as_object_mut().unwrap();
	if object.get("tools") == Some(&json!([])) {
		object.remove("tools");
	}
	if object.get("generationConfig") == Some(&json!({})) {
		object.remove("generationConfig");
	}
	if let Some(Value::String(text)) = object.get("system") {
		let blocks = json!([{"type": "text", "text": text}]);
		object.insert("system".to_string(), blocks);
	}
	for message in object
		.get_mut("messages")
		.and_then(Value::as_array_mut)
		.into_iter()
		.flatten()
	{
		for block in message["content"].as_array_mut().into_iter().flatten() {
			let block = block.as_object_mut().unwrap();
			if block.get("type") == Some(&json!("tool_result"))
				&& let Some(Value::String(text)) = block.get("content")
			{
				let parts = json!([{"type": "text", "text": text}]);
				block.insert("content".to_string(), parts);
			}
		}
	}
	request
}

const ANTHROPIC_REPLIES: &str = "src/tests/response/anthropic";
const GEMINI_REPLIES: &str = "src/tests/response/vertex-gemini";

const ITERATIONS: &str = "the IR counts the top-level usage, which leaves out the compaction and advisor iterations legacy sums as Anthropic bills them; a Chat Completions client can't ask for either";

/// Messages replies, written for a Chat Completions client. Names starting with `stream` are
/// streamed.
const FROM_MESSAGES: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	("count_tokens", OutsideRoute),
	("guardrails", OutsideRoute),
	("iterations", Changed(ITERATIONS)),
	(
		"multiple_text_blocks",
		Unsupported(
			"a `char_location` citation, which the Messages decoder doesn't read; a Chat Completions client can't send the document it cites",
		),
	),
	("stream_basic", Equivalent),
	("stream_error", Equivalent),
	("stream_iterations", Changed(ITERATIONS)),
	(
		"stream_message_delta_usage",
		Changed(
			"an engine whose `message_delta` restates the input with a different cache split: the IR keeps the largest of each count, where legacy takes the last",
		),
	),
	("stream_thinking", Equivalent),
	("stream_tool", Equivalent),
	("stream_tool_empty_args", Equivalent),
	("thinking", Equivalent),
	("tool", Equivalent),
];

/// Gemini replies, written for a Chat Completions client.
const FROM_GEMINI: &[(&str, Disposition)] = &[
	("basic", Equivalent),
	(
		"blocked",
		Changed("the IR leaves unreported output tokens unknown, where legacy records zero"),
	),
	("guardrails", OutsideRoute),
	("reasoning", Equivalent),
	("stream_tool", Equivalent),
	("tool", Equivalent),
];

#[test]
fn every_reply_golden_has_a_chat_disposition() {
	assert_every_input_listed(
		ANTHROPIC_REPLIES,
		FROM_MESSAGES.iter().map(|(name, _)| *name),
	);
	assert_every_input_listed(GEMINI_REPLIES, FROM_GEMINI.iter().map(|(name, _)| *name));
}

#[tokio::test]
async fn messages_replies_for_a_chat_client() {
	for (name, disposition) in FROM_MESSAGES {
		compare_reply(name, *disposition, Provider::Messages).await;
	}
}

#[tokio::test]
async fn gemini_replies_for_a_chat_client() {
	for (name, disposition) in FROM_GEMINI {
		compare_reply(name, *disposition, Provider::Gemini).await;
	}
}

/// What a client can read from a reply, and the usage accounting read.
#[derive(Debug, PartialEq)]
struct Reply {
	client: ClientReply,
	usage: Value,
}

#[derive(Debug, PartialEq)]
enum ClientReply {
	Body(Value),
	/// The `data` of each event, and whether the stream ended with `[DONE]`.
	Events {
		events: Vec<Value>,
		done: bool,
	},
	/// Legacy's failures are errors without an HTTP reply; both are compared as failures only.
	Failed {
		detail: String,
	},
}

impl ClientReply {
	/// The reply as its snapshot records it.
	fn report(&self) -> Value {
		match self {
			Self::Body(body) => json!({"body": body}),
			Self::Events { events, done } => json!({"events": events, "done": done}),
			Self::Failed { detail } => json!({"failed": detail}),
		}
	}
}

async fn compare_reply(name: &str, disposition: Disposition, provider: Provider) {
	if matches!(disposition, OutsideRoute) {
		return;
	}
	let directory = replies(provider);
	let body = fs::read(super::legacy_fixture(directory).join(format!("{name}.json"))).unwrap();
	let (legacy, ir) = if name.starts_with("stream") {
		(
			legacy_streamed_reply(&body, provider).await,
			ir_streamed_reply(&body, provider),
		)
	} else {
		(
			legacy_buffered_reply(&body, provider),
			ir_buffered_reply(&body, provider),
		)
	};
	let case = format!("{name}.llm-ir-completions");
	reply_snapshot(
		directory,
		&case,
		disposition,
		&body,
		&ir.client.report(),
		&ir.usage,
	);
	let (legacy, ir) = (readable(&legacy), readable(&ir));
	match disposition {
		Equivalent => assert_eq!(legacy, ir, "{name} from {provider:?}"),
		Changed(_) | LegacyBug(_) => assert_reply_usage_change(name, provider, &legacy, &ir),
		Unsupported(_) => {
			assert!(ir["reply"]["failed"] == true && legacy["reply"]["failed"] != true);
			assert!(matches!(provider, Provider::Messages));
			assert_eq!(name, "multiple_text_blocks");
			assert_reply_refusal(
				&body,
				reply_context(provider),
				false,
				"unsupported output item char_location",
			);
		},
		OutsideRoute => unreachable!("not converted"),
	}
}

fn assert_reply_usage_change(name: &str, provider: Provider, legacy: &Value, ir: &Value) {
	if matches!(provider, Provider::Gemini) {
		assert_eq!(name, "blocked");
		assert_eq!(legacy["usage"]["output_tokens"], json!(0));
		assert_eq!(ir["usage"]["output_tokens"], Value::Null);
		let mut expected = legacy.clone();
		expected["usage"]["output_tokens"] = Value::Null;
		assert_eq!(
			&expected, ir,
			"{name}: changes beyond unreported output tokens"
		);
		return;
	}
	assert!(
		matches!(provider, Provider::Messages),
		"{name}: unexpected changed route"
	);
	let changes: &[(&str, Value, Value)] = match name {
		"iterations" | "stream_iterations" => &[
			("input_tokens", json!(213), json!(23)),
			("output_tokens", json!(230), json!(10)),
			("total_tokens", json!(443), json!(33)),
			("cached_tokens", json!(50), Value::Null),
		],
		"stream_message_delta_usage" => &[
			("input_tokens", json!(34347), json!(34824)),
			("total_tokens", json!(34396), json!(34824)),
		],
		_ => panic!("{name}: intentional reply change needs specific assertions"),
	};
	let mut expected = legacy.clone();
	for (field, old, new) in changes {
		assert_eq!(&expected["usage"][*field], old, "{name}: legacy {field}");
		expected["usage"][*field] = new.clone();
	}
	assert_eq!(ir, &expected, "{name}: changes beyond those declared");
}

const fn replies(provider: Provider) -> &'static str {
	match provider {
		Provider::Messages => ANTHROPIC_REPLIES,
		Provider::Gemini => GEMINI_REPLIES,
	}
}

fn legacy_buffered_reply(body: &[u8], provider: Provider) -> Reply {
	let body = Bytes::copy_from_slice(body);
	let reply = match provider {
		Provider::Messages => messages::from_completions::translate_response(&body),
		Provider::Gemini => vertex_gemini::to_completions::translate_response(&body),
	};
	match reply {
		Ok(reply) => Reply {
			usage: legacy_usage_in_ir_terms(
				&reply.to_llm_response(LogContentFields::default()),
				provider,
			),
			client: ClientReply::Body(serde_json::from_slice(&reply.serialize().unwrap()).unwrap()),
		},
		Err(err) => Reply {
			client: ClientReply::Failed {
				detail: err.to_string(),
			},
			usage: Value::Null,
		},
	}
}

async fn legacy_streamed_reply(body: &[u8], provider: Provider) -> Reply {
	let (info, guard) = legacy_stream_usage(InputFormat::Completions);
	let input = agent_http::Body::from(body.to_vec());
	let output = match provider {
		Provider::Messages => messages::from_completions::translate_stream(
			input,
			1024 * 1024,
			guard,
			LogContentFields::default(),
		),
		Provider::Gemini => vertex_gemini::to_completions::translate_stream(
			input,
			1024 * 1024,
			strng::literal!("input-model"),
			guard,
			LogContentFields::default(),
		),
	};
	let output = output.collect().await.unwrap().to_bytes();
	let usage = legacy_usage_in_ir_terms(&info.lock().unwrap().response, provider);
	Reply {
		client: chat_events(&output),
		usage,
	}
}

/// Legacy records a Messages provider's input and total without cache reads and writes, and
/// says so with `InputExcludesCache`; the IR's input and total include them.
fn legacy_usage_in_ir_terms(response: &LLMResponse, provider: Provider) -> Value {
	let mut usage = legacy_usage(response);
	if let (Provider::Messages, Some(input)) = (provider, response.input_tokens) {
		let cache = CacheTokenConvention::InputExcludesCache.include_cache_tokens(
			input,
			response.cached_input_tokens,
			response.cache_creation_input_tokens,
		) - input;
		usage["input_tokens"] = json!(input + cache);
		if let Some(total) = response.total_tokens {
			usage["total_tokens"] = json!(total + cache);
		}
	}
	usage
}

fn ir_buffered_reply(body: &[u8], provider: Provider) -> Reply {
	let (reply, usage) = drive_buffered_reply(body, reply_context(provider));
	Reply {
		client: match reply.status {
			200 => ClientReply::Body(serde_json::from_slice(&reply.body).unwrap()),
			status => ClientReply::Failed {
				detail: format!("{status} {}", String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

fn ir_streamed_reply(body: &[u8], provider: Provider) -> Reply {
	let (reply, usage) = drive_streamed_reply(body, reply_context(provider));
	Reply {
		client: match reply {
			Ok(out) => chat_events(&out),
			Err(reply) => ClientReply::Failed {
				detail: format!("{} {}", reply.status, String::from_utf8_lossy(&reply.body)),
			},
		},
		usage: ir_usage(usage),
	}
}

const fn reply_context(provider: Provider) -> ReplyContext<'static> {
	ReplyContext {
		client: ClientProtocol::ChatCompletions,
		upstream: match provider {
			Provider::Messages => UpstreamProtocol::Messages,
			Provider::Gemini => UpstreamProtocol::Gemini,
		},
		scope: REPLY_SCOPE,
		routes: &[],
		show_reasoning: false,
		include_usage: true,
		created: CREATED,
		fallback_reply_id: "reply-1",
		stop_sequences: Vec::new(),
		max_held_bytes: usize::MAX,
	}
}

const REPLY_SCOPE: StateScope<'static> = StateScope {
	backend: "golden",
	provider: "provider",
	model: "input-model",
};
const CREATED: u64 = 1_700_000_000;

/// The `data` of each event of a well-formed SSE stream, read without the IR's parser.
fn chat_events(stream: &[u8]) -> ClientReply {
	let mut events = Vec::new();
	let mut done = false;
	for event in std::str::from_utf8(stream)
		.unwrap()
		.replace("\r\n", "\n")
		.split("\n\n")
	{
		let data: Vec<&str> = event
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.collect();
		match data.join("\n").as_str() {
			"" => {},
			"[DONE]" => done = true,
			data => events.push(serde_json::from_str(data).unwrap()),
		}
	}
	ClientReply::Events { events, done }
}

/// What the client reads, apart from how its format splits it: the answer's text, the
/// reasoning text, each call with parsed arguments, how the reply ended, and the usage
/// accounting read. A stream that ends in an error or without `[DONE]` failed. Only zero
/// cache and reasoning detail counts are equivalent to omitted detail counts.
fn readable(reply: &Reply) -> Value {
	let mut usage = reply.usage.clone();
	for field in ["cached_tokens", "reasoning_tokens"] {
		if usage[field] == json!(0) {
			usage[field] = Value::Null;
		}
	}
	let read = match &reply.client {
		ClientReply::Failed { .. } => json!({"failed": true}),
		ClientReply::Body(body) => {
			let choice = &body["choices"][0];
			let message = &choice["message"];
			let calls: Vec<_> = message["tool_calls"]
				.as_array()
				.into_iter()
				.flatten()
				.map(readable_call)
				.collect();
			read(
				message["content"].as_str().unwrap_or(""),
				message["reasoning_content"].as_str().unwrap_or(""),
				&calls,
				&choice["finish_reason"],
			)
		},
		ClientReply::Events { events, done } => {
			let mut reply_id = None;
			for id in events.iter().filter_map(|event| event.get("id")) {
				assert!(id.as_str().is_some_and(|id| !id.is_empty()));
				assert_eq!(
					reply_id.get_or_insert(id),
					&id,
					"reply id changed between chunks"
				);
			}
			if !done || events.iter().any(|event| event.get("error").is_some()) {
				json!({"failed": true})
			} else {
				streamed(events)
			}
		},
	};
	json!({"reply": read, "usage": usage})
}

/// A stream's chunks as one message. A call's pieces share its index.
fn streamed(events: &[Value]) -> Value {
	let (mut content, mut reasoning) = (String::new(), String::new());
	let mut calls: Vec<Value> = Vec::new();
	let mut finish_reason = Value::Null;
	for choice in events.iter().filter_map(|event| event["choices"].get(0)) {
		let delta = &choice["delta"];
		content.push_str(delta["content"].as_str().unwrap_or(""));
		reasoning.push_str(delta["reasoning_content"].as_str().unwrap_or(""));
		for call in delta["tool_calls"].as_array().into_iter().flatten() {
			let at = usize::try_from(call["index"].as_u64().unwrap()).unwrap();
			if at == calls.len() {
				calls.push(json!({"id": call["id"], "type": "function",
					"function": {"name": call["function"]["name"], "arguments": ""}}));
			}
			if let Some(id) = call.get("id") {
				assert_eq!(id, &calls[at]["id"], "call id changed between deltas");
			}
			if let Some(name) = call["function"].get("name") {
				assert_eq!(
					name, &calls[at]["function"]["name"],
					"call name changed between deltas"
				);
			}
			let arguments = call["function"]["arguments"].as_str().unwrap_or("");
			let whole = calls[at]["function"]["arguments"]
				.as_str()
				.unwrap()
				.to_string()
				+ arguments;
			calls[at]["function"]["arguments"] = whole.into();
		}
		if !choice["finish_reason"].is_null() {
			finish_reason = choice["finish_reason"].clone();
		}
	}
	let calls: Vec<_> = calls.iter().map(readable_call).collect();
	read(&content, &reasoning, &calls, &finish_reason)
}

fn readable_call(call: &Value) -> Value {
	let arguments = call["function"]["arguments"].as_str().unwrap();
	let arguments: Value = serde_json::from_str(arguments).unwrap_or_else(|_| arguments.into());
	json!({"id": call["id"], "name": call["function"]["name"], "arguments": arguments})
}

fn read(content: &str, reasoning: &str, calls: &[Value], finish_reason: &Value) -> Value {
	json!({"content": content, "reasoning": reasoning, "calls": calls,
		"finish_reason": finish_reason})
}

#[test]
fn normalization_preserves_cache_values_and_placement() {
	let request = json!({"system": [
		{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}},
		{"type": "text", "text": "world"}
	]});
	for mark in [json!({"type": "persistent"}), Value::Null] {
		let mut changed = request.clone();
		changed["system"][0]["cache_control"] = mark;
		assert_ne!(
			without_representation(&request),
			without_representation(&changed)
		);
	}
	let mut moved = request.clone();
	let mark = moved["system"][0]
		.as_object_mut()
		.unwrap()
		.remove("cache_control")
		.unwrap();
	moved["system"][1]["cache_control"] = mark;
	assert_ne!(
		without_representation(&request),
		without_representation(&moved)
	);
}

#[test]
fn streamed_call_ids_cannot_change_between_deltas() {
	let events = vec![
		json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1",
			"function": {"name": "read", "arguments": "{"}}]}}]}),
		json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "wrong",
			"function": {"arguments": "}"}}]}, "finish_reason": "tool_calls"}]}),
	];
	assert!(std::panic::catch_unwind(|| streamed(&events)).is_err());
}
