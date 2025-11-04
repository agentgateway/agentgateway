use std::fs;
use std::path::Path;

use agent_core::strng;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::*;

fn test_response<T: DeserializeOwned>(
	test_name: &str,
	xlate: impl Fn(T) -> Result<universal::Response, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let provider_str = &fs::read_to_string(&input_path)
		.unwrap_or_else(|_| panic!("{test_name}: Failed to read input file"));
	let provider_raw: Value = serde_json_path_to_error::from_str(provider_str)
		.unwrap_or_else(|_| panic!("{test_name}: Failed to parse provider json"));
	let provider: T = serde_json_path_to_error::from_str(provider_str)
		.unwrap_or_else(|_| panic!("{test_name}: Failed to parse provider JSON"));

	let openai_response =
		xlate(provider).expect("Failed to translate provider response to OpenAI format");

	insta::with_settings!({
			info => &provider_raw,
			description => input_path.to_string_lossy().to_string(),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
	}, {
			 insta::assert_json_snapshot!(test_name, openai_response, {
			".id" => "[id]",
			".created" => "[date]",
		});
	});
}

async fn test_streaming(
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
			 insta::assert_snapshot!(test_name, resp_str);
	});
}

fn test_request<I, O>(provider_name: &str, test_name: &str, xlate: impl Fn(I) -> Result<O, AIError>)
where
	I: DeserializeOwned,
	O: Serialize,
{
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse input json");
	let openai: I = serde_json::from_str(openai_str).expect("Failed to parse input JSON");

	let provider_response =
		xlate(openai).expect("Failed to translate input format to provider request ");

	insta::with_settings!({
			info => &openai_raw,
			description => format!("{}: {}", provider_name, test_name),
			omit_expression => true,
			prepend_module_to_snapshot => false,
			snapshot_path => "tests",
	}, {
			 insta::assert_json_snapshot!(format!("{}-{}", provider_name, test_name), provider_response, {
			".id" => "[id]",
			".created" => "[date]",
		});
	});
}

const ALL_REQUESTS: &[&str] = &[
	"request_basic",
	"request_full",
	"request_tool-call",
	"request_reasoning",
];

#[test]
fn test_openai() {
	let response = |i| Ok(i);
	test_response::<universal::Response>("response_basic", response);
	test_response::<universal::Response>("response_reasoning_openrouter", response);
}

#[tokio::test]
async fn test_bedrock() {
	let provider = bedrock::Provider {
		model: Some(strng::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
		region: strng::new("us-west-2"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let test_dir = Path::new("src/llm/tests");

	// ===== Completions ↔ Bedrock =====
	let response = |i| bedrock::translate_response_to_completions(i, &strng::new("fake-model"));
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_basic", response);
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_tool", response);

	let stream_response = |i, log| {
		Ok(bedrock::translate_stream_to_completions(
			i,
			log,
			"model".to_string(),
			"request-id".to_string(),
		))
	};
	test_streaming("response_stream-bedrock_basic.bin", stream_response).await;

	let request = |i| {
		Ok(bedrock::translate_request_completions(
			i, &provider, None, None,
		))
	};
	for r in ALL_REQUESTS {
		test_request("bedrock", r, request);
	}

	// ===== Messages ↔ Bedrock =====
	// Test Messages → Bedrock request translation
	let input_path = test_dir.join("request_anthropic_basic.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read request_anthropic_basic.json");
	let req: anthropic::types::MessagesRequest =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_request_messages(req, &provider, None).expect("Translation failed");
	insta::assert_json_snapshot!("anthropic-bedrock-request_anthropic_basic", result);

	// Test Messages → Bedrock with tools
	let input_path = test_dir.join("request_anthropic_tools.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read request_anthropic_tools.json");
	let req: anthropic::types::MessagesRequest =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_request_messages(req, &provider, None).expect("Translation failed");
	insta::assert_json_snapshot!("anthropic-bedrock-request_anthropic_tools", result);

	// Test Bedrock → Messages response translation
	let input_path = test_dir.join("response_bedrock_basic.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read response_bedrock_basic.json");
	let bedrock_resp: bedrock::types::ConverseResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result = bedrock::translate_response_to_messages(bedrock_resp, "claude-3-5-sonnet-20241022")
		.expect("Translation failed");
	insta::assert_json_snapshot!("anthropic-bedrock-response_bedrock_to_messages_basic", result, {
		".id" => "[id]",
	});

	// Test Bedrock → Messages with tool response
	let input_path = test_dir.join("response_bedrock_tool.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read response_bedrock_tool.json");
	let bedrock_resp: bedrock::types::ConverseResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result = bedrock::translate_response_to_messages(bedrock_resp, "claude-3-5-sonnet-20241022")
		.expect("Translation failed");
	insta::assert_json_snapshot!("anthropic-bedrock-response_bedrock_to_messages_tool", result, {
		".id" => "[id]",
	});

	// Test Bedrock streaming → Messages SSE translation
	let input_path = test_dir.join("response_stream-bedrock_basic.bin");
	let provider_bytes = &fs::read(&input_path).expect("Failed to read streaming fixture");
	let body = Body::from(provider_bytes.clone());
	let log = AsyncLog::default();
	let resp = bedrock::translate_stream_to_messages(
		body,
		log,
		"claude-3-5-sonnet-20241022".to_string(),
		"request-id".to_string(),
	);
	let resp_bytes = resp.collect().await.unwrap().to_bytes();
	let resp_str = std::str::from_utf8(&resp_bytes).unwrap();
	insta::with_settings!({
		description => "Bedrock → Anthropic Messages SSE",
		omit_expression => true,
		prepend_module_to_snapshot => false,
		snapshot_path => "tests",
		filters => vec![
			("\"created\":[0-9]+","\"created\":123"),
			("\"created_at\":[0-9]+","\"created_at\":123"),
			("\"id\":\"msg_[0-9a-f]+\"","\"id\":\"msg_xxx\""),
		]
	}, {
		insta::assert_snapshot!("anthropic-bedrock-stream_basic", resp_str);
	});

	// ===== Responses ↔ Bedrock =====
	// Test Responses → Bedrock request translation
	let input_path = test_dir.join("request_responses_basic.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read request_responses_basic.json");
	let req: openai::responses::CreateResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_request_responses(&req, &provider, None, None).expect("Translation failed");
	insta::assert_json_snapshot!("responses-request_responses_basic", result);

	// Test request with instructions
	let input_path = test_dir.join("request_responses_instructions.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read request_responses_instructions.json");
	let req: openai::responses::CreateResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_request_responses(&req, &provider, None, None).expect("Translation failed");
	insta::assert_json_snapshot!("responses-request_responses_instructions", result);

	// Test Bedrock → Responses response translation
	let input_path = test_dir.join("response_bedrock_basic.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read response_bedrock_basic.json");
	let bedrock_resp: bedrock::types::ConverseResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_response_to_responses(bedrock_resp, "gpt-4o").expect("Translation failed");
	insta::assert_json_snapshot!("responses-response_bedrock_to_responses_basic", result, {
		".id" => "[id]",
		".output[].id" => "[id]",
	});

	// Test with tool response
	let input_path = test_dir.join("response_bedrock_tool.json");
	let json_str =
		fs::read_to_string(&input_path).expect("Failed to read response_bedrock_tool.json");
	let bedrock_resp: bedrock::types::ConverseResponse =
		serde_json::from_str(&json_str).expect("Failed to parse");
	let result =
		bedrock::translate_response_to_responses(bedrock_resp, "gpt-4o").expect("Translation failed");
	insta::assert_json_snapshot!("responses-response_bedrock_to_responses_tool", result, {
		".id" => "[id]",
		".output[].id" => "[id]",
		".output[].call_id" => "[id]",
	});

	// Test Bedrock streaming → Responses SSE translation
	let stream_response = |i, log| {
		Ok(bedrock::translate_stream_to_responses(
			i,
			log,
			"gpt-4o".to_string(),
			"request-id".to_string(),
		))
	};
	test_streaming("response_stream-responses_basic.bin", stream_response).await;
}

#[tokio::test]
async fn test_passthrough() {
	let test_dir = Path::new("src/llm/tests");

	let test_name = "request_full";
	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse input json");
	let openai: universal::passthrough::Request =
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
async fn test_anthropic_to_anthropic() {
	let request = |i| Ok(anthropic::translate_anthropic_request(i));
	test_request::<anthropic::types::MessagesRequest, universal::Request>(
		"anthropic",
		"request_anthropic_basic",
		request,
	);
	test_request::<anthropic::types::MessagesRequest, universal::Request>(
		"anthropic",
		"request_anthropic_tools",
		request,
	);
}

#[tokio::test]
async fn test_anthropic() {
	let response = |i| Ok(anthropic::translate_response(i));
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_basic", response);
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_tool", response);
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_thinking", response);

	let stream_response = |i, log| Ok(anthropic::translate_stream(i, 1024, log));
	test_streaming("response_stream-anthropic_basic.json", stream_response).await;
	test_streaming("response_stream-anthropic_thinking.json", stream_response).await;

	let request = |i| Ok(anthropic::translate_request(i));
	for r in ALL_REQUESTS {
		test_request("anthropic", r, request);
	}
}
