use std::fs;
use std::path::Path;

use agent_core::strng;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::Value;

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
	xlate: impl Fn(Body, AsyncLog<LLMInfo>) -> Result<Body, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(test_name);
	let provider =
		&fs::read(&input_path).unwrap_or_else(|_| panic!("{test_name}: Failed to read input file"));
	let body = Body::from(provider.clone());
	let log = AsyncLog::default();
	let resp = xlate(body, log).expect("failed to translate stream");
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
const MESSAGES_REQUESTS: &[&str] = &["request_anthropic_basic", "request_anthropic_tools"];
const RESPONSES_REQUESTS: &[&str] = &["request_responses_basic", "request_responses_instructions"];

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
	test_request("anthropic", "request_anthropic_basic", request);
	test_request("anthropic", "request_anthropic_tools", request);
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
