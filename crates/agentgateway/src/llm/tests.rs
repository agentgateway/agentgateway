use std::fs;
use std::path::{Path, PathBuf};

use agent_core::strng;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;

fn test_root() -> &'static Path {
	Path::new("src/llm/tests")
}

fn fixture_path(relative_path: &str) -> PathBuf {
	test_root().join(relative_path)
}

fn snapshot_path_and_name(relative_path: &str, provider: &str) -> (String, String) {
	let rel = Path::new(relative_path);
	let parent = rel.parent().unwrap_or_else(|| Path::new(""));
	let stem = rel
		.file_stem()
		.unwrap_or_else(|| panic!("{relative_path}: missing filename"))
		.to_string_lossy();
	(
		format!("tests/{}", parent.display()),
		format!("{stem}.{provider}"),
	)
}

fn test_response(
	provider: &str,
	relative_path: &str,
	xlate: impl Fn(Bytes) -> Result<Box<dyn ResponseType>, AIError>,
) {
	let input_path = fixture_path(relative_path);
	let provider_str = &fs::read_to_string(&input_path)
		.unwrap_or_else(|_| panic!("{relative_path}: Failed to read input file"));
	let provider_value = serde_json::from_str::<Value>(provider_str).unwrap();

	let resp = xlate(Bytes::copy_from_slice(provider_str.as_bytes()))
		.expect("Failed to translate provider response to OpenAI format");
	let raw = resp
		.serialize()
		.expect("Failed to serialize OpenAI response");
	let resp_val = serde_json::from_slice::<Value>(&raw).expect("Failed to parse OpenAI response");
	let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, provider);

	insta::with_settings!({
			info => &provider_value,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
	}, {
			 insta::assert_json_snapshot!(snapshot_name, resp_val, {
			".id" => "[id]",
			".output.*.id" => "[id]",
			".created" => "[date]",
		});
	});
}

async fn test_streaming(
	provider: &str,
	relative_path: &str,
	xlate: impl Fn(Body, AmendOnDrop) -> Result<Body, AIError>,
) {
	let input_path = fixture_path(relative_path);
	let input_bytes =
		&fs::read(&input_path).unwrap_or_else(|_| panic!("{relative_path}: Failed to read input file"));
	let body = Body::from(input_bytes.clone());
	let log = AsyncLog::default();
	let resp = xlate(body, AmendOnDrop::new(log, LLMResponsePolicies::default()))
		.expect("failed to translate stream");
	let resp_bytes = resp.collect().await.unwrap().to_bytes();
	let resp_str = std::str::from_utf8(&resp_bytes).unwrap();
	let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, provider);

	insta::with_settings!({
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
			filters => vec![
				(r#""created":[0-9]+"#, r#""created":123"#),
				(r#""created_at":[0-9]+"#, r#""created_at":123"#),
				(r#""id":"(resp|msg|call)_[0-9a-f]+""#, r#""id":"$1_xxx""#),
				(r#""item_id":"(msg|call)_[0-9a-f]+""#, r#""item_id":"$1_xxx""#),
				(r#""call_id":"call_[0-9a-f]+""#, r#""call_id":"call_xxx""#),
			]
	}, {
			 insta::assert_snapshot!(snapshot_name, resp_str);
	});
}

fn test_request<I>(
	provider: &str,
	relative_path: &str,
	xlate: impl Fn(I) -> Result<Vec<u8>, AIError>,
) where
	I: DeserializeOwned,
{
	let input_path = fixture_path(relative_path);
	let input_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let input_raw: Value = serde_json::from_str(input_str).expect("Failed to parse input json");
	let input_typed: I = serde_json::from_str(input_str).expect("Failed to parse input JSON");

	let provider_response =
		xlate(input_typed).expect("Failed to translate input format to provider request ");
	let provider_value =
		serde_json::from_slice::<Value>(&provider_response).expect("Failed to parse provider response");
	let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, provider);

	insta::with_settings!({
			info => &input_raw,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
	}, {
			 insta::assert_json_snapshot!(snapshot_name, provider_value, {
			".id" => "[id]",
			".created" => "[date]",
		});
	});
}

const ANTHROPIC: &str = "anthropic";
const BEDROCK: &str = "bedrock";
const VERTEX: &str = "vertex";
const OPENAI: &str = "openai";
const BEDROCK_TITAN: &str = "bedrock-titan";
const BEDROCK_COHERE: &str = "bedrock-cohere";

const COMPLETION_REQUESTS: &[(&str, &[&str])] = &[
	("basic", &[ANTHROPIC, BEDROCK]),
	("full", &[ANTHROPIC, BEDROCK]),
	("tool-call", &[ANTHROPIC, BEDROCK]),
	("reasoning", &[ANTHROPIC, BEDROCK]),
    ("reasoning_max", &[ANTHROPIC]),
];
const MESSAGES_REQUESTS: &[(&str, &[&str])] = &[
	("basic", &[ANTHROPIC, BEDROCK, VERTEX]),
	("tools", &[ANTHROPIC, BEDROCK, VERTEX]),
	("reasoning", &[ANTHROPIC, BEDROCK, VERTEX]),
];
const RESPONSES_REQUESTS: &[(&str, &[&str])] =
	&[("basic", &[BEDROCK]), ("instructions", &[BEDROCK])];
const COUNT_TOKENS_REQUESTS: &[(&str, &[&str])] = &[
	("basic", &[ANTHROPIC, BEDROCK, VERTEX]),
	("with_system", &[ANTHROPIC, BEDROCK, VERTEX]),
];
const EMBEDDINGS_REQUESTS: &[(&str, &[&str])] = &[
	("basic", &[BEDROCK_TITAN, BEDROCK_COHERE, VERTEX]),
	("array", &[BEDROCK_COHERE, VERTEX]),
];

mod requests {
	use super::*;

	#[test]
	fn from_completions() {
		let bedrock_provider = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		let bedrock =
			|i| conversion::bedrock::from_completions::translate(&i, &bedrock_provider, None, None);
		let anthropic = |i| conversion::messages::from_completions::translate(&i);

		for (name, providers) in COMPLETION_REQUESTS {
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(
						BEDROCK,
						&format!("requests/completions/{name}.json"),
						bedrock,
					),
					ANTHROPIC => test_request(
						ANTHROPIC,
						&format!("requests/completions/{name}.json"),
						anthropic,
					),
					other => panic!("unsupported provider in COMPLETION_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn from_messages() {
		let bedrock_provider = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

      let bedrock_request= |i| conversion::bedrock::from_messages::translate(&i, &bedrock_provider, None);
		for (name, providers) in MESSAGES_REQUESTS {
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(
						BEDROCK,
						&format!("requests/completions/{name}.json"),
						bedrock,
					),
					ANTHROPIC => test_request(
						ANTHROPIC,
						&format!("requests/completions/{name}.json"),
						anthropic,
					),
					other => panic!("unsupported provider in COMPLETION_REQUESTS: {other}"),
				}
			}
		}
	}

	#[tokio::test]
	async fn from_embeddings() {
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

		let vertex_provider = vertex::Provider {
			model: Some(strng::new("text-embedding-004")),
			region: Some(strng::new("us-central1")),
			project_id: strng::new("test-project-123"),
		};

		let titan_request = |i| conversion::bedrock::from_embeddings::translate(&i, &titan_provider);
		let cohere_request = |i| conversion::bedrock::from_embeddings::translate(&i, &cohere_provider);
		let vertex_request = |i: types::embeddings::Request| i.to_vertex(&vertex_provider);
		for (name, providers) in EMBEDDINGS_REQUESTS {
			for provider in *providers {
				match *provider {
					BEDROCK_TITAN => {
						test_request(
							BEDROCK_TITAN,
							&format!("requests/embeddings/{name}.json"),
							titan_request,
						);
					},
					BEDROCK_COHERE => test_request(
						BEDROCK_COHERE,
						&format!("requests/embeddings/{name}.json"),
						cohere_request,
					),
					VERTEX => {
						test_request(
							VERTEX,
							&format!("requests/embeddings/{name}.json"),
							vertex_request,
						);
					},
					other => panic!("unsupported provider in EMBEDDINGS_REQUESTS: {other}"),
				}
			}
		}
	}
}

#[tokio::test]
async fn test_bedrock_completions() {
	let response =
		|i| conversion::bedrock::from_completions::translate_response(&i, &strng::new("fake-model"));
	test_response("completions", "response/bedrock/basic.json", response);
	test_response("completions", "response/bedrock/tool.json", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_completions::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming("completions", "response/bedrock/basic.bin", stream_response).await;
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
	test_response("messages", "response/bedrock/basic.json", response);
	test_response("messages", "response/bedrock/tool.json", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_messages::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming("messages", "response/bedrock/basic.bin", stream_response).await;

	let request = |i| conversion::bedrock::from_messages::translate(&i, &provider, None);
	for (name, providers) in MESSAGES_REQUESTS {
		if providers.contains(&BEDROCK) {
			test_request(BEDROCK, &format!("requests/messages/{name}.json"), request);
		}
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
	test_response("responses", "response/bedrock/basic.json", response);
	test_response("responses", "response/bedrock/tool.json", response);

	let stream_response = |i, log| {
		Ok(conversion::bedrock::from_responses::translate_stream(
			i,
			0,
			log,
			"model",
			"request-id",
		))
	};
	test_streaming("responses", "response/bedrock/basic.bin", stream_response).await;

	let request = |i| conversion::bedrock::from_responses::translate(&i, &provider, None, None);
	for (name, providers) in RESPONSES_REQUESTS {
		if providers.contains(&BEDROCK) {
			test_request(BEDROCK, &format!("requests/responses/{name}.json"), request);
		}
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
	test_response(VERTEX, "response/anthropic/basic.json", response);
	test_response(VERTEX, "response/anthropic/tool.json", response);

	let stream_response = |body, log| Ok(conversion::messages::passthrough_stream(body, 1024, log));
	test_streaming(
		VERTEX,
		"response/anthropic/stream_basic.json",
		stream_response,
	)
	.await;

	let request = |input: types::messages::Request| -> Result<Vec<u8>, AIError> {
		let anthropic_body = serde_json::to_vec(&input).map_err(AIError::RequestMarshal)?;
		provider.prepare_anthropic_message_body(anthropic_body)
	};

	for (name, providers) in MESSAGES_REQUESTS {
		if providers.contains(&VERTEX) {
			test_request(VERTEX, &format!("requests/messages/{name}.json"), request);
		}
	}
}

#[tokio::test]
async fn test_passthrough() {
	let input_path = fixture_path("requests/completions/full.json");
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
	for (name, providers) in MESSAGES_REQUESTS {
		if providers.contains(&ANTHROPIC) {
			test_request(
				ANTHROPIC,
				&format!("requests/messages/{name}.json"),
				request,
			);
		}
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
	test_response(ANTHROPIC, "response/anthropic/basic.json", response);
	test_response(ANTHROPIC, "response/anthropic/tool.json", response);
	test_response(ANTHROPIC, "response/anthropic/thinking.json", response);

	let stream_response = |i, log| {
		Ok(conversion::messages::from_completions::translate_stream(
			i, 1024, log,
		))
	};
	test_streaming(
		ANTHROPIC,
		"response/anthropic/stream_basic.json",
		stream_response,
	)
	.await;
	test_streaming(
		ANTHROPIC,
		"response/anthropic/stream_thinking.json",
		stream_response,
	)
	.await;
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
		ANTHROPIC,
		"requests/policies/anthropic_with_system.json",
		apply_test_prompts,
	);
	test_request::<types::responses::Request>(
		OPENAI,
		"requests/policies/openai_with_inputs.json",
		apply_test_prompts,
	);
	test_request::<types::completions::Request>(
		OPENAI,
		"requests/policies/openai_with_messages.json",
		apply_test_prompts,
	);
}

#[tokio::test]
async fn test_anthropic_count_tokens() {
	let request = |i: types::count_tokens::Request| i.to_anthropic();
	for (name, providers) in COUNT_TOKENS_REQUESTS {
		if providers.contains(&ANTHROPIC) {
			test_request(
				ANTHROPIC,
				&format!("requests/count-tokens/{name}.json"),
				request,
			);
		}
	}

	let input_path = fixture_path("response/anthropic/count_tokens.json");
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
	let (snapshot_path, snapshot_name) =
		snapshot_path_and_name("response/anthropic/count_tokens.json", ANTHROPIC);

	insta::with_settings!({
			info => &provider_value,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
	}, {
			 insta::assert_json_snapshot!(snapshot_name, serde_json::json!({
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

	for (name, providers) in COUNT_TOKENS_REQUESTS {
		if providers.contains(&BEDROCK) {
			test_request(
				BEDROCK,
				&format!("requests/count-tokens/{name}.json"),
				request,
			);
		}
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

	for (name, providers) in COUNT_TOKENS_REQUESTS {
		if providers.contains(&VERTEX) {
			test_request(
				VERTEX,
				&format!("requests/count-tokens/{name}.json"),
				request,
			);
		}
	}
}

#[test]
fn test_get_messages() {
	use crate::llm::types::RequestType;

	let input_path = fixture_path("requests/completions/full.json");
	let input_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let input_raw: Value = serde_json::from_str(input_str).expect("Failed to parse input json");

	fn extract_messages<R: RequestType + DeserializeOwned>(
		input: &str,
		path: &Path,
		raw: &Value,
		provider: &str,
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

		let (snapshot_path, snapshot_name) =
			snapshot_path_and_name("requests/completions/full.json", provider);
		insta::with_settings!({
			info => raw,
			description => path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
		}, {
			insta::assert_json_snapshot!(snapshot_name, out);
		});
	}

	extract_messages::<types::completions::Request>(
		input_str,
		&input_path,
		&input_raw,
		"get-messages-completions",
	);
	extract_messages::<types::messages::Request>(
		input_str,
		&input_path,
		&input_raw,
		"get-messages-messages",
	);
}
