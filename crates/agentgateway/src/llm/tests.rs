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
	let snapshot_name = snapshot_name + "-streaming";

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
const COMPLETIONS: &str = "completions";
const MESSAGES: &str = "messages";
const RESPONSES: &str = "responses";
const BEDROCK_TITAN: &str = "bedrock-titan";
const BEDROCK_COHERE: &str = "bedrock-cohere";

mod requests {
	use super::*;

	const COMPLETION_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK]),
		("full", &[ANTHROPIC, BEDROCK]),
		("tool-call", &[ANTHROPIC, BEDROCK]),
		("reasoning", &[ANTHROPIC, BEDROCK]),
		("reasoning_max", &[ANTHROPIC]),
	];
	const MESSAGES_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[COMPLETIONS, BEDROCK, VERTEX]),
		("tools", &[COMPLETIONS, BEDROCK, VERTEX]),
		("reasoning", &[COMPLETIONS, BEDROCK, VERTEX]),
	];
	const RESPONSES_REQUESTS: &[(&str, &[&str])] =
		&[("basic", &[BEDROCK]), ("instructions", &[BEDROCK])];
	pub const COUNT_TOKENS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK, VERTEX]),
		("with_system", &[ANTHROPIC, BEDROCK, VERTEX]),
	];
	const EMBEDDINGS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[BEDROCK_TITAN, BEDROCK_COHERE, VERTEX]),
		("array", &[BEDROCK_COHERE, VERTEX]),
	];

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
		let vertex_provider = vertex::Provider {
			model: Some(strng::new("anthropic/claude-sonnet-4-5")),
			region: Some(strng::new("us-central1")),
			project_id: strng::new("test-project-123"),
		};

		let bedrock_request =
			|i| conversion::bedrock::from_messages::translate(&i, &bedrock_provider, None);
		let vertex_request = |input: types::messages::Request| -> Result<Vec<u8>, AIError> {
			let anthropic_body = serde_json::to_vec(&input).map_err(AIError::RequestMarshal)?;
			vertex_provider.prepare_anthropic_message_body(anthropic_body)
		};
		let completions_request = |i| conversion::completions::from_messages::translate(&i);
		for (name, providers) in MESSAGES_REQUESTS {
			let test = &format!("requests/messages/{name}.json");
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(BEDROCK, test, bedrock_request),
					COMPLETIONS => test_request(COMPLETIONS, test, completions_request),
					VERTEX => test_request(VERTEX, test, vertex_request),
					other => panic!("unsupported provider in MESSAGES_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn from_responses() {
		let bedrock_provider = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		let bed_request =
			|i| conversion::bedrock::from_responses::translate(&i, &bedrock_provider, None, None);

		for (name, providers) in RESPONSES_REQUESTS {
			let test = &format!("requests/responses/{name}.json");
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(BEDROCK, test, bed_request),
					other => panic!("unsupported provider in RESPONSES_REQUESTS: {other}"),
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

	#[tokio::test]
	async fn from_count_tokens() {
		let mut headers = http::HeaderMap::new();
		headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
		let vertex_provider = vertex::Provider {
			model: Some(strng::new("anthropic/claude-sonnet-4-5")),
			region: Some(strng::new("us-central1")),
			project_id: strng::new("test-project-123"),
		};

		let bedrock_request =
			|input: types::count_tokens::Request| input.to_bedrock_token_count(&headers);
		let anthropic_request = |i: types::count_tokens::Request| i.to_anthropic();
		let vertex_request = |input: types::count_tokens::Request| -> Result<Vec<u8>, AIError> {
			let anthropic_body = input.to_anthropic()?;
			vertex_provider.prepare_anthropic_count_tokens_body(anthropic_body)
		};
		for (name, providers) in COUNT_TOKENS_REQUESTS {
			let test = &format!("requests/count-tokens/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => test_request(provider, test, anthropic_request),
					BEDROCK => test_request(provider, test, bedrock_request),
					VERTEX => test_request(provider, test, vertex_request),
					other => panic!("unsupported provider in COUNT_TOKENS_REQUESTS: {other}"),
				}
			}
		}
	}
}

mod response {
	use super::*;

	const BEDROCK_RESPONSES: &[(&str, &[&str])] = &[
		("basic", &[COMPLETIONS, MESSAGES, RESPONSES]),
		("tool", &[COMPLETIONS, MESSAGES, RESPONSES]),
	];
	const BEDROCK_STREAM_RESPONSES: &[(&str, &[&str])] =
		&[("basic", &[COMPLETIONS, MESSAGES, RESPONSES])];
	const ANTHROPIC_RESPONSES: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC]),
		("tool", &[ANTHROPIC]),
		("thinking", &[ANTHROPIC]),
	];
	const ANTHROPIC_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("stream_basic", &[ANTHROPIC, COMPLETIONS]),
		("stream_thinking", &[ANTHROPIC, COMPLETIONS]),
	];
	const VERTEX_RESPONSES: &[(&str, &[&str])] = &[("basic", &[VERTEX]), ("tool", &[VERTEX])];
	const VERTEX_STREAM_RESPONSES: &[(&str, &[&str])] = &[("stream_basic", &[VERTEX])];
	const EMBEDDING_RESPONSES: &[(&str, &[&str])] = &[
		("response/bedrock-titan/embeddings.json", &[BEDROCK_TITAN]),
		("response/bedrock-cohere/embeddings.json", &[BEDROCK_COHERE]),
		("response/vertex/embeddings.json", &[VERTEX]),
	];
	const COUNT_TOKEN_RESPONSES: &[(&str, &[&str])] = &[("count_tokens", &[ANTHROPIC])];

	#[tokio::test]
	async fn from_bedrock() {
		let to_completions =
			|i| conversion::bedrock::from_completions::translate_response(&i, &strng::new("fake-model"));
		let to_messages =
			|i| conversion::bedrock::from_messages::translate_response(&i, &strng::new("fake-model"));
		let to_responses =
			|i| conversion::bedrock::from_responses::translate_response(&i, &strng::new("fake-model"));

		for (name, providers) in BEDROCK_RESPONSES {
			let test = &format!("response/bedrock/{name}.json");
			for provider in *providers {
				match *provider {
					COMPLETIONS => test_response(COMPLETIONS, test, to_completions),
					MESSAGES => test_response(MESSAGES, test, to_messages),
					RESPONSES => test_response(RESPONSES, test, to_responses),
					other => panic!("unsupported provider in BEDROCK_RESPONSES: {other}"),
				}
			}
		}

		let stream_to_completions = |i, log| {
			Ok(conversion::bedrock::from_completions::translate_stream(
				i,
				0,
				log,
				"model",
				"request-id",
			))
		};
		let stream_to_messages = |i, log| {
			Ok(conversion::bedrock::from_messages::translate_stream(
				i,
				0,
				log,
				"model",
				"request-id",
			))
		};
		let stream_to_responses = |i, log| {
			Ok(conversion::bedrock::from_responses::translate_stream(
				i,
				0,
				log,
				"model",
				"request-id",
			))
		};
		for (name, providers) in BEDROCK_STREAM_RESPONSES {
			let test = &format!("response/bedrock/{name}.bin");
			for provider in *providers {
				match *provider {
					COMPLETIONS => test_streaming(COMPLETIONS, test, stream_to_completions).await,
					MESSAGES => test_streaming(MESSAGES, test, stream_to_messages).await,
					RESPONSES => test_streaming(RESPONSES, test, stream_to_responses).await,
					other => panic!("unsupported provider in BEDROCK_STREAM_RESPONSES: {other}"),
				}
			}
		}
	}

	#[tokio::test]
	async fn from_anthropic() {
		let to_completions = |i| conversion::messages::from_completions::translate_response(&i);
		for (name, providers) in ANTHROPIC_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => test_response(ANTHROPIC, test, to_completions),
					other => panic!("unsupported provider in ANTHROPIC_RESPONSES: {other}"),
				}
			}
		}

		let stream_to_completions = |i, log| {
			Ok(conversion::messages::from_completions::translate_stream(
				i, 1024, log,
			))
		};
		for (name, providers) in ANTHROPIC_STREAM_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					COMPLETIONS => test_streaming(COMPLETIONS, test, stream_to_completions).await,
					ANTHROPIC => test_streaming(ANTHROPIC, test, stream_to_completions).await,
					other => panic!("unsupported provider in ANTHROPIC_STREAM_RESPONSES: {other}"),
				}
			}
		}
	}

	#[tokio::test]
	async fn from_vertex() {
		let to_messages = |bytes: Bytes| -> Result<Box<dyn ResponseType>, AIError> {
			Ok(Box::new(
				serde_json::from_slice::<types::messages::Response>(&bytes)
					.map_err(AIError::ResponseParsing)?,
			))
		};
		for (name, providers) in VERTEX_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					VERTEX => test_response(VERTEX, test, to_messages),
					other => panic!("unsupported provider in VERTEX_RESPONSES: {other}"),
				}
			}
		}

		let stream_to_messages =
			|body, log| Ok(conversion::messages::passthrough_stream(body, 1024, log));
		for (name, providers) in VERTEX_STREAM_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					VERTEX => test_streaming(VERTEX, test, stream_to_messages).await,
					other => panic!("unsupported provider in VERTEX_STREAM_RESPONSES: {other}"),
				}
			}
		}
	}

	#[tokio::test]
	async fn from_embeddings() {
		let titan = |i: Bytes| {
			conversion::bedrock::from_embeddings::translate_response(
				&i,
				&http::HeaderMap::new(),
				"amazon.titan-embed-text-v2:0",
			)
		};
		let cohere = |i: Bytes| {
			conversion::bedrock::from_embeddings::translate_response(
				&i,
				&http::HeaderMap::new(),
				"cohere.embed-english-v3",
			)
		};
		let vertex =
			|i: Bytes| conversion::vertex::from_embeddings::translate_response(&i, "text-embedding-004");

		for (test, providers) in EMBEDDING_RESPONSES {
			for provider in *providers {
				match *provider {
					BEDROCK_TITAN => test_response(BEDROCK_TITAN, test, titan),
					BEDROCK_COHERE => test_response(BEDROCK_COHERE, test, cohere),
					VERTEX => test_response(VERTEX, test, vertex),
					other => panic!("unsupported provider in EMBEDDING_RESPONSES: {other}"),
				}
			}
		}
	}

	#[tokio::test]
	async fn from_count_tokens() {
		for (name, providers) in COUNT_TOKEN_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => {
						let input_path = fixture_path(test);
						let response_str =
							&fs::read_to_string(&input_path).expect("Failed to read response file");
						let bytes = Bytes::copy_from_slice(response_str.as_bytes());
						let provider_value = serde_json::from_str::<Value>(response_str).unwrap();

						let (returned_bytes, count) =
							types::count_tokens::Response::translate_response(bytes.clone())
								.expect("Failed to translate count_tokens response");

						assert_eq!(
							returned_bytes, bytes,
							"Response bytes should be returned unchanged"
						);

						let resp: types::count_tokens::Response =
							serde_json::from_slice(&returned_bytes).expect("Failed to deserialize response");
						let (snapshot_path, snapshot_name) = snapshot_path_and_name(test, ANTHROPIC);

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
					},
					other => panic!("unsupported provider in COUNT_TOKEN_RESPONSES: {other}"),
				}
			}
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
