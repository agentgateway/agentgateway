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
		"response_stream-bedrock_basic.bin",
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
		"response_stream-bedrock_basic.bin",
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
		"response_stream-bedrock_basic.bin",
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
		"response_stream-anthropic_basic.json",
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
	test_streaming(
		"anthropic",
		"response_stream-anthropic_basic.json",
		stream_response,
	)
	.await;
	test_streaming(
		"anthropic",
		"response_stream-anthropic_thinking.json",
		stream_response,
	)
	.await;

	let request = |i| conversion::messages::from_completions::translate(&i);
	for r in COMPLETION_REQUESTS {
		test_request("anthropic", r, request);
	}
	for r in ANTHROPIC_COMPLETION_REQUESTS {
		test_request("anthropic", r, request);
	}
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
