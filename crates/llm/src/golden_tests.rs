use std::fs;
use std::path::{Path, PathBuf};

use agent_core::strng;
use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;

fn test_root() -> &'static Path {
	Path::new("src/tests")
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

fn test_request<I>(
	provider: &str,
	relative_path: &str,
	xlate: impl Fn(I) -> Result<Vec<u8>, AIError>,
) where
	I: DeserializeOwned,
{
	let input_path = fixture_path(relative_path);
	let input_str = fs::read_to_string(&input_path).expect("failed to read input file");
	let input_raw: Value = serde_json::from_str(&input_str).expect("failed to parse input JSON");
	let input_typed: I = serde_json::from_str(&input_str).expect("failed to parse input JSON");

	let provider_response =
		xlate(input_typed).expect("failed to translate input format to provider request");
	let provider_value =
		serde_json::from_slice::<Value>(&provider_response).expect("failed to parse provider JSON");
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

fn test_response(
	provider: &str,
	relative_path: &str,
	xlate: impl Fn(Bytes) -> Result<Box<dyn ResponseType>, AIError>,
) {
	let input_path = fixture_path(relative_path);
	let provider_bytes = fs::read(&input_path)
		.unwrap_or_else(|e| panic!("{relative_path}: failed to read response input file: {e}"));
	let provider_value = serde_json::from_slice::<Value>(&provider_bytes)
		.unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&provider_bytes).to_string()));

	let resp = xlate(Bytes::copy_from_slice(&provider_bytes))
		.expect("failed to translate provider response to expected format");
	let llm_response = resp.to_llm_response(false);
	let raw = resp.serialize().expect("failed to serialize response");
	let resp_val = serde_json::from_slice::<Value>(&raw)
		.unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&raw).to_string()));
	let report = json!({
		"response": resp_val,
		"parsed": llm_response,
	});
	let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, provider);

	insta::with_settings!({
		info => &provider_value,
		description => input_path.to_string_lossy().to_string(),
		omit_expression => true,
		prepend_module_to_snapshot => false,
		snapshot_path => snapshot_path,
	}, {
		insta::assert_json_snapshot!(snapshot_name, report, {
			".response.id" => "[id]",
			".response.output.*.id" => "[id]",
			".response.created" => "[date]",
		});
	});
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

const ANTHROPIC: &str = "anthropic";
const BEDROCK: &str = "bedrock";
const VERTEX: &str = "vertex";
const OPENAI: &str = "openai";
const GEMINI: &str = "gemini";
const COMPLETIONS: &str = "completions";
const BEDROCK_TITAN: &str = "bedrock-titan";
const BEDROCK_COHERE: &str = "bedrock-cohere";
const COHERE: &str = "cohere";
const VERTEX_GEMINI: &str = "vertex-gemini";

#[test]
fn request_conversion_golden() {
	let bedrock_claude = bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let bedrock_titan = bedrock::Provider {
		model: Some(strng::new("amazon.titan-embed-text-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let bedrock_cohere = bedrock::Provider {
		model: Some(strng::new("cohere.embed-english-v3")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let bedrock_rerank = bedrock::Provider {
		model: Some(strng::new("cohere.rerank-v3-5:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let vertex_anthropic = vertex::Provider {
		model: Some(strng::new("anthropic/claude-sonnet-4-5")),
		region: Some(strng::new("us-central1")),
		project_id: strng::new("test-project-123"),
	};
	let vertex_rerank = vertex::Provider {
		model: Some(strng::new("semantic-ranker-default@latest")),
		region: Some(strng::new("global")),
		project_id: strng::new("test-project-123"),
	};

	for name in ["basic", "full", "tool-call", "reasoning", "reasoning_max"] {
		let path = format!("requests/completions/{name}.json");
		test_request(ANTHROPIC, &path, |i| {
			conversion::messages::from_completions::translate(&i)
		});
		if name != "reasoning_max" {
			test_request(BEDROCK, &path, |i| {
				conversion::bedrock::from_completions::translate(&i, &bedrock_claude, None, None)
					.map(|r| r.body)
			});
		}
		if name != "full" {
			test_request(VERTEX_GEMINI, &path, |i| {
				conversion::vertex_gemini::from_completions::translate(&i, Some("gemini-2.5-pro"))
			});
		}
	}
	for name in [
		"parallel-tool-call",
		"reasoning_replay",
		"reasoning_replay_unsigned",
	] {
		let path = format!("requests/completions/{name}.json");
		test_request(BEDROCK, &path, |i| {
			conversion::bedrock::from_completions::translate(&i, &bedrock_claude, None, None)
				.map(|r| r.body)
		});
		if name == "parallel-tool-call" {
			test_request(VERTEX_GEMINI, &path, |i| {
				conversion::vertex_gemini::from_completions::translate(&i, Some("gemini-2.5-pro"))
			});
		}
	}
	// `generation-config` stands in for `full`, whose remote http image the Gemini path rejects.
	for name in [
		"image-inline",
		"image-file",
		"structured-output",
		"multi-turn-tools",
		"generation-config",
	] {
		let path = format!("requests/completions/{name}.json");
		test_request(VERTEX_GEMINI, &path, |i| {
			conversion::vertex_gemini::from_completions::translate(&i, Some("gemini-2.5-pro"))
		});
	}

	for name in ["basic", "system_message", "tools", "reasoning"] {
		let path = format!("requests/messages/{name}.json");
		test_request(COMPLETIONS, &path, |i| {
			conversion::completions::from_messages::translate(&i)
		});
		test_request(BEDROCK, &path, |i| {
			conversion::bedrock::from_messages::translate(&i, &bedrock_claude, None).map(|r| r.body)
		});
		test_request(VERTEX, &path, |input: types::messages::Request| {
			let body = serde_json::to_vec(&input).map_err(AIError::RequestMarshal)?;
			vertex_anthropic.prepare_anthropic_message_body(body)
		});
	}
	test_request(
		COMPLETIONS,
		"requests/messages/gpt_adaptive_thinking_with_tools.json",
		|i| conversion::completions::from_messages::translate(&i),
	);
	test_request(BEDROCK, "requests/messages/reasoning_replay.json", |i| {
		conversion::bedrock::from_messages::translate(&i, &bedrock_claude, None).map(|r| r.body)
	});

	for name in ["basic", "instructions", "input-list", "parallel-tool-call"] {
		let path = format!("requests/responses/{name}.json");
		test_request(BEDROCK, &path, |i| {
			conversion::bedrock::from_responses::translate(&i, &bedrock_claude, None, None)
				.map(|r| r.body)
		});
		test_request(GEMINI, &path, |i| {
			conversion::openai_compat::from_responses::translate(&i)
		});
	}

	for name in ["basic", "array"] {
		let path = format!("requests/embeddings/{name}.json");
		test_request(OPENAI, &path, |i: types::embeddings::Request| {
			serde_json::to_vec(&i).map_err(AIError::RequestMarshal)
		});
		test_request(BEDROCK_COHERE, &path, |i| {
			conversion::bedrock::from_embeddings::translate(&i, &bedrock_cohere)
		});
		test_request(VERTEX, &path, |i: types::embeddings::Request| {
			conversion::vertex::from_embeddings::translate(&i)
		});
		if name == "basic" {
			test_request(BEDROCK_TITAN, &path, |i| {
				conversion::bedrock::from_embeddings::translate(&i, &bedrock_titan)
			});
		}
	}

	for name in ["basic", "passthrough-fields"] {
		let path = format!("requests/rerank/{name}.json");
		test_request(COHERE, &path, |i: types::rerank::Request| {
			serde_json::to_vec(&i).map_err(AIError::RequestMarshal)
		});
		test_request(BEDROCK, &path, |i: types::rerank::Request| {
			conversion::bedrock::from_rerank::translate(&i, &bedrock_rerank)
		});
		test_request(VERTEX, &path, |i: types::rerank::Request| {
			conversion::vertex::from_rerank::translate(&i, &vertex_rerank)
		});
	}

	let mut headers = http::HeaderMap::new();
	headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
	for name in ["basic", "with_system"] {
		let path = format!("requests/count-tokens/{name}.json");
		test_request(ANTHROPIC, &path, |i: types::count_tokens::Request| {
			serde_json::to_vec(&i).map_err(AIError::RequestMarshal)
		});
		test_request(BEDROCK, &path, |i: types::count_tokens::Request| {
			conversion::bedrock::from_anthropic_token_count::translate(&i, &headers)
		});
		test_request(VERTEX, &path, |i: types::count_tokens::Request| {
			let body = serde_json::to_vec(&i).map_err(AIError::RequestMarshal)?;
			vertex_anthropic.prepare_anthropic_count_tokens_body(body)
		});
	}

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
	test_request::<types::responses::Request>(
		OPENAI,
		"requests/policies/openai_with_text_input.json",
		apply_test_prompts,
	);
	test_request::<types::responses::Request>(
		OPENAI,
		"requests/responses/assistant-history.json",
		apply_test_prompts,
	);
}

#[test]
fn response_conversion_golden() {
	for name in ["basic", "tool", "reasoning", "reasoning_unsigned"] {
		let path = format!("response/bedrock/{name}.json");
		test_response("bedrock-messages", &path, |i| {
			conversion::bedrock::from_messages::translate_response(&i, "input-model", None)
		});
		test_response("bedrock-completions", &path, |i| {
			conversion::bedrock::from_completions::translate_response(&i, "input-model", None)
		});
		test_response("bedrock-responses", &path, |i| {
			conversion::bedrock::from_responses::translate_response(&i, "input-model", None)
		});
	}
	test_response(
		"bedrock-completions",
		"response/bedrock/cache_write.json",
		|i| conversion::bedrock::from_completions::translate_response(&i, "input-model", None),
	);

	for name in ["basic", "tool", "thinking", "multiple_text_blocks"] {
		let path = format!("response/anthropic/{name}.json");
		test_response("messages-messages", &path, |i| {
			serde_json::from_slice::<types::messages::Response>(&i)
				.map(|e| Box::new(e) as Box<dyn ResponseType>)
				.map_err(AIError::ResponseParsing)
		});
		test_response("messages-completions", &path, |i| {
			conversion::messages::from_completions::translate_response(&i)
		});
	}

	for name in [
		"basic",
		"audio",
		"openrouter_reasoning",
		"gemini_zero_completion_tokens",
		"gemini_with_completion_tokens",
	] {
		let path = format!("response/completions/{name}.json");
		test_response("completions-completions", &path, |i| {
			serde_json::from_slice::<types::completions::Response>(&i)
				.map(|e| Box::new(e) as Box<dyn ResponseType>)
				.map_err(AIError::ResponseParsing)
		});
		test_response("completions-messages", &path, |i| {
			conversion::completions::from_messages::translate_response(&i)
		});
	}

	for (provider, path) in [
		(BEDROCK_TITAN, "response/bedrock-titan/embeddings.json"),
		(BEDROCK_COHERE, "response/bedrock-cohere/embeddings.json"),
	] {
		let model = if provider == BEDROCK_TITAN {
			"amazon.titan-embed-text-v2:0"
		} else {
			"cohere.embed-english-v3"
		};
		test_response(provider, path, |i| {
			conversion::bedrock::from_embeddings::translate_response(&i, &http::HeaderMap::new(), model)
		});
	}
	test_response(VERTEX, "response/vertex/embeddings.json", |i| {
		conversion::vertex::from_embeddings::translate_response(&i, "text-embedding-004")
	});
	for path in [
		"response/openai/embeddings.json",
		"response/openai/gemini-embeddings.json",
	] {
		test_response(OPENAI, path, |i| {
			serde_json::from_slice::<types::embeddings::Response>(&i)
				.map(|e| Box::new(e) as Box<dyn ResponseType>)
				.map_err(AIError::ResponseParsing)
		});
	}

	test_response(BEDROCK, "response/bedrock/rerank.json", |i| {
		conversion::bedrock::from_rerank::translate_response(&i)
	});
	for path in [
		"response/vertex/rerank.json",
		"response/vertex/rerank-no-details.json",
	] {
		test_response(VERTEX, path, |i| {
			conversion::vertex::from_rerank::translate_response(&i)
		});
	}
	test_response(COHERE, "response/cohere/rerank.json", |i| {
		types::rerank::parse_response_lenient(&i)
			.map(|e| Box::new(e) as Box<dyn ResponseType>)
			.map_err(AIError::ResponseParsing)
	});

	// Native Vertex Gemini responses translated to the OpenAI completions shape.
	// Streaming translation is covered by conversion::vertex_gemini unit tests.
	for name in ["basic", "tool", "reasoning", "blocked"] {
		let path = format!("response/vertex-gemini/{name}.json");
		test_response("vertex-gemini-completions", &path, |i| {
			conversion::vertex_gemini::to_completions::translate_response(&i)
		});
	}
}

#[test]
fn get_messages_golden() {
	fn extract_messages<R: RequestType + DeserializeOwned>(fixture: &str, provider: &str) {
		let path = fixture_path(fixture);
		let input_str = fs::read_to_string(&path).expect("failed to read input file");
		let raw: Value = serde_json::from_str(&input_str).expect("failed to parse input JSON");
		let request: R = serde_json::from_str(&input_str).expect("failed to parse JSON");

		let out: Vec<Value> = request
			.get_messages()
			.iter()
			.map(|m| {
				json!({
					"role": m.role.as_str(),
					"content": m.content.as_str(),
				})
			})
			.collect();

		let (snapshot_path, snapshot_name) = snapshot_path_and_name(fixture, provider);
		insta::with_settings!({
			info => &raw,
			description => path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
		}, {
			insta::assert_json_snapshot!(snapshot_name, out);
		});
	}

	extract_messages::<types::completions::Request>(
		"requests/completions/full.json",
		"get-messages-completions",
	);
	extract_messages::<types::messages::Request>(
		"requests/completions/full.json",
		"get-messages-messages",
	);
	extract_messages::<types::responses::Request>(
		"requests/responses/assistant-history.json",
		"get-messages-responses",
	);
}
