use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_core::strng;
use base64::Engine;
use bytes::Bytes;
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;

fn fixture_path(relative_path: &str) -> PathBuf {
	Path::new("src/tests").join(relative_path)
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

mod requests {
	use super::*;

	fn test_request<I>(
		provider: &str,
		relative_path: &str,
		xlate: impl FnOnce(&mut I) -> Result<Vec<u8>, AIError>,
	) where
		I: DeserializeOwned + RequestType,
	{
		let input_path = fixture_path(relative_path);
		let input_str = fs::read_to_string(&input_path).expect("failed to read input file");
		let input_raw: Value = serde_json::from_str(&input_str).expect("failed to parse input JSON");
		let mut input_typed: I = serde_json::from_str(&input_str).expect("failed to parse input JSON");
		let provider_response =
			xlate(&mut input_typed).expect("failed to translate input format to provider request");
		let mut llm_request = input_typed
			.to_llm_request(
				strng::new(match provider {
					COMPLETIONS => OPENAI,
					BEDROCK_TITAN | BEDROCK_COHERE => BEDROCK,
					VERTEX_GEMINI => VERTEX,
					provider => provider,
				}),
				false,
			)
			.expect("failed to extract LLM request metadata");
		if llm_request.input_format.supports_prompt_guard() {
			llm_request.prompt = Some(input_typed.get_messages().into());
		}

		let provider_value =
			serde_json::from_slice::<Value>(&provider_response).expect("failed to parse provider JSON");
		let report = json!({
			"request": provider_value,
			"parsed": llm_request,
		});
		let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, provider);

		insta::with_settings!({
			info => &input_raw,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
		}, {
			insta::assert_json_snapshot!(snapshot_name, report, {
				".request.id" => "[id]",
				".request.created" => "[date]",
				".request.metadata" => insta::sorted_redaction(),
				".request.additionalModelRequestFields.metadata" => insta::sorted_redaction(),
			});
		});
	}

	fn apply_test_prompts<R: RequestType + Serialize>(r: &mut R) -> Result<Vec<u8>, AIError> {
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
		serde_json::to_vec(r).map_err(AIError::RequestMarshal)
	}

	const COMPLETION_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK, VERTEX_GEMINI]),
		("full", &[ANTHROPIC, BEDROCK]),
		("tool-call", &[ANTHROPIC, BEDROCK, VERTEX_GEMINI]),
		("parallel-tool-call", &[BEDROCK, VERTEX_GEMINI]),
		("reasoning", &[ANTHROPIC, BEDROCK, VERTEX_GEMINI]),
		("reasoning_max", &[ANTHROPIC, VERTEX_GEMINI]),
		("reasoning_replay", &[BEDROCK]),
		("reasoning_replay_unsigned", &[BEDROCK]),
		("image-url", &[ANTHROPIC]),
		("image-inline", &[VERTEX_GEMINI]),
		("image-file", &[VERTEX_GEMINI]),
		("structured-output", &[VERTEX_GEMINI]),
		("multi-turn-tools", &[VERTEX_GEMINI]),
		// Stands in for `full`, whose remote HTTP image the Gemini path rejects.
		("generation-config", &[VERTEX_GEMINI]),
	];
	const MESSAGES_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		("system_message", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		("tools", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		("server_tools", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		("reasoning", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		("metadata", &[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX]),
		(
			"structured-output",
			&[ANTHROPIC, COMPLETIONS, BEDROCK, VERTEX],
		),
		("cache_control", &[COMPLETIONS]),
		("gpt_adaptive_thinking_with_tools", &[COMPLETIONS]),
		("reasoning_replay", &[BEDROCK]),
		("tool_history_without_tools", &[BEDROCK]),
	];
	const RESPONSES_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[BEDROCK, GEMINI]),
		("instructions", &[BEDROCK, GEMINI]),
		("input-list", &[BEDROCK, GEMINI]),
		("parallel-tool-call", &[BEDROCK, GEMINI]),
		("structured-output", &[BEDROCK]),
		("input-media", &[BEDROCK]),
		("cache_control", &[GEMINI]),
	];
	const COUNT_TOKENS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK, VERTEX]),
		("with_system", &[ANTHROPIC, BEDROCK, VERTEX]),
	];
	const EMBEDDINGS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[OPENAI, BEDROCK_TITAN, BEDROCK_COHERE, VERTEX]),
		("cohere-v4", &[BEDROCK_COHERE]),
		("array", &[OPENAI, BEDROCK_COHERE, VERTEX]),
		("full", &[VERTEX]),
	];
	const RERANK_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[COHERE, BEDROCK, VERTEX]),
		("passthrough-fields", &[COHERE, BEDROCK, VERTEX]),
	];

	#[test]
	fn from_completions() {
		let bedrock = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		for (name, providers) in COMPLETION_REQUESTS {
			let path = format!("requests/completions/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => test_request(ANTHROPIC, &path, |i| {
						conversion::messages::from_completions::translate(i)
					}),
					BEDROCK => test_request(BEDROCK, &path, |i| {
						conversion::bedrock::from_completions::translate(i, &bedrock, None, None)
							.map(|r| r.body)
					}),
					VERTEX_GEMINI => test_request(VERTEX_GEMINI, &path, |i| {
						conversion::vertex_gemini::from_completions::translate(i, Some("gemini-2.5-pro"))
					}),
					other => panic!("unsupported provider in COMPLETION_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn from_messages() {
		let bedrock = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		let vertex = vertex::Provider {
			model: Some(strng::new("anthropic/claude-sonnet-4-5")),
			region: Some(strng::new("us-central1")),
			project_id: strng::new("test-project-123"),
		};
		for (name, providers) in MESSAGES_REQUESTS {
			let path = format!("requests/messages/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => test_request(ANTHROPIC, &path, |i: &mut types::messages::Request| {
						serde_json::to_vec(i).map_err(AIError::RequestMarshal)
					}),
					COMPLETIONS => test_request(COMPLETIONS, &path, |i| {
						conversion::completions::from_messages::translate(i)
					}),
					BEDROCK => test_request(BEDROCK, &path, |i| {
						conversion::bedrock::from_messages::translate(i, &bedrock, None).map(|r| r.body)
					}),
					VERTEX => test_request(VERTEX, &path, |i: &mut types::messages::Request| {
						let body = serde_json::to_vec(i).map_err(AIError::RequestMarshal)?;
						vertex.prepare_anthropic_message_body(body)
					}),
					other => panic!("unsupported provider in MESSAGES_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn from_responses() {
		let bedrock = bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		for (name, providers) in RESPONSES_REQUESTS {
			let path = format!("requests/responses/{name}.json");
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(BEDROCK, &path, |i| {
						conversion::bedrock::from_responses::translate(i, &bedrock, None, None).map(|r| r.body)
					}),
					GEMINI => test_request(GEMINI, &path, |i| {
						conversion::openai_compat::from_responses::translate(i)
					}),
					other => panic!("unsupported provider in RESPONSES_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn embeddings() {
		let titan = bedrock::Provider {
			model: Some(strng::new("amazon.titan-embed-text-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		let cohere = bedrock::Provider {
			model: Some(strng::new("cohere.embed-english-v3")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		let cohere_v4 = bedrock::Provider {
			model: Some(strng::new("cohere.embed-v4:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		for (name, providers) in EMBEDDINGS_REQUESTS {
			let path = format!("requests/embeddings/{name}.json");
			for provider in *providers {
				match *provider {
					OPENAI => test_request(OPENAI, &path, |i: &mut types::embeddings::Request| {
						serde_json::to_vec(i).map_err(AIError::RequestMarshal)
					}),
					BEDROCK_TITAN => test_request(BEDROCK_TITAN, &path, |i| {
						conversion::bedrock::from_embeddings::translate(i, &titan)
					}),
					BEDROCK_COHERE => test_request(BEDROCK_COHERE, &path, |i| {
						let provider = if *name == "cohere-v4" {
							&cohere_v4
						} else {
							&cohere
						};
						conversion::bedrock::from_embeddings::translate(i, provider)
					}),
					VERTEX => test_request(VERTEX, &path, |i: &mut types::embeddings::Request| {
						conversion::vertex::from_embeddings::translate(i)
					}),
					other => panic!("unsupported provider in EMBEDDINGS_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn rerank() {
		let bedrock = bedrock::Provider {
			model: Some(strng::new("cohere.rerank-v3-5:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		};
		let vertex = vertex::Provider {
			model: Some(strng::new("semantic-ranker-default@latest")),
			region: Some(strng::new("global")),
			project_id: strng::new("test-project-123"),
		};
		for (name, providers) in RERANK_REQUESTS {
			let path = format!("requests/rerank/{name}.json");
			for provider in *providers {
				match *provider {
					COHERE => test_request(COHERE, &path, |i: &mut types::rerank::Request| {
						serde_json::to_vec(i).map_err(AIError::RequestMarshal)
					}),
					BEDROCK => test_request(BEDROCK, &path, |i: &mut types::rerank::Request| {
						conversion::bedrock::from_rerank::translate(i, &bedrock)
					}),
					VERTEX => test_request(VERTEX, &path, |i: &mut types::rerank::Request| {
						conversion::vertex::from_rerank::translate(i, &vertex)
					}),
					other => panic!("unsupported provider in RERANK_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn count_tokens() {
		let mut headers = http::HeaderMap::new();
		headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
		let vertex = vertex::Provider {
			model: Some(strng::new("anthropic/claude-sonnet-4-5")),
			region: Some(strng::new("us-central1")),
			project_id: strng::new("test-project-123"),
		};
		for (name, providers) in COUNT_TOKENS_REQUESTS {
			let path = format!("requests/count-tokens/{name}.json");
			for provider in *providers {
				match *provider {
					ANTHROPIC => test_request(ANTHROPIC, &path, |i: &mut types::count_tokens::Request| {
						serde_json::to_vec(i).map_err(AIError::RequestMarshal)
					}),
					BEDROCK => test_request(BEDROCK, &path, |i: &mut types::count_tokens::Request| {
						conversion::bedrock::from_anthropic_token_count::translate(i, &headers)
					}),
					VERTEX => test_request(VERTEX, &path, |i: &mut types::count_tokens::Request| {
						let body = serde_json::to_vec(i).map_err(AIError::RequestMarshal)?;
						vertex.prepare_anthropic_count_tokens_body(body)
					}),
					other => panic!("unsupported provider in COUNT_TOKENS_REQUESTS: {other}"),
				}
			}
		}
	}

	#[test]
	fn prompt_enrichment() {
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
	fn get_messages() {
		fn extract<R: RequestType + DeserializeOwned>(fixture: &str, provider: &str) {
			let path = fixture_path(fixture);
			let input_str = fs::read_to_string(&path).expect("failed to read input file");
			let raw: Value = serde_json::from_str(&input_str).expect("failed to parse input JSON");
			let request: R = serde_json::from_str(&input_str).expect("failed to parse JSON");

			let messages: Vec<Value> = request
				.get_messages()
				.iter()
				.map(|message| {
					json!({
						"role": message.role.as_str(),
						"content": message.content.as_str(),
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
				insta::assert_json_snapshot!(snapshot_name, messages);
			});
		}

		extract::<types::completions::Request>(
			"requests/completions/full.json",
			"get-messages-completions",
		);
		extract::<types::messages::Request>("requests/completions/full.json", "get-messages-messages");
		extract::<types::responses::Request>(
			"requests/responses/assistant-history.json",
			"get-messages-responses",
		);
	}
}

mod responses {
	use super::*;

	fn test_response(
		provider: &str,
		relative_path: &str,
		xlate: impl Fn(Bytes) -> Result<Box<dyn ResponseType>, AIError>,
	) {
		let input_path = fixture_path(relative_path);
		let provider_bytes = fs::read(&input_path)
			.unwrap_or_else(|e| panic!("{relative_path}: failed to read response input file: {e}"));
		let mut provider_value = serde_json::from_slice::<Value>(&provider_bytes)
			.unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&provider_bytes).to_string()));
		if let Value::String(raw) = &mut provider_value {
			*raw = raw.replace("\r\n", "\n");
		}

		let resp = xlate(Bytes::copy_from_slice(&provider_bytes))
			.expect("failed to translate provider response to expected format");
		let llm_response = resp.to_llm_response(crate::LogContentFields {
			completion: false,
			tool_calls: true,
		});
		let raw = resp.serialize().expect("failed to serialize response");
		let mut resp_val = serde_json::from_slice::<Value>(&raw)
			.unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&raw).to_string()));
		if let Value::String(raw) = &mut resp_val {
			*raw = raw.replace("\r\n", "\n");
		}
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
				".response.created_at" => "[date]",
			});
		});
	}

	#[derive(Clone)]
	struct TestStreamingReporter {
		info: Arc<Mutex<LLMInfo>>,
	}

	impl StreamingUsageReporter for TestStreamingReporter {
		fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
			f(&mut self.info.lock().unwrap());
		}

		fn report_usage(&mut self) {}
	}

	async fn test_streaming(
		provider: &str,
		relative_path: &str,
		xlate: impl FnOnce(
			http::Response<axum_core::body::Body>,
			StreamingUsageGuard,
		) -> http::Response<axum_core::body::Body>,
	) {
		let input_path = fixture_path(relative_path);
		let input_bytes = fs::read(&input_path)
			.unwrap_or_else(|e| panic!("{relative_path}: failed to read streaming input file: {e}"));
		let info = Arc::new(Mutex::new(LLMInfo::new(
			LLMRequest {
				input_tokens: None,
				input_format: InputFormat::Detect,
				cache_convention: CacheTokenConvention::pending(),
				request_model: strng::literal!("input-model"),
				provider: Default::default(),
				streaming: true,
				params: Default::default(),
				prompt: None,
				provider_state: None,
			},
			LLMResponse::default(),
		)));
		let reporter = TestStreamingReporter { info: info.clone() };
		let mut response = http::Response::new(axum_core::body::Body::from(input_bytes));
		response
			.headers_mut()
			.insert("x-amzn-requestid", "request_id".parse().unwrap());
		let response = xlate(response, StreamingUsageGuard::new(Box::new(reporter)));
		let response_bytes = response.collect().await.unwrap().to_bytes();
		let llm_response = info.lock().unwrap().response.clone();
		let response_body = match String::from_utf8(response_bytes.to_vec()) {
			Ok(s)
				if !s
					.chars()
					.any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
			{
				s
			},
			Ok(s) => format!(
				"base64: {}",
				base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
			),
			Err(e) => format!(
				"base64: {}",
				base64::engine::general_purpose::STANDARD.encode(e.into_bytes())
			),
		}
		.replace("\r\n", "\n");
		let report = format!(
			"{response_body}\n\n{}",
			serde_json::to_string_pretty(&llm_response).unwrap()
		);
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
			],
		}, {
			insta::assert_snapshot!(format!("{snapshot_name}-streaming"), report);
		});
	}

	const COMPLETIONS_TO_COMPLETIONS: &str = "completions-completions";
	const COMPLETIONS_TO_MESSAGES: &str = "completions-messages";
	const COMPLETIONS_TO_RESPONSES: &str = "completions-responses";
	const COMPLETIONS_TO_DETECT: &str = "completions-detect";
	const MESSAGES_TO_MESSAGES: &str = "messages-messages";
	const MESSAGES_TO_COMPLETIONS: &str = "messages-completions";
	const MESSAGES_TO_DETECT: &str = "messages-detect";
	const BEDROCK_TO_COMPLETIONS: &str = "bedrock-completions";
	const BEDROCK_TO_MESSAGES: &str = "bedrock-messages";
	const BEDROCK_TO_RESPONSES: &str = "bedrock-responses";
	const BEDROCK_TO_DETECT: &str = "bedrock-detect";
	const RESPONSES_TO_RESPONSES: &str = "responses-responses";
	const RESPONSES_TO_DETECT: &str = "responses-detect";
	const VERTEX_GEMINI_TO_COMPLETIONS: &str = "vertex-gemini-completions";

	const ALL_BEDROCK: &[&str] = &[
		BEDROCK_TO_COMPLETIONS,
		BEDROCK_TO_MESSAGES,
		BEDROCK_TO_RESPONSES,
	];
	const BEDROCK_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_BEDROCK),
		("max_tokens", &[BEDROCK_TO_RESPONSES]),
		("tool", ALL_BEDROCK),
		("reasoning", ALL_BEDROCK),
		("reasoning_unsigned", ALL_BEDROCK),
		(
			"cache_write",
			&[BEDROCK_TO_COMPLETIONS, BEDROCK_TO_RESPONSES],
		),
	];
	const ALL_ANTHROPIC: &[&str] = &[
		MESSAGES_TO_MESSAGES,
		MESSAGES_TO_COMPLETIONS,
		MESSAGES_TO_DETECT,
	];
	const ANTHROPIC_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_ANTHROPIC),
		("tool", ALL_ANTHROPIC),
		("thinking", ALL_ANTHROPIC),
		("multiple_text_blocks", ALL_ANTHROPIC),
	];
	const ALL_COMPLETIONS: &[&str] = &[
		COMPLETIONS_TO_COMPLETIONS,
		COMPLETIONS_TO_MESSAGES,
		COMPLETIONS_TO_RESPONSES,
		COMPLETIONS_TO_DETECT,
	];
	const COMPLETIONS_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_COMPLETIONS),
		("audio", ALL_COMPLETIONS),
		(
			"cache_write",
			&[COMPLETIONS_TO_COMPLETIONS, COMPLETIONS_TO_MESSAGES],
		),
		("openrouter_reasoning", ALL_COMPLETIONS),
		("gemini_zero_completion_tokens", ALL_COMPLETIONS),
		("gemini_with_completion_tokens", ALL_COMPLETIONS),
		("tool_call", ALL_COMPLETIONS),
		(
			"truncated_tool_call",
			&[COMPLETIONS_TO_COMPLETIONS, COMPLETIONS_TO_RESPONSES],
		),
	];
	const RESPONSES_RESPONSES: &[(&str, &[&str])] = &[
		("basic", &[RESPONSES_TO_RESPONSES, RESPONSES_TO_DETECT]),
		("truncated_tool_call", &[RESPONSES_TO_RESPONSES]),
	];
	const EMBEDDING_RESPONSES: &[(&str, &str)] = &[
		("response/bedrock-titan/embeddings.json", BEDROCK_TITAN),
		("response/bedrock-cohere/embeddings.json", BEDROCK_COHERE),
		("response/vertex/embeddings.json", VERTEX),
		("response/openai/embeddings.json", OPENAI),
		("response/openai/gemini-embeddings.json", OPENAI),
	];
	const RERANK_RESPONSES: &[(&str, &str)] = &[
		("response/bedrock/rerank.json", BEDROCK),
		("response/vertex/rerank.json", VERTEX),
		("response/vertex/rerank-no-details.json", VERTEX),
		("response/cohere/rerank.json", COHERE),
	];
	const VERTEX_GEMINI_RESPONSES: &[&str] = &["basic", "tool", "reasoning", "blocked"];
	const DETECT_RESPONSES: &[(&str, &str)] = &[
		("response/detect/bedrock-invoke.bin", BEDROCK_TO_DETECT),
		("response/detect/bedrock-basic.bin", BEDROCK_TO_DETECT),
		("response/detect/bedrock-broken.bin", BEDROCK_TO_DETECT),
		("response/detect/broken-sse", COMPLETIONS_TO_DETECT),
		("response/detect/non-json", COMPLETIONS_TO_DETECT),
		(
			"response/detect/stream-image-generation",
			COMPLETIONS_TO_DETECT,
		),
	];
	const BEDROCK_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_BEDROCK),
		("tool", ALL_BEDROCK),
		("reasoning", ALL_BEDROCK),
	];
	const ANTHROPIC_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("stream_basic", ALL_ANTHROPIC),
		("stream_thinking", ALL_ANTHROPIC),
		(
			"stream_tool",
			&[MESSAGES_TO_MESSAGES, MESSAGES_TO_COMPLETIONS],
		),
	];
	const COMPLETIONS_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("stream", ALL_COMPLETIONS),
		(
			"stream_tool_empty_content",
			&[COMPLETIONS_TO_MESSAGES, COMPLETIONS_TO_RESPONSES],
		),
	];
	const VERTEX_GEMINI_STREAM_RESPONSES: &[&str] = &["stream_tool"];
	const RESPONSES_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("stream", &[RESPONSES_TO_RESPONSES, RESPONSES_TO_DETECT]),
		(
			"stream-image",
			&[RESPONSES_TO_RESPONSES, RESPONSES_TO_DETECT],
		),
	];

	#[test]
	fn buffered_chat() {
		for (name, providers) in BEDROCK_RESPONSES {
			let path = format!("response/bedrock/{name}.json");
			for provider in *providers {
				match *provider {
					BEDROCK_TO_MESSAGES => test_response(provider, &path, |i| {
						conversion::bedrock::from_messages::translate_response(&i, "input-model", None)
					}),
					BEDROCK_TO_COMPLETIONS => test_response(provider, &path, |i| {
						conversion::bedrock::from_completions::translate_response(&i, "input-model", None)
					}),
					BEDROCK_TO_RESPONSES => test_response(provider, &path, |i| {
						conversion::bedrock::from_responses::translate_response(&i, "input-model", None)
					}),
					other => panic!("unsupported provider in BEDROCK_RESPONSES: {other}"),
				}
			}
		}

		for (name, providers) in ANTHROPIC_RESPONSES {
			let path = format!("response/anthropic/{name}.json");
			for provider in *providers {
				match *provider {
					MESSAGES_TO_MESSAGES => test_response(provider, &path, |i| {
						serde_json::from_slice::<types::messages::Response>(&i)
							.map(|e| Box::new(e) as Box<dyn ResponseType>)
							.map_err(AIError::ResponseParsing)
					}),
					MESSAGES_TO_COMPLETIONS => test_response(provider, &path, |i| {
						conversion::messages::from_completions::translate_response(&i)
					}),
					MESSAGES_TO_DETECT => test_response(provider, &path, |bytes| {
						Ok(Box::new(
							serde_json::from_slice::<types::detect::Response>(&bytes)
								.unwrap_or_else(|_| types::detect::Response::new_raw(bytes)),
						))
					}),
					other => panic!("unsupported provider in ANTHROPIC_RESPONSES: {other}"),
				}
			}
		}

		for (name, providers) in COMPLETIONS_RESPONSES {
			let path = format!("response/completions/{name}.json");
			for provider in *providers {
				match *provider {
					COMPLETIONS_TO_COMPLETIONS => test_response(provider, &path, |i| {
						serde_json::from_slice::<types::completions::Response>(&i)
							.map(|e| Box::new(e) as Box<dyn ResponseType>)
							.map_err(AIError::ResponseParsing)
					}),
					COMPLETIONS_TO_MESSAGES => test_response(provider, &path, |i| {
						conversion::completions::from_messages::translate_response(&i)
					}),
					COMPLETIONS_TO_RESPONSES => test_response(provider, &path, |i| {
						conversion::openai_compat::to_responses::translate_response(&i, "input-model")
					}),
					COMPLETIONS_TO_DETECT => test_response(provider, &path, |bytes| {
						Ok(Box::new(
							serde_json::from_slice::<types::detect::Response>(&bytes)
								.unwrap_or_else(|_| types::detect::Response::new_raw(bytes)),
						))
					}),
					other => panic!("unsupported provider in COMPLETIONS_RESPONSES: {other}"),
				}
			}
		}

		for (name, providers) in RESPONSES_RESPONSES {
			let path = format!("response/responses/{name}.json");
			for provider in *providers {
				match *provider {
					RESPONSES_TO_RESPONSES => test_response(provider, &path, |bytes| {
						serde_json::from_slice::<types::responses::Response>(&bytes)
							.map(|response| Box::new(response) as Box<dyn ResponseType>)
							.map_err(AIError::ResponseParsing)
					}),
					RESPONSES_TO_DETECT => test_response(provider, &path, |bytes| {
						Ok(Box::new(
							serde_json::from_slice::<types::detect::Response>(&bytes)
								.unwrap_or_else(|_| types::detect::Response::new_raw(bytes)),
						))
					}),
					other => panic!("unsupported provider in RESPONSES_RESPONSES: {other}"),
				}
			}
		}

		for name in VERTEX_GEMINI_RESPONSES {
			let path = format!("response/vertex-gemini/{name}.json");
			test_response(VERTEX_GEMINI_TO_COMPLETIONS, &path, |i| {
				conversion::vertex_gemini::to_completions::translate_response(&i)
			});
		}
	}
	#[test]
	fn embeddings() {
		for (path, provider) in EMBEDDING_RESPONSES {
			match *provider {
				BEDROCK_TITAN | BEDROCK_COHERE => {
					let model = if *provider == BEDROCK_TITAN {
						"amazon.titan-embed-text-v2:0"
					} else {
						"cohere.embed-english-v3"
					};
					test_response(provider, path, |i| {
						conversion::bedrock::from_embeddings::translate_response(
							&i,
							&http::HeaderMap::new(),
							model,
						)
					});
				},
				VERTEX => test_response(provider, path, |i| {
					conversion::vertex::from_embeddings::translate_response(&i, "text-embedding-004")
				}),
				OPENAI => test_response(provider, path, |i| {
					serde_json::from_slice::<types::embeddings::Response>(&i)
						.map(|e| Box::new(e) as Box<dyn ResponseType>)
						.map_err(AIError::ResponseParsing)
				}),
				other => panic!("unsupported provider in EMBEDDING_RESPONSES: {other}"),
			}
		}
	}

	#[test]
	fn rerank() {
		for (path, provider) in RERANK_RESPONSES {
			match *provider {
				BEDROCK => test_response(provider, path, |i| {
					conversion::bedrock::from_rerank::translate_response(&i)
				}),
				VERTEX => test_response(provider, path, |i| {
					conversion::vertex::from_rerank::translate_response(&i)
				}),
				COHERE => test_response(provider, path, |i| {
					types::rerank::parse_response_lenient(&i)
						.map(|e| Box::new(e) as Box<dyn ResponseType>)
						.map_err(AIError::ResponseParsing)
				}),
				other => panic!("unsupported provider in RERANK_RESPONSES: {other}"),
			}
		}
	}

	#[test]
	fn detect_buffered() {
		for (path, provider) in DETECT_RESPONSES {
			test_response(provider, path, |bytes| {
				Ok(Box::new(
					serde_json::from_slice::<types::detect::Response>(&bytes)
						.unwrap_or_else(|_| types::detect::Response::new_raw(bytes)),
				))
			});
		}
	}

	#[test]
	fn count_tokens() {
		let relative_path = "response/anthropic/count_tokens.json";
		let input_path = fixture_path(relative_path);
		let bytes = Bytes::from(
			fs::read(&input_path).unwrap_or_else(|e| panic!("failed to read count-tokens response: {e}")),
		);
		let provider_value: Value = serde_json::from_slice(&bytes).unwrap();
		let (returned_bytes, token_count) =
			types::count_tokens::Response::translate_response(bytes.clone()).unwrap();
		assert_eq!(returned_bytes, bytes);
		let response: types::count_tokens::Response = serde_json::from_slice(&returned_bytes).unwrap();
		let (snapshot_path, snapshot_name) = snapshot_path_and_name(relative_path, ANTHROPIC);

		insta::with_settings!({
			info => &provider_value,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => snapshot_path,
		}, {
			insta::assert_json_snapshot!(snapshot_name, json!({
				"input_tokens": response.input_tokens,
				"token_count": token_count,
			}));
		});
	}

	#[tokio::test]
	async fn streaming() {
		const BUFFER_LIMIT: usize = 1024 * 1024;
		const LOG_CONTENT: LogContentFields = LogContentFields {
			completion: true,
			tool_calls: true,
		};

		for (name, providers) in BEDROCK_STREAM_RESPONSES {
			let path = format!("response/bedrock/{name}.bin");
			for provider in *providers {
				test_streaming(provider, &path, |response, reporter| {
					let message_id = conversion::bedrock::message_id(&response);
					match *provider {
						BEDROCK_TO_COMPLETIONS => response.map(|body| {
							conversion::bedrock::from_completions::translate_stream(
								body,
								BUFFER_LIMIT,
								reporter,
								"input-model",
								&message_id,
								LOG_CONTENT,
								None,
							)
						}),
						BEDROCK_TO_MESSAGES => response.map(|body| {
							conversion::bedrock::from_messages::translate_stream(
								body,
								BUFFER_LIMIT,
								reporter,
								"input-model",
								&message_id,
								LOG_CONTENT,
								None,
							)
						}),
						BEDROCK_TO_RESPONSES => response.map(|body| {
							conversion::bedrock::from_responses::translate_stream(
								body,
								BUFFER_LIMIT,
								reporter,
								"input-model",
								&message_id,
								LOG_CONTENT,
								None,
							)
						}),
						_ => unreachable!(),
					}
				})
				.await;
			}
		}

		for (name, providers) in ANTHROPIC_STREAM_RESPONSES {
			let path = format!("response/anthropic/{name}.json");
			for provider in *providers {
				test_streaming(provider, &path, |response, reporter| match *provider {
					MESSAGES_TO_MESSAGES => response.map(|body| {
						conversion::messages::passthrough_stream(body, BUFFER_LIMIT, reporter, LOG_CONTENT)
					}),
					MESSAGES_TO_COMPLETIONS => response.map(|body| {
						conversion::messages::from_completions::translate_stream(
							body,
							BUFFER_LIMIT,
							reporter,
							LOG_CONTENT,
						)
					}),
					MESSAGES_TO_DETECT => types::detect::passthrough_stream(reporter, response),
					_ => unreachable!(),
				})
				.await;
			}
		}

		for (name, providers) in COMPLETIONS_STREAM_RESPONSES {
			let path = format!("response/completions/{name}.json");
			for provider in *providers {
				test_streaming(provider, &path, |response, reporter| match *provider {
					COMPLETIONS_TO_COMPLETIONS => {
						conversion::completions::passthrough_stream(reporter, LOG_CONTENT, response)
					},
					COMPLETIONS_TO_MESSAGES => response.map(|body| {
						conversion::completions::from_messages::translate_stream(
							body,
							BUFFER_LIMIT,
							reporter,
							LOG_CONTENT,
						)
					}),
					COMPLETIONS_TO_RESPONSES => response.map(|body| {
						conversion::openai_compat::to_responses::translate_stream(
							body,
							BUFFER_LIMIT,
							reporter,
							LOG_CONTENT,
						)
					}),
					COMPLETIONS_TO_DETECT => types::detect::passthrough_stream(reporter, response),
					_ => unreachable!(),
				})
				.await;
			}
		}

		for name in VERTEX_GEMINI_STREAM_RESPONSES {
			let path = format!("response/vertex-gemini/{name}.json");
			test_streaming(VERTEX_GEMINI_TO_COMPLETIONS, &path, |response, reporter| {
				response.map(|body| {
					conversion::vertex_gemini::to_completions::translate_stream(
						body,
						BUFFER_LIMIT,
						strng::literal!("input-model"),
						reporter,
						LOG_CONTENT,
					)
				})
			})
			.await;
		}

		for (name, providers) in RESPONSES_STREAM_RESPONSES {
			let path = format!("response/responses/{name}.json");
			for provider in *providers {
				test_streaming(provider, &path, |response, reporter| match *provider {
					RESPONSES_TO_RESPONSES => response.map(|body| {
						conversion::responses::passthrough_stream(body, BUFFER_LIMIT, reporter, LOG_CONTENT)
					}),
					RESPONSES_TO_DETECT => types::detect::passthrough_stream(reporter, response),
					_ => unreachable!(),
				})
				.await;
			}
		}

		for (path, provider) in DETECT_RESPONSES {
			test_streaming(provider, path, |response, reporter| match *provider {
				BEDROCK_TO_DETECT => types::detect::passthrough_aws_stream(reporter, response),
				COMPLETIONS_TO_DETECT => types::detect::passthrough_stream(reporter, response),
				other => panic!("unsupported provider in DETECT_RESPONSES: {other}"),
			})
			.await;
		}
	}

	#[tokio::test]
	async fn completions_to_messages_stream_preserves_cache_usage() {
		let input = r#"data: {"id":"chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"model":"gpt-5","usage":{"prompt_tokens":100,"completion_tokens":5,"total_tokens":105,"prompt_tokens_details":{"cached_tokens":20,"cache_write_tokens":30}}}

data: [DONE]

"#;
		let output = conversion::completions::from_messages::translate_stream(
			axum_core::body::Body::from(input),
			1024 * 1024,
			StreamingUsageGuard::default(),
			crate::LogContentFields::default(),
		)
		.collect()
		.await
		.unwrap()
		.to_bytes();
		let delta = String::from_utf8(output.to_vec())
			.unwrap()
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.filter_map(|data| serde_json::from_str::<Value>(data).ok())
			.find(|event| event["type"] == "message_delta")
			.unwrap();

		assert_eq!(
			delta["usage"],
			json!({
				"input_tokens": 50,
				"output_tokens": 5,
				"cache_creation_input_tokens": 30,
				"cache_read_input_tokens": 20,
			})
		);
	}

	#[test]
	fn responses_usage_allows_missing_cache_write_tokens() {
		let event = serde_json::from_value::<types::responses::typed::ResponseStreamEvent>(json!({
			"type": "response.completed",
			"sequence_number": 1,
			"response": {
				"created_at": 1,
				"id": "response",
				"model": "gpt-5",
				"object": "response",
				"output": [],
				"status": "completed",
				"usage": {
					"input_tokens": 10,
					"input_tokens_details": {
						"cached_tokens": 4
					},
					"output_tokens": 2,
					"output_tokens_details": {
						"reasoning_tokens": 0
					},
					"total_tokens": 12
				}
			}
		}))
		.unwrap();

		let types::responses::typed::ResponseStreamEvent::ResponseCompleted(completed) = event else {
			panic!("expected response.completed");
		};
		assert_eq!(
			completed
				.response
				.usage
				.unwrap()
				.input_tokens_details
				.cache_write_tokens,
			None
		);
	}
}
