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
	let provider_value = serde_json::from_str::<Value>(provider_str)
		.unwrap_or_else(|_| Value::String(provider_str.to_string()));

	let resp = xlate(Bytes::copy_from_slice(provider_str.as_bytes()))
		.expect("Failed to translate provider response to expected format");
	let llm_response = resp.to_llm_response(false);
	let raw = resp.serialize().expect("Failed to serialize response");
	let resp_val = serde_json::from_slice::<Value>(&raw)
		.unwrap_or_else(|_| Value::String(provider_str.to_string()));
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

async fn test_streaming(
	provider: &str,
	relative_path: &str,
	xlate: impl AsyncFnOnce(Response, AsyncLog<llm::LLMInfo>) -> Result<Response, AIError>,
) {
	let input_path = fixture_path(relative_path);
	let input_bytes =
		&fs::read(&input_path).unwrap_or_else(|_| panic!("{relative_path}: Failed to read input file"));
	let body = Body::from(input_bytes.clone());
	let log = AsyncLog::default();
	let log2 = log.clone();
	let mut resp = Response::new(body);
	resp.headers_mut().insert(
		crate::http::x_headers::X_AMZN_REQUESTID,
		"request_id".try_into().unwrap(),
	);
	let resp = xlate(resp, log).await.expect("failed to translate stream");
	let resp_bytes = resp.collect().await.unwrap().to_bytes();
	let llm_response = log2.take().unwrap().response;
	let llm_resp_str = serde_json::to_string_pretty(&llm_response).unwrap();
	let resp_str = String::from_utf8(resp_bytes.to_vec()).unwrap() + "\n\n" + llm_resp_str.as_str();
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
const GEMINI: &str = "gemini";
const COMPLETIONS: &str = "completions";
const BEDROCK_TITAN: &str = "bedrock-titan";
const BEDROCK_COHERE: &str = "bedrock-cohere";

mod requests {
	use super::*;

	const COMPLETION_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK]),
		("full", &[ANTHROPIC, BEDROCK]),
		("tool-call", &[ANTHROPIC, BEDROCK]),
		("parallel-tool-call", &[BEDROCK]),
		("reasoning", &[ANTHROPIC, BEDROCK]),
		("reasoning_max", &[ANTHROPIC]),
	];
	const MESSAGES_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[COMPLETIONS, BEDROCK, VERTEX]),
		("tools", &[COMPLETIONS, BEDROCK, VERTEX]),
		("reasoning", &[COMPLETIONS, BEDROCK, VERTEX]),
	];
	const RESPONSES_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[BEDROCK, GEMINI]),
		("instructions", &[BEDROCK, GEMINI]),
		("input-list", &[BEDROCK, GEMINI]),
		("parallel-tool-call", &[BEDROCK, GEMINI]),
	];
	pub const COUNT_TOKENS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[ANTHROPIC, BEDROCK, VERTEX]),
		("with_system", &[ANTHROPIC, BEDROCK, VERTEX]),
	];
	const EMBEDDINGS_REQUESTS: &[(&str, &[&str])] = &[
		("basic", &[OPENAI, BEDROCK_TITAN, BEDROCK_COHERE, VERTEX]),
		("array", &[OPENAI, BEDROCK_COHERE, VERTEX]),
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

		for (name, providers) in RESPONSES_REQUESTS {
			let test = &format!("requests/responses/{name}.json");
			for provider in *providers {
				match *provider {
					BEDROCK => test_request(BEDROCK, test, |req| {
						conversion::bedrock::from_responses::translate(&req, &bedrock_provider, None, None)
					}),
					GEMINI => test_request(GEMINI, test, |req| {
						conversion::gemini::from_responses::translate(&req)
					}),
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
		let openai_request = |i: types::embeddings::Request| i.to_openai();
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
					OPENAI => {
						test_request(
							OPENAI,
							&format!("requests/embeddings/{name}.json"),
							openai_request,
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

	// <response from provider> --> <response to user>
	const COMPLETIONS_TO_COMPLETIONS: &str = "completions-completions";
	const MESSAGES_TO_MESSAGES: &str = "messages-messages";
	const MESSAGES_TO_COMPLETIONS: &str = "messages-completions";
	const MESSAGES_TO_DETECT: &str = "messages-detect";
	const COMPLETIONS_TO_MESSAGES: &str = "completions-messages";
	const COMPLETIONS_TO_DETECT: &str = "completions-detect";
	const BEDROCK_TO_MESSAGES: &str = "bedrock-messages";
	const BEDROCK_TO_COMPLETIONS: &str = "bedrock-completions";
	const BEDROCK_TO_RESPONSES: &str = "bedrock-responses";
	const RESPONSES_TO_RESPONSES: &str = "responses-responses";
	const RESPONSES_TO_DETECT: &str = "responses-detect";

	const ALL_BEDROCK: &[&str] = &[
		BEDROCK_TO_MESSAGES,
		BEDROCK_TO_COMPLETIONS,
		BEDROCK_TO_RESPONSES,
	];
	const BEDROCK_RESPONSES: &[(&str, &[&str])] = &[("basic", ALL_BEDROCK), ("tool", ALL_BEDROCK)];
	const BEDROCK_STREAM_RESPONSES: &[(&str, &[&str])] =
		&[("basic", ALL_BEDROCK), ("tool", ALL_BEDROCK)];

	const ALL_ANTHROPIC: &[&str] = &[
		MESSAGES_TO_MESSAGES,
		MESSAGES_TO_COMPLETIONS,
		MESSAGES_TO_DETECT,
	];
	const ANTHROPIC_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_ANTHROPIC),
		("tool", ALL_ANTHROPIC),
		("thinking", ALL_ANTHROPIC),
	];
	const ANTHROPIC_STREAM_RESPONSES: &[(&str, &[&str])] = &[
		("stream_basic", ALL_ANTHROPIC),
		("stream_thinking", ALL_ANTHROPIC),
	];

	const ALL_COMPLETIONS: &[&str] = &[
		COMPLETIONS_TO_COMPLETIONS,
		COMPLETIONS_TO_MESSAGES,
		COMPLETIONS_TO_DETECT,
	];
	const COMPLETIONS_RESPONSES: &[(&str, &[&str])] = &[
		("basic", ALL_COMPLETIONS),
		("audio", ALL_COMPLETIONS),
		("openrouter_reasoning", ALL_COMPLETIONS),
		("gemini_zero_completion_tokens", ALL_COMPLETIONS),
		("gemini_with_completion_tokens", ALL_COMPLETIONS),
	];
	const COMPLETIONS_STREAM_RESPONSES: &[(&str, &[&str])] = &[("stream", ALL_COMPLETIONS)];

	const EMBEDDING_RESPONSES: &[(&str, &[&str])] = &[
		("response/bedrock-titan/embeddings.json", &[BEDROCK_TITAN]),
		("response/bedrock-cohere/embeddings.json", &[BEDROCK_COHERE]),
		("response/vertex/embeddings.json", &[VERTEX]),
		("response/openai/embeddings.json", &[OPENAI]),
	];
	const COUNT_TOKEN_RESPONSES: &[(&str, &[&str])] = &[("count_tokens", &[ANTHROPIC])];

	const ALL_RESPONSES: &[&str] = &[RESPONSES_TO_RESPONSES, RESPONSES_TO_DETECT];
	const RESPONSES_RESPONSES: &[(&str, &[&str])] = &[("basic", ALL_RESPONSES)];
	const RESPONSES_STREAM_RESPONSES: &[(&str, &[&str])] =
		&[("stream", ALL_RESPONSES), ("stream-image", ALL_RESPONSES)];

	const DETECT_RESPONSES: &[(&str, &[&str])] = &[
		("non-json", &[COMPLETIONS_TO_DETECT]),
		("broken-sse", &[COMPLETIONS_TO_DETECT]),
		("stream-image-generation", &[COMPLETIONS_TO_DETECT]),
	];

	#[tokio::test]
	async fn from_bedrock() {
		for (name, providers) in BEDROCK_RESPONSES {
			let test = &format!("response/bedrock/{name}.json");
			for provider in *providers {
				test_response_for_provider(provider, test)
			}
		}
		for (name, providers) in BEDROCK_STREAM_RESPONSES {
			let test = &format!("response/bedrock/{name}.bin");
			for provider in *providers {
				test_streaming_response_for_provider(provider, test).await
			}
		}
	}

	#[tokio::test]
	async fn from_anthropic() {
		for (name, providers) in ANTHROPIC_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				test_response_for_provider(provider, test)
			}
		}

		for (name, providers) in ANTHROPIC_STREAM_RESPONSES {
			let test = &format!("response/anthropic/{name}.json");
			for provider in *providers {
				test_streaming_response_for_provider(provider, test).await
			}
		}
	}

	#[tokio::test]
	async fn from_completions() {
		for (name, providers) in COMPLETIONS_RESPONSES {
			let test = &format!("response/completions/{name}.json");
			for provider in *providers {
				test_response_for_provider(provider, test)
			}
		}

		for (name, providers) in COMPLETIONS_STREAM_RESPONSES {
			let test = &format!("response/completions/{name}.json");
			for provider in *providers {
				test_streaming_response_for_provider(provider, test).await
			}
		}
	}

	#[tokio::test]
	async fn from_responses() {
		for (name, providers) in RESPONSES_RESPONSES {
			let test = &format!("response/responses/{name}.json");
			for provider in *providers {
				test_response_for_provider(provider, test)
			}
		}

		for (name, providers) in RESPONSES_STREAM_RESPONSES {
			let test = &format!("response/responses/{name}.json");
			for provider in *providers {
				test_streaming_response_for_provider(provider, test).await
			}
		}
	}

	#[tokio::test]
	async fn detect() {
		for (name, providers) in DETECT_RESPONSES {
			let test = &format!("response/detect/{name}");
			for provider in *providers {
				// Test each one as a stream and not
				test_response_for_provider(provider, test);
				test_streaming_response_for_provider(provider, test).await
			}
		}
	}

	fn test_response_for_provider(provider: &str, test: &str) {
		let (p, r) = build_provider_request(provider);
		let test_fn = |i: Bytes| p.process_success(&r, &i);
		test_response(provider, test, test_fn)
	}

	async fn test_streaming_response_for_provider(provider: &str, test: &str) {
		let (p, r) = build_provider_request(provider);
		let test_fn = async |i: Response, log: AsyncLog<llm::LLMInfo>| {
			p.process_streaming(r, LLMResponsePolicies::default(), log, false, i)
				.await
		};
		test_streaming(provider, test, test_fn).await
	}

	fn build_provider_request(provider: &str) -> (AIProvider, LLMRequest) {
		let bedrock_provider = AIProvider::Bedrock(bedrock::Provider {
			model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
			region: strng::new("us-west-2"),
			guardrail_identifier: None,
			guardrail_version: None,
		});
		let (p, r) = match provider {
			RESPONSES_TO_RESPONSES => (
				AIProvider::OpenAI(openai::Provider { model: None }),
				dummy_llm_req(InputFormat::Responses),
			),
			COMPLETIONS_TO_COMPLETIONS => (
				AIProvider::OpenAI(openai::Provider { model: None }),
				dummy_llm_req(InputFormat::Completions),
			),
			COMPLETIONS_TO_MESSAGES => (
				AIProvider::OpenAI(openai::Provider { model: None }),
				dummy_llm_req(InputFormat::Messages),
			),
			MESSAGES_TO_MESSAGES => (
				AIProvider::Anthropic(anthropic::Provider { model: None }),
				dummy_llm_req(InputFormat::Messages),
			),
			MESSAGES_TO_COMPLETIONS => (
				AIProvider::Anthropic(anthropic::Provider { model: None }),
				dummy_llm_req(InputFormat::Completions),
			),
			BEDROCK_TO_MESSAGES => (bedrock_provider, dummy_llm_req(InputFormat::Messages)),
			BEDROCK_TO_COMPLETIONS => (bedrock_provider, dummy_llm_req(InputFormat::Completions)),
			BEDROCK_TO_RESPONSES => (bedrock_provider, dummy_llm_req(InputFormat::Responses)),
			COMPLETIONS_TO_DETECT => (
				AIProvider::OpenAI(openai::Provider { model: None }),
				dummy_llm_req(InputFormat::Detect),
			),
			MESSAGES_TO_DETECT => (
				AIProvider::Anthropic(anthropic::Provider { model: None }),
				dummy_llm_req(InputFormat::Detect),
			),
			RESPONSES_TO_DETECT => (
				AIProvider::OpenAI(openai::Provider { model: None }),
				dummy_llm_req(InputFormat::Detect),
			),
			// No other ones are supported.
			// We do not have Responses<-->Completions
			other => panic!("unsupported provider for responses: {other}"),
		};
		(p, r)
	}

	pub fn dummy_llm_req(input_format: InputFormat) -> LLMRequest {
		LLMRequest {
			input_tokens: None,
			input_format,
			request_model: "input-model".into(),
			provider: Default::default(),
			streaming: false,
			params: Default::default(),
			prompt: None,
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
		let openai = |i: Bytes| {
			serde_json::from_slice::<types::embeddings::Response>(&i)
				.map(|e| Box::new(e) as Box<dyn ResponseType>)
				.map_err(AIError::ResponseParsing)
		};

		for (test, providers) in EMBEDDING_RESPONSES {
			for provider in *providers {
				match *provider {
					BEDROCK_TITAN => test_response(BEDROCK_TITAN, test, titan),
					BEDROCK_COHERE => test_response(BEDROCK_COHERE, test, cohere),
					VERTEX => test_response(VERTEX, test, vertex),
					OPENAI => test_response(OPENAI, test, openai),
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
fn test_get_messages() {
	use crate::llm::types::RequestType;

	fn extract_messages<R: RequestType + DeserializeOwned>(fixture: &str, provider: &str) {
		let path = fixture_path(fixture);
		let input_str = fs::read_to_string(&path).expect("Failed to read input file");
		let raw: Value = serde_json::from_str(&input_str).expect("Failed to parse input json");
		let request: R = serde_json::from_str(&input_str).expect("Failed to parse json");

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

/// Verifies that `process_response` routes a non-success response through
/// the buffered error path even when the request has `streaming: true`.
///
/// Constructs a Bedrock 400 JSON error response and passes it through
/// `process_response` with a streaming `LLMRequest`. Asserts the returned
/// body is non-empty, valid JSON, and preserves the original error message.
#[tokio::test]
async fn process_response_routes_streaming_error_to_buffered_path() {
	use crate::proxy::httpproxy::PolicyClient;
	use crate::test_helpers::proxymock::setup_proxy_test;

	let bedrock = AIProvider::Bedrock(bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	});

	let error_json = r#"{"message":"Expected toolResult blocks at messages.2.content for the following Ids: tooluse_abc123"}"#;

	let req = LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Completions,
		request_model: "input-model".into(),
		provider: Default::default(),
		streaming: true,
		params: Default::default(),
		prompt: None,
	};

	let body = Body::from(error_json.as_bytes().to_vec());
	let mut resp = Response::new(body);
	*resp.status_mut() = ::http::StatusCode::BAD_REQUEST;
	resp.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"application/json".parse().unwrap(),
	);

	let client = PolicyClient {
		inputs: setup_proxy_test("{}").unwrap().pi,
	};

	let result = bedrock
		.process_response(
			client,
			req,
			LLMResponsePolicies::default(),
			AsyncLog::default(),
			false,
			resp,
		)
		.await
		.expect("process_response should succeed for error responses");

	assert_eq!(result.status(), ::http::StatusCode::BAD_REQUEST);

	let result_body = result.collect().await.unwrap().to_bytes();
	assert!(
		!result_body.is_empty(),
		"error response body must not be empty",
	);

	let parsed: Value =
		serde_json::from_slice(&result_body).expect("translated error should be valid JSON");

	let message = parsed
		.pointer("/error/message")
		.and_then(|v| v.as_str())
		.unwrap_or_default();
	assert!(
		message.contains("toolResult"),
		"translated error should preserve the original message, got: {message}",
	);
}

#[tokio::test]
async fn process_streaming_bedrock_completions_normalizes_sse_headers_and_done() {
	let bedrock = AIProvider::Bedrock(bedrock::Provider {
		model: Some(strng::new("openai.gpt-oss-120b-1:0")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	});

	let body = Body::from(
		fs::read(fixture_path("response/bedrock/basic.bin"))
			.expect("failed to read Bedrock streaming fixture"),
	);
	let mut resp = Response::new(body);
	resp.headers_mut().insert(
		::http::header::CONTENT_TYPE,
		"application/vnd.amazon.eventstream".parse().unwrap(),
	);
	resp.headers_mut().insert(
		crate::http::x_headers::X_AMZN_REQUESTID,
		"request_id".parse().unwrap(),
	);

	let translated = bedrock
		.process_streaming(
			LLMRequest {
				input_tokens: None,
				input_format: InputFormat::Completions,
				request_model: "input-model".into(),
				provider: Default::default(),
				streaming: true,
				params: Default::default(),
				prompt: None,
			},
			LLMResponsePolicies::default(),
			AsyncLog::default(),
			false,
			resp,
		)
		.await
		.expect("Bedrock streaming translation should succeed");

	crate::http::tests_common::assert_header(
		&translated,
		::http::header::CONTENT_TYPE,
		"text/event-stream",
	);

	let body = translated.collect().await.unwrap().to_bytes();
	let text = String::from_utf8(body.to_vec()).expect("stream should be valid UTF-8");
	assert!(
		text.ends_with("data: [DONE]\n\n"),
		"translated Bedrock completions stream must end with [DONE], got:\n{text}",
	);
	assert!(
		!text.contains("event: \n"),
		"translated Bedrock completions stream must not emit empty event fields:\n{text}",
	);
}

#[test]
fn setup_request_openai_applies_prefixed_path_without_host_override() {
	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			None,
			None,
			Some("/v1/custom"),
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(
		req.uri().authority().map(|a| a.as_str()),
		Some("api.openai.com")
	);
	assert_eq!(req.uri().path(), "/v1/custom/chat/completions");
	assert_eq!(req.uri().query(), Some("trace=repro"));
}

#[test]
fn setup_request_openai_normalizes_trailing_slash_in_path_prefix() {
	let provider = AIProvider::OpenAI(openai::Provider { model: None });
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/messages?trace=repro",
		http::Method::POST,
		&[],
	);

	provider
		.setup_request(
			&mut req,
			RouteType::Messages,
			None,
			None,
			Some("/v1/custom/"),
			false,
		)
		.expect("setup_request should succeed");

	assert_eq!(req.uri().path(), "/v1/custom/chat/completions");
	assert_eq!(req.uri().query(), Some("trace=repro"));
}

#[test]
fn completions_response_missing_message_and_usage_fields() {
	// Gemini's OpenAI-compat endpoint can omit `message` from choices and
	// `completion_tokens` from usage. Verify deserialization succeeds with defaults.
	let json = r#"{
		"id": "1",
		"object": "chat.completion",
		"created": 0,
		"model": "google/gemini-2.5-flash",
		"choices": [{"index": 0, "finish_reason": "length"}],
		"usage": {"prompt_tokens": 5, "total_tokens": 12}
	}"#;
	let resp: types::completions::Response = serde_json::from_str(json).unwrap();
	assert_eq!(resp.choices.len(), 1);
	assert_eq!(resp.choices[0].message.content, None);
	assert_eq!(resp.choices[0].message.role, None);
	let usage = resp.usage.unwrap();
	assert_eq!(usage.prompt_tokens, 5);
	assert_eq!(usage.completion_tokens, 0);
	assert_eq!(usage.total_tokens, 12);
}
