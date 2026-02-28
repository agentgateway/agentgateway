use std::fs;
use std::path::Path;

use agent_core::strng;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;

fn test_response(
	provider_name: &str,
	test_name: &str,
	xlate: impl Fn(Bytes) -> Result<Box<dyn ResponseType>, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let provider_str = &fs::read_to_string(&input_path)
		.unwrap_or_else(|_| panic!("{test_name}: Failed to read input file"));
	let provider_value = serde_json::from_str::<Value>(provider_str).unwrap();

	let resp = xlate(Bytes::copy_from_slice(provider_str.as_bytes()))
		.expect("Failed to translate provider response to OpenAI format");
	let raw = resp
		.serialize()
		.expect("Failed to serialize OpenAI response");
	let resp_val = serde_json::from_slice::<Value>(&raw).expect("Failed to parse OpenAI response");

	insta::with_settings!({
			info => &provider_value,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
	}, {
			 insta::assert_json_snapshot!(format!("{}-{}", provider_name, test_name), resp_val, {
			".id" => "[id]",
			".output.*.id" => "[id]",
			".created" => "[date]",
		});
	});
}

async fn test_streaming(
	provider_name: &str,
	test_name: &str,
	xlate: impl Fn(Body, AmendOnDrop) -> Result<Body, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(test_name);
	let provider =
		&fs::read(&input_path).unwrap_or_else(|_| panic!("{test_name}: Failed to read input file"));
	let body = Body::from(provider.clone());
	let log = AsyncLog::default();
	let resp = xlate(body, AmendOnDrop::new(log, LLMResponsePolicies::default()))
		.expect("failed to translate stream");
	let resp_bytes = resp.collect().await.unwrap().to_bytes();
	let resp_str = std::str::from_utf8(&resp_bytes).unwrap();

	insta::with_settings!({
			// info => "",
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
			filters => vec![
				("\"created\":[0-9]+","\"created\":123"),
				("\"created_at\":[0-9]+","\"created_at\":123"),
				("\"id\":\"(resp|msg|call)_[0-9a-f]+\"","\"id\":\"$1_xxx\""),
				("\"item_id\":\"(msg|call)_[0-9a-f]+\"","\"item_id\":\"$1_xxx\""),
				("\"call_id\":\"call_[0-9a-f]+\"","\"call_id\":\"call_xxx\""),
			]
	}, {
			 insta::assert_snapshot!(format!("{}-{}", provider_name, test_name), resp_str);
	});
}

fn test_request<I>(
	provider_name: &str,
	test_name: &str,
	xlate: impl Fn(I) -> Result<Vec<u8>, AIError>,
) where
	I: DeserializeOwned,
{
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let input_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let input_raw: Value = serde_json::from_str(input_str).expect("Failed to parse input json");
	let input_typed: I = serde_json::from_str(input_str).expect("Failed to parse input JSON");

	let provider_response =
		xlate(input_typed).expect("Failed to translate input format to provider request ");
	let provider_value =
		serde_json::from_slice::<Value>(&provider_response).expect("Failed to parse provider response");

	insta::with_settings!({
			info => &input_raw,
			description => format!("{}: {}", provider_name, test_name),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
	}, {
			 insta::assert_json_snapshot!(format!("{}-{}", provider_name, test_name), provider_value, {
			".id" => "[id]",
			".created" => "[date]",
		});
	});
}

const COMPLETION_REQUESTS: &[&str] = &[
	"request_basic",
	"request_full",
	"request_tool-call",
	"request_reasoning",
];
const ANTHROPIC_COMPLETION_REQUESTS: &[&str] = &["request_reasoning_max"];
const MESSAGES_REQUESTS: &[&str] = &[
	"request_anthropic_basic",
	"request_anthropic_tools",
	"request_anthropic_reasoning",
];
const RESPONSES_REQUESTS: &[&str] = &["request_responses_basic", "request_responses_instructions"];
const COUNT_TOKENS_REQUESTS: &[&str] = &[
	"request_count_tokens_basic",
	"request_count_tokens_with_system",
];
const EMBEDDINGS_REQUESTS: &[&str] = &["request_embeddings_basic", "request_embeddings_array"];
const STREAM_FIXTURE_BEDROCK_BASIC: &str = "response_stream-bedrock_basic.bin";
const STREAM_FIXTURE_ANTHROPIC_BASIC: &str = "response_stream-anthropic_basic.json";
const STREAM_FIXTURE_ANTHROPIC_THINKING: &str = "response_stream-anthropic_thinking.json";
const STREAM_FIXTURE_ANTHROPIC_TOOL: &str = "response_stream-anthropic_tool.json";

fn llm_request(
	input_format: InputFormat,
	provider: &str,
	model: &str,
	streaming: bool,
) -> LLMRequest {
	LLMRequest {
		input_tokens: None,
		input_format,
		request_model: strng::new(model),
		provider: strng::new(provider),
		streaming,
		params: LLMRequestParams::default(),
		prompt: None,
	}
}

fn provider_message_adapters() -> Vec<(&'static str, AIProvider)> {
	vec![
		(
			"openai",
			AIProvider::OpenAI(openai::Provider {
				model: Some(strng::new("gpt-4.1")),
			}),
		),
		(
			"gemini",
			AIProvider::Gemini(gemini::Provider {
				model: Some(strng::new("gemini-2.5-pro")),
			}),
		),
		(
			"azure",
			AIProvider::AzureOpenAI(azureopenai::Provider {
				model: Some(strng::new("gpt-4.1")),
				host: strng::new("example.openai.azure.com"),
				api_version: None,
			}),
		),
		(
			"vertex",
			AIProvider::Vertex(vertex::Provider {
				model: Some(strng::new("gemini-2.5-pro")),
				region: Some(strng::new("us-central1")),
				project_id: strng::new("test-project-123"),
			}),
		),
	]
}

#[tokio::test]
async fn test_bedrock_embeddings() {
	let titan_provider = bedrock::Provider {
		model: Some(strng::new("amazon.titan-embed-text-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let cohere_provider = bedrock::Provider {
		model: Some(strng::new("cohere.embed-english-v3")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let titan_request = |i| conversion::bedrock::from_embeddings::translate(&i, &titan_provider);
	let cohere_request = |i| conversion::bedrock::from_embeddings::translate(&i, &cohere_provider);

	test_request(
		"bedrock-embeddings-titan",
		"request_embeddings_basic",
		titan_request,
	);
	for r in EMBEDDINGS_REQUESTS {
		test_request("bedrock-embeddings-cohere", r, cohere_request);
	}

	let titan_response = |i: Bytes| {
		conversion::bedrock::from_embeddings::translate_response(
			&i,
			&http::HeaderMap::new(),
			"amazon.titan-embed-text-v2:0",
		)
	};
	let cohere_response = |i: Bytes| {
		conversion::bedrock::from_embeddings::translate_response(
			&i,
			&http::HeaderMap::new(),
			"cohere.embed-english-v3",
		)
	};

	test_response(
		"bedrock-embeddings-titan",
		"response_bedrock_titan_embeddings",
		titan_response,
	);
	test_response(
		"bedrock-embeddings-cohere",
		"response_bedrock_cohere_embeddings",
		cohere_response,
	);
}

#[tokio::test]
async fn test_vertex_embeddings() {
	let provider = vertex::Provider {
		model: Some(strng::new("text-embedding-004")),
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project-123"),
	};

	let request = |i: types::embeddings::Request| i.to_vertex(&provider);
	for r in EMBEDDINGS_REQUESTS {
		test_request("vertex-embeddings", r, request);
	}

	let response =
		|i: Bytes| conversion::vertex::from_embeddings::translate_response(&i, "text-embedding-004");
	test_response("vertex-embeddings", "response_vertex_embeddings", response);
}

#[tokio::test]
async fn test_bedrock_completions() {
	let provider = bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let response =
		|i| conversion::bedrock::from_completions::translate_response(&i, &strng::new("fake-model"));
	test_response("bedrock-completions", "response_bedrock_basic", response);
	test_response("bedrock-completions", "response_bedrock_tool", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_completions::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming(
		"bedrock-completions",
		STREAM_FIXTURE_BEDROCK_BASIC,
		stream_response,
	)
	.await;

	let request = |i| conversion::bedrock::from_completions::translate(&i, &provider, None, None);
	for r in COMPLETION_REQUESTS {
		test_request("bedrock-completions", r, request);
	}
}

#[tokio::test]
async fn test_bedrock_messages() {
	let provider = bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let response =
		|i| conversion::bedrock::from_messages::translate_response(&i, &strng::new("fake-model"));
	test_response("bedrock-messages", "response_bedrock_basic", response);
	test_response("bedrock-messages", "response_bedrock_tool", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_messages::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming(
		"bedrock-messages",
		STREAM_FIXTURE_BEDROCK_BASIC,
		stream_response,
	)
	.await;

	let request = |i| conversion::bedrock::from_messages::translate(&i, &provider, None);
	for r in MESSAGES_REQUESTS {
		test_request("bedrock-messages", r, request);
	}
}

#[tokio::test]
async fn test_bedrock_responses() {
	let provider = bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};

	let response =
		|i| conversion::bedrock::from_responses::translate_response(&i, &strng::new("fake-model"));
	test_response("bedrock-response", "response_bedrock_basic", response);
	test_response("bedrock-response", "response_bedrock_tool", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_responses::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming(
		"bedrock-response",
		STREAM_FIXTURE_BEDROCK_BASIC,
		stream_response,
	)
	.await;

	let request = |i| conversion::bedrock::from_responses::translate(&i, &provider, None, None);
	for r in RESPONSES_REQUESTS {
		test_request("bedrock-response", r, request);
	}
}

#[tokio::test]
async fn test_vertex_messages() {
	let provider = vertex::Provider {
		model: Some(strng::new("anthropic/claude-sonnet-4-5")),
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project-123"),
	};

	let response = |bytes: Bytes| -> Result<Box<dyn ResponseType>, AIError> {
		Ok(Box::new(
			serde_json::from_slice::<types::messages::Response>(&bytes)
				.map_err(AIError::ResponseParsing)?,
		))
	};
	test_response("vertex-messages", "response_anthropic_basic", response);
	test_response("vertex-messages", "response_anthropic_tool", response);

	let stream_response = |body, log| Ok(conversion::messages::passthrough_stream(body, 1024, log));
	test_streaming(
		"vertex-messages",
		STREAM_FIXTURE_ANTHROPIC_BASIC,
		stream_response,
	)
	.await;

	let request = |input: types::messages::Request| -> Result<Vec<u8>, AIError> {
		let anthropic_body = serde_json::to_vec(&input).map_err(AIError::RequestMarshal)?;
		provider.prepare_anthropic_message_body(anthropic_body)
	};

	for r in MESSAGES_REQUESTS {
		test_request("vertex-messages", r, request);
	}
}

#[tokio::test]
async fn test_passthrough() {
	let test_dir = Path::new("src/llm/tests");

	let test_name = "request_full";
	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse input json");
	let openai: types::completions::Request =
		serde_json::from_str(openai_str).expect("Failed to parse input JSON");
	let t = serde_json::to_string_pretty(&openai).unwrap();
	let t2 = serde_json::to_string_pretty(&openai_raw).unwrap();
	assert_eq!(
		serde_json::from_str::<Value>(&t).unwrap(),
		serde_json::from_str::<Value>(&t2).unwrap(),
		"{t}\n{t2}"
	);
}

#[tokio::test]
async fn test_messages_to_completions() {
	let request = |i| conversion::completions::from_messages::translate(&i);
	for r in MESSAGES_REQUESTS {
		test_request("anthropic", r, request);
	}
}

#[test]
fn test_adaptive_thinking_without_effort_maps_to_high_reasoning_effort() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"max_tokens": 256,
		"thinking": {
			"type": "adaptive"
		},
		"messages": [
			{
				"role": "user",
				"content": "Give one concise insight."
			}
		]
	}))
	.expect("valid messages request");

	let translated = conversion::completions::from_messages::translate(&request)
		.expect("messages->completions translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(translated.get("reasoning_effort"), Some(&json!("high")));
}

#[test]
fn test_completions_reasoning_effort_maps_to_enabled_thinking_budget() {
	let request: types::completions::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"messages": [
			{ "role": "user", "content": "Give one concise insight." }
		],
		"reasoning_effort": "minimal"
	}))
	.expect("valid completions request");

	let translated = conversion::messages::from_completions::translate(&request)
		.expect("completions->messages translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(
		translated["thinking"],
		json!({
			"type": "enabled",
			"budget_tokens": 1024
		})
	);
	assert!(translated.get("output_config").is_none());
}

#[test]
fn test_completions_json_schema_response_format_maps_to_anthropic_output_config() {
	let request: types::completions::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"messages": [
			{ "role": "user", "content": "Return one short summary." }
		],
		"response_format": {
			"type": "json_schema",
			"json_schema": {
				"name": "summary_schema",
				"schema": {
					"type": "object",
					"properties": { "summary": { "type": "string" } },
					"required": ["summary"],
					"additionalProperties": false
				}
			}
		}
	}))
	.expect("valid completions request");

	let translated = conversion::messages::from_completions::translate(&request)
		.expect("completions->messages translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(
		translated["output_config"]["format"],
		json!({
			"type": "json_schema",
			"schema": {
				"type": "object",
				"properties": { "summary": { "type": "string" } },
				"required": ["summary"],
				"additionalProperties": false
			}
		})
	);
}

#[test]
fn test_messages_output_config_format_maps_to_openai_response_format() {
	let request: types::messages::Request = serde_json::from_value(json!({
		"model": "claude-opus-4-6",
		"max_tokens": 256,
		"output_config": {
			"format": {
				"type": "json_schema",
				"schema": {
					"type": "object",
					"properties": { "answer": { "type": "number" } },
					"required": ["answer"],
					"additionalProperties": false
				}
			}
		},
		"messages": [
			{
				"role": "user",
				"content": "What is 2+2?"
			}
		]
	}))
	.expect("valid messages request");

	let translated = conversion::completions::from_messages::translate(&request)
		.expect("messages->completions translation");
	let translated: Value =
		serde_json::from_slice(&translated).expect("translated request should be valid json");

	assert_eq!(translated["response_format"]["type"], json!("json_schema"));
	assert_eq!(
		translated["response_format"]["json_schema"]["name"],
		json!("structured_output")
	);
	assert_eq!(
		translated["response_format"]["json_schema"]["schema"],
		json!({
			"type": "object",
			"properties": { "answer": { "type": "number" } },
			"required": ["answer"],
			"additionalProperties": false
		})
	);
}

#[test]
fn test_messages_to_completions_stream_sets_include_usage_stream_option() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"stream": true,
		"messages": [
			{"role": "user", "content": "hello"}
		]
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	assert_eq!(value["stream"], serde_json::json!(true));
	assert_eq!(
		value["stream_options"]["include_usage"],
		serde_json::json!(true)
	);
}

#[test]
fn test_messages_to_completions_non_stream_omits_stream_options() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"stream": false,
		"messages": [
			{"role": "user", "content": "hello"}
		]
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	assert_eq!(value["stream"], serde_json::json!(false));
	assert!(
		value.get("stream_options").is_none(),
		"non-stream request should not include stream_options: {value}"
	);
}

#[test]
fn test_messages_to_completions_drops_assistant_thinking_blocks_but_keeps_text_and_tool_use() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"messages": [
			{"role": "user", "content": "hello"},
			{
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": "internal chain of thought", "signature": "sig_123"},
					{"type": "redacted_thinking", "data": "opaque"},
					{"type": "text", "text": "final answer"},
					{"type": "tool_use", "id": "call_1", "name": "lookup", "input": {"q": "x"}}
				]
			}
		]
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	let msgs = value["messages"]
		.as_array()
		.expect("translated messages should be an array");
	let assistant = msgs
		.iter()
		.find(|m| m["role"] == "assistant")
		.expect("assistant message should be present");

	assert_eq!(assistant["content"], serde_json::json!("final answer"));
	assert_eq!(
		assistant["tool_calls"][0]["id"],
		serde_json::json!("call_1")
	);
	assert_eq!(
		assistant["tool_calls"][0]["function"]["name"],
		serde_json::json!("lookup")
	);
	assert!(
		!assistant.to_string().contains("redacted_thinking")
			&& !assistant.to_string().contains("thinking"),
		"assistant message should not include thinking blocks: {assistant}"
	);
}

#[test]
fn test_messages_to_completions_preserves_parallel_tool_calls_preference() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "hello"}],
		"tool_choice": {
			"type": "auto",
			"disable_parallel_tool_use": true
		}
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: serde_json::Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	assert_eq!(value["parallel_tool_calls"], serde_json::json!(false));

	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"messages": [{"role": "user", "content": "hello"}],
		"tool_choice": {
			"type": "any",
			"disable_parallel_tool_use": false
		}
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: serde_json::Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	assert_eq!(value["parallel_tool_calls"], serde_json::json!(true));
}

#[test]
fn test_messages_to_completions_omits_assistant_message_when_only_thinking_blocks() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 64,
		"messages": [
			{"role": "user", "content": "hello"},
			{
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": "internal chain of thought", "signature": "sig_123"},
					{"type": "redacted_thinking", "data": "opaque"}
				]
			}
		]
	}))
	.expect("valid messages request");

	let out = conversion::completions::from_messages::translate(&req)
		.expect("messages -> completions translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated completions request should parse");

	let msgs = value["messages"]
		.as_array()
		.expect("translated messages should be an array");
	assert_eq!(
		msgs.len(),
		1,
		"assistant-only thinking blocks should be dropped"
	);
	assert_eq!(msgs[0]["role"], serde_json::json!("user"));
	assert_eq!(
		msgs[0]["content"],
		serde_json::json!([{"type": "text", "text": "hello"}])
	);
}

#[test]
fn test_completions_to_messages_synthesizes_auto_tool_choice_for_parallel_preference() {
	let req: types::completions::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"parallel_tool_calls": false,
		"messages": [{"role": "user", "content": "hello"}],
		"tools": [{
			"type": "function",
			"function": {
				"name": "get_weather",
				"description": "Get weather",
				"parameters": {
					"type": "object",
					"properties": {"location": {"type": "string"}},
					"required": ["location"]
				}
			}
		}]
	}))
	.expect("valid completions request");

	let out = conversion::messages::from_completions::translate(&req)
		.expect("completions -> messages translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated messages request should parse");

	assert_eq!(
		value["tool_choice"],
		serde_json::json!({
			"type": "auto",
			"disable_parallel_tool_use": true
		})
	);
}

#[test]
fn test_completions_to_messages_does_not_synthesize_tool_choice_without_tools() {
	let req: types::completions::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"parallel_tool_calls": false,
		"messages": [{"role": "user", "content": "hello"}]
	}))
	.expect("valid completions request");

	let out = conversion::messages::from_completions::translate(&req)
		.expect("completions -> messages translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated messages request should parse");

	assert!(
		value.get("tool_choice").is_none(),
		"tool_choice should be omitted when no tools are present: {value}"
	);
}

#[test]
fn test_completions_to_messages_preserves_explicit_tool_choice_and_parallel_preference() {
	let req: types::completions::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"parallel_tool_calls": false,
		"tool_choice": "required",
		"messages": [{"role": "user", "content": "hello"}],
		"tools": [{
			"type": "function",
			"function": {
				"name": "get_weather",
				"description": "Get weather",
				"parameters": {
					"type": "object",
					"properties": {"location": {"type": "string"}},
					"required": ["location"]
				}
			}
		}]
	}))
	.expect("valid completions request");

	let out = conversion::messages::from_completions::translate(&req)
		.expect("completions -> messages translation should succeed");
	let value: Value =
		serde_json::from_slice(&out).expect("translated messages request should parse");

	assert_eq!(
		value["tool_choice"],
		serde_json::json!({
			"type": "any",
			"disable_parallel_tool_use": true
		})
	);
}

#[tokio::test]
async fn test_completions_to_messages() {
	let response = |i| conversion::messages::from_completions::translate_response(&i);
	test_response("anthropic", "response_anthropic_basic", response);
	test_response("anthropic", "response_anthropic_tool", response);
	test_response("anthropic", "response_anthropic_thinking", response);

	let stream_response = |i, log| {
		Ok(conversion::messages::from_completions::translate_stream(
			i, 1024, log,
		))
	};
	test_streaming("anthropic", STREAM_FIXTURE_ANTHROPIC_BASIC, stream_response).await;
	test_streaming(
		"anthropic",
		STREAM_FIXTURE_ANTHROPIC_THINKING,
		stream_response,
	)
	.await;
	test_streaming("anthropic", STREAM_FIXTURE_ANTHROPIC_TOOL, stream_response).await;

	let request = |i| conversion::messages::from_completions::translate(&i);
	for r in COMPLETION_REQUESTS {
		test_request("anthropic", r, request);
	}
	for r in ANTHROPIC_COMPLETION_REQUESTS {
		test_request("anthropic", r, request);
	}
}

#[test]
fn test_completions_from_messages_rejects_empty_choices() {
	let response = serde_json::json!({
		"id": "chatcmpl-empty",
		"object": "chat.completion",
		"created": 123,
		"model": "gpt-4.1",
		"choices": [],
		"usage": null,
		"service_tier": null,
		"system_fingerprint": null
	});
	let bytes = Bytes::from(response.to_string());

	let out = conversion::completions::from_messages::translate_response(&bytes);
	assert!(matches!(out, Err(AIError::InvalidResponse(_))));
}

#[test]
fn test_completions_from_messages_preserves_multiple_assistant_text_blocks() {
	let req: types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"max_tokens": 32,
		"messages": [
			{
				"role": "assistant",
				"content": [
					{"type": "text", "text": "first"},
					{"type": "text", "text": "second"}
				]
			}
		]
	}))
	.expect("valid messages request");

	let out =
		conversion::completions::from_messages::translate(&req).expect("translation should work");
	let value: serde_json::Value = serde_json::from_slice(&out).expect("valid translated request");
	assert_eq!(value["messages"][0]["content"], "first\nsecond");
}

#[tokio::test]
async fn test_completions_from_messages_stream_done_without_finish_reason_emits_message_stop() {
	let input = concat!(
		"data: {\"id\":\"cmpl-2\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-2\",\"choices\":[],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1,\"total_tokens\":6}}\n\n",
		"data: [DONE]\n\n"
	);
	let out = conversion::completions::from_messages::translate_stream(
		Body::from(input.as_bytes().to_vec()),
		1024 * 1024,
		AsyncLog::default(),
	)
	.collect()
	.await
	.expect("stream should be readable")
	.to_bytes();
	let out = std::str::from_utf8(&out).expect("stream is utf-8");

	assert!(
		out.contains("\"type\":\"message_stop\""),
		"stream missing message_stop:\n{out}"
	);
	assert!(
		out.contains("\"stop_reason\":\"end_turn\""),
		"stream missing end_turn stop_reason:\n{out}"
	);
}

#[tokio::test]
async fn test_completions_from_messages_stream_interleaved_tool_calls_single_block_start_per_index()
{
	let input = concat!(
		"data: {\"id\":\"cmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"foo\",\"arguments\":\"{\\\"a\\\":\"}}]}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bar\",\"arguments\":\"{\\\"b\\\":\"}}]}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"type\":\"function\",\"function\":{\"arguments\":\"1}\"}}]}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8}}\n\n",
		"data: [DONE]\n\n"
	);

	let out = conversion::completions::from_messages::translate_stream(
		Body::from(input.as_bytes().to_vec()),
		1024 * 1024,
		AsyncLog::default(),
	)
	.collect()
	.await
	.expect("stream should be readable")
	.to_bytes();
	let out = std::str::from_utf8(&out).expect("stream is utf-8");

	let idx0 = out
		.matches("\"type\":\"content_block_start\",\"index\":0")
		.count();
	let idx1 = out
		.matches("\"type\":\"content_block_start\",\"index\":1")
		.count();
	assert_eq!(idx0, 1, "tool index 0 block started multiple times:\n{out}");
	assert_eq!(idx1, 1, "tool index 1 block started multiple times:\n{out}");
}

#[tokio::test]
async fn test_completions_from_messages_stream_waits_for_tool_name_before_open() {
	let input = concat!(
		"data: {\"id\":\"cmpl-3\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"arguments\":\"{\\\"a\\\":\"}}]}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-3\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"type\":\"function\",\"function\":{\"name\":\"foo\",\"arguments\":\"1}\"}}]}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-3\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8}}\n\n",
		"data: [DONE]\n\n"
	);

	let out = conversion::completions::from_messages::translate_stream(
		Body::from(input.as_bytes().to_vec()),
		1024 * 1024,
		AsyncLog::default(),
	)
	.collect()
	.await
	.expect("stream should be readable")
	.to_bytes();
	let out = std::str::from_utf8(&out).expect("stream is utf-8");

	assert!(
		!out.contains("\"name\":\"\""),
		"tool block should not be opened with empty name:\n{out}"
	);
	assert!(
		out.contains("\"name\":\"foo\""),
		"tool block should include resolved tool name:\n{out}"
	);
}

#[tokio::test]
async fn test_completions_from_messages_stream_text_then_tool_reindexes_tool_calls() {
	let input = concat!(
		"event: message_start\n",
		"data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
		"event: content_block_start\n",
		"data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"Thinking...\"}}\n\n",
		"event: content_block_delta\n",
		"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Thinking...\"}}\n\n",
		"event: content_block_stop\n",
		"data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
		"event: content_block_start\n",
		"data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"tool\",\"input\":{}}}\n\n",
		"event: content_block_delta\n",
		"data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
		"event: message_delta\n",
		"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}\n\n",
		"event: message_stop\n",
		"data: {\"type\":\"message_stop\"}\n\n",
		"data: [DONE]\n\n"
	);

	let out = conversion::messages::from_completions::translate_stream(
		Body::from(input.as_bytes().to_vec()),
		1024 * 1024,
		AmendOnDrop::new(AsyncLog::default(), LLMResponsePolicies::default()),
	)
	.collect()
	.await
	.expect("stream should be readable")
	.to_bytes();
	let out = std::str::from_utf8(&out).expect("stream is utf-8");

	// Check that we have tool_calls with index 0
	assert!(
		out.contains(r#""tool_calls":[{"index":0"#),
		"tool call should be re-indexed to 0:\n{out}"
	);
	// Check that we DO NOT have tool_calls with index 1
	assert!(
		!out.contains(r#""tool_calls":[{"index":1"#),
		"tool call should not use raw index 1:\n{out}"
	);
}

#[test]
fn test_process_success_messages_uses_completions_translation_for_provider_adapters() {
	let test_dir = Path::new("src/llm/tests");
	let input_path = test_dir.join("response_basic.json");
	let body = Bytes::from(fs::read(input_path).expect("Failed to read provider response fixture"));

	let expected = conversion::completions::from_messages::translate_response(&body)
		.expect("expected translation should succeed")
		.serialize()
		.expect("expected translation should serialize");
	let expected: Value =
		serde_json::from_slice(&expected).expect("expected response should be JSON");

	for (name, provider) in provider_message_adapters() {
		let req = llm_request(InputFormat::Messages, name, "gpt-4.1", false);
		let got = provider
			.process_success(&req, &body)
			.unwrap_or_else(|e| panic!("{name}: process_success failed: {e}"))
			.serialize()
			.unwrap_or_else(|e| panic!("{name}: failed to serialize translated response: {e}"));
		let got: Value = serde_json::from_slice(&got)
			.unwrap_or_else(|e| panic!("{name}: invalid translated JSON: {e}"));
		assert_eq!(got, expected, "{name}: translated response mismatch");
	}
}

#[tokio::test]
async fn test_process_streaming_messages_uses_completions_translation_for_provider_adapters() {
	let stream = concat!(
		"data: {\"id\":\"cmpl-routing\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":null}\n\n",
		"data: {\"id\":\"cmpl-routing\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"created\":123,\"model\":\"gpt-4.1\",\"service_tier\":null,\"system_fingerprint\":null,\"object\":\"chat.completion.chunk\",\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1,\"total_tokens\":6}}\n\n",
		"data: [DONE]\n\n"
	);
	let stream_bytes = stream.as_bytes().to_vec();
	let expected = conversion::completions::from_messages::translate_stream(
		Body::from(stream_bytes.clone()),
		1024 * 1024,
		AsyncLog::default(),
	)
	.collect()
	.await
	.expect("expected translated stream should be readable")
	.to_bytes();

	for (name, provider) in provider_message_adapters() {
		let req = llm_request(InputFormat::Messages, name, "gpt-4.1", true);
		let resp = ::http::Response::builder()
			.status(::http::StatusCode::OK)
			.header(::http::header::CONTENT_TYPE, "text/event-stream")
			.body(Body::from(stream_bytes.clone()))
			.expect("failed to build provider stream response");

		let out = provider
			.process_streaming(
				req,
				crate::store::LLMResponsePolicies::default(),
				AsyncLog::default(),
				false,
				resp,
			)
			.await
			.unwrap_or_else(|e| panic!("{name}: process_streaming failed: {e}"))
			.into_body()
			.collect()
			.await
			.unwrap_or_else(|e| panic!("{name}: translated stream read failed: {e}"))
			.to_bytes();

		assert_eq!(out, expected, "{name}: streaming translation mismatch");
	}
}

#[tokio::test]
async fn test_process_streaming_vertex_anthropic_completions_uses_messages_translation() {
	let test_dir = Path::new("src/llm/tests");
	let stream_path = test_dir.join("response_stream-anthropic_basic.json");
	let stream_bytes = fs::read(stream_path).expect("Failed to read anthropic stream fixture");
	let expected = conversion::messages::from_completions::translate_stream(
		Body::from(stream_bytes.clone()),
		1024 * 1024,
		AmendOnDrop::new(AsyncLog::default(), LLMResponsePolicies::default()),
	)
	.collect()
	.await
	.expect("expected translated stream should be readable")
	.to_bytes();

	let provider = AIProvider::Vertex(vertex::Provider {
		model: None,
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project-123"),
	});
	let req = llm_request(
		InputFormat::Completions,
		"vertex",
		"anthropic/claude-sonnet-4-5-20251001",
		true,
	);
	let resp = ::http::Response::builder()
		.status(::http::StatusCode::OK)
		.header(::http::header::CONTENT_TYPE, "text/event-stream")
		.body(Body::from(stream_bytes))
		.expect("failed to build provider stream response");

	let got = provider
		.process_streaming(
			req,
			crate::store::LLMResponsePolicies::default(),
			AsyncLog::default(),
			false,
			resp,
		)
		.await
		.expect("vertex anthropic completions streaming should be translated")
		.into_body()
		.collect()
		.await
		.expect("translated stream should be readable")
		.to_bytes();

	assert_eq!(
		got, expected,
		"vertex anthropic completions streaming translation mismatch"
	);
}

#[test]
fn test_roundtrip_completions_messages_preserves_tool_call_and_tool_result_semantics() {
	let source: types::completions::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4.1",
		"messages": [
			{"role": "system", "content": "system instruction"},
			{"role": "developer", "content": "developer instruction"},
			{"role": "user", "content": "please call the tool"},
			{
				"role": "assistant",
				"content": "calling tool",
				"tool_calls": [
					{
						"id": "call_123",
						"type": "function",
						"function": {
							"name": "lookup_weather",
							"arguments": "{\"city\":\"Paris\"}"
						}
					}
				]
			},
			{"role": "tool", "tool_call_id": "call_123", "content": "sunny"}
		],
		"max_tokens": 64
	}))
	.expect("valid source completion request");

	let messages = conversion::messages::from_completions::translate(&source)
		.expect("completions -> messages should succeed");
	let roundtrip = conversion::completions::from_messages::translate(
		&serde_json::from_slice::<types::messages::Request>(&messages)
			.expect("translated messages request should deserialize"),
	)
	.expect("messages -> completions should succeed");
	let roundtrip: Value =
		serde_json::from_slice(&roundtrip).expect("roundtrip completions should deserialize");

	let msgs = roundtrip["messages"]
		.as_array()
		.expect("roundtrip messages should be an array");
	assert!(
		msgs.iter().any(|m| {
			m["role"] == "assistant"
				&& m["tool_calls"].is_array()
				&& m["tool_calls"][0]["id"] == "call_123"
				&& m["tool_calls"][0]["function"]["name"] == "lookup_weather"
		}),
		"assistant tool call not preserved in roundtrip: {roundtrip}"
	);
	assert!(
		msgs.iter().any(|m| {
			m["role"] == "tool" && m["tool_call_id"] == "call_123" && m["content"] == "sunny"
		}),
		"tool result not preserved in roundtrip: {roundtrip}"
	);
	assert!(
		msgs
			.iter()
			.any(|m| m["role"] == "system" && m["content"] == "system instruction\ndeveloper instruction"),
		"system+developer prompt merge missing in roundtrip: {roundtrip}"
	);
}

fn apply_test_prompts<R: RequestType + Serialize>(mut r: R) -> Result<Vec<u8>, AIError> {
	r.prepend_prompts(vec![
		SimpleChatCompletionMessage {
			role: strng::new("system"),
			content: strng::new("prepend system prompt"),
		},
		SimpleChatCompletionMessage {
			role: strng::new("user"),
			content: strng::new("prepend user message"),
		},
		SimpleChatCompletionMessage {
			role: strng::new("assistant"),
			content: strng::new("prepend assistant message"),
		},
	]);
	r.append_prompts(vec![
		SimpleChatCompletionMessage {
			role: strng::new("user"),
			content: strng::new("append user message"),
		},
		SimpleChatCompletionMessage {
			role: strng::new("system"),
			content: strng::new("append system prompt"),
		},
		SimpleChatCompletionMessage {
			role: strng::new("assistant"),
			content: strng::new("append assistant prompt"),
		},
	]);
	serde_json::to_vec(&r).map_err(AIError::RequestMarshal)
}

#[test]
fn test_prompt_enrichment() {
	test_request::<types::messages::Request>(
		"anthropic",
		"request_anthropic_with_system",
		apply_test_prompts,
	);
	test_request::<types::responses::Request>(
		"openai",
		"request_openai_with_inputs",
		apply_test_prompts,
	);
	test_request::<types::completions::Request>(
		"openai",
		"request_openai_with_messages",
		apply_test_prompts,
	);
}

#[tokio::test]
async fn test_anthropic_count_tokens() {
	let request = |i: types::count_tokens::Request| i.to_anthropic();
	for r in COUNT_TOKENS_REQUESTS {
		test_request("anthropic-count-tokens", r, request);
	}

	// test count_tokens response
	let test_dir = Path::new("src/llm/tests");
	let input_path = test_dir.join("response_anthropic_count_tokens.json");
	let response_str = &fs::read_to_string(&input_path).expect("Failed to read response file");
	let bytes = Bytes::copy_from_slice(response_str.as_bytes());
	let provider_value = serde_json::from_str::<Value>(response_str).unwrap();

	let (returned_bytes, count) = types::count_tokens::Response::translate_response(bytes.clone())
		.expect("Failed to translate count_tokens response");

	assert_eq!(
		returned_bytes, bytes,
		"Response bytes should be returned unchanged"
	);

	let resp: types::count_tokens::Response =
		serde_json::from_slice(&returned_bytes).expect("Failed to deserialize response");

	insta::with_settings!({
			info => &provider_value,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
	}, {
			 insta::assert_json_snapshot!("anthropic_response_count_tokens.json", serde_json::json!({
				"input_tokens": resp.input_tokens,
				"token_count": count,
			}));
	});
}

#[tokio::test]
async fn test_bedrock_count_tokens() {
	let mut headers = http::HeaderMap::new();
	headers.insert("anthropic-version", "2023-06-01".parse().unwrap());

	let request = |input: types::count_tokens::Request| input.to_bedrock_token_count(&headers);

	for r in COUNT_TOKENS_REQUESTS {
		test_request("bedrock-count-tokens", r, request);
	}
}

#[tokio::test]
async fn test_vertex_count_tokens() {
	let provider = vertex::Provider {
		model: Some(strng::new("anthropic/claude-sonnet-4-5")),
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project-123"),
	};

	let request = |input: types::count_tokens::Request| -> Result<Vec<u8>, AIError> {
		let anthropic_body = input.to_anthropic()?;
		provider.prepare_anthropic_count_tokens_body(anthropic_body)
	};

	for r in COUNT_TOKENS_REQUESTS {
		test_request("vertex-count-tokens", r, request);
	}
}

#[test]
fn test_get_messages() {
	use crate::llm::types::RequestType;

	let test_dir = Path::new("src/llm/tests");
	let input_path = test_dir.join("request_full.json");
	let input_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let input_raw: Value = serde_json::from_str(input_str).expect("Failed to parse input json");

	fn extract_messages<R: RequestType + DeserializeOwned>(
		input: &str,
		name: &str,
		raw: &Value,
		path: &Path,
	) {
		let request: R = serde_json::from_str(input).expect("Failed to parse json");

		let out: Vec<Value> = request
			.get_messages()
			.iter()
			.map(|m| {
				serde_json::json!({
					"role": m.role.as_str(),
					"content": m.content.as_str(),
				})
			})
			.collect();

		insta::with_settings!({
			info => raw,
			description => path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
		}, {
			insta::assert_json_snapshot!(name, out);
		});
	}

	extract_messages::<types::completions::Request>(
		input_str,
		"completions_get_messages",
		&input_raw,
		&input_path,
	);

	extract_messages::<types::messages::Request>(
		input_str,
		"messages_get_messages",
		&input_raw,
		&input_path,
	);
}

#[test]
fn test_messages_set_messages_roundtrip_preserves_system_field() {
	use crate::llm::types::RequestType;
	use crate::llm::types::SimpleChatCompletionMessage;

	let mut req = types::messages::Request {
		model: Some("claude-haiku-4-5-20251001".to_string()),
		system: Some(types::messages::TextBlock::Text(
			"original system".to_string(),
		)),
		messages: vec![types::messages::RequestMessage {
			role: "user".to_string(),
			content: Some(types::messages::ContentBlock::Text("hello".to_string())),
			rest: Default::default(),
		}],
		..Default::default()
	};

	let mut msgs = req.get_messages();
	msgs[0] = SimpleChatCompletionMessage {
		role: strng::literal!("system"),
		content: strng::literal!("masked system"),
	};
	req.set_messages(msgs);

	match req.system {
		Some(types::messages::TextBlock::Array(ref parts)) => {
			assert_eq!(parts.len(), 1, "expected one system prompt after roundtrip");
			match &parts[0] {
				types::messages::TextPart::Text { text, .. } => {
					assert_eq!(text, "masked system");
				},
				other => panic!("unexpected system prompt block: {other:?}"),
			}
		},
		other => panic!("expected system prompts in array form, got: {other:?}"),
	}

	assert!(
		req.messages.iter().all(|m| m.role != "system"),
		"system messages must not be stored inside messages[]"
	);
}
