use super::*;
use agent_core::strng;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::path::Path;

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
	xlate: impl Fn(Body, AsyncLog<LLMResponse>) -> Result<Body, AIError>,
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
				("\"created\":[0-9]*","\"created\":123")
			]
	}, {
			 insta::assert_snapshot!(test_name, resp_str);
	});
}

fn test_request<T: Serialize>(
	provider_name: &str,
	test_name: &str,
	xlate: impl Fn(universal::Request) -> Result<T, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse openai json");
	let openai: universal::Request =
		serde_json::from_str(openai_str).expect("Failed to parse openai JSON");

	let provider_response =
		xlate(openai).expect("Failed to translate OpenAI format to provider request ");

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
	let response = |i| bedrock::translate_response(i, &strng::new("fake-model"));
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_basic", response);
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_tool", response);

	let stream_response = |i, log| {
		Ok(bedrock::translate_stream(
			i,
			log,
			"model".to_string(),
			"request-id".to_string(),
		))
	};
	test_streaming("response_stream-bedrock_basic.bin", stream_response).await;

	let provider = bedrock::Provider {
		model: Some(strng::new("test-model")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let request = |i| Ok(bedrock::translate_request(i, &provider));
	for r in ALL_REQUESTS {
		test_request("bedrock", r, request);
	}
}

#[tokio::test]
async fn test_anthropic() {
	let response = |i| Ok(anthropic::translate_response(i));
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_basic", response);
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_tool", response);
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_thinking", response);

	let stream_response = |i, log| Ok(anthropic::translate_stream(i, log));
	test_streaming("response_stream-anthropic_basic.json", stream_response).await;
	test_streaming("response_stream-anthropic_thinking.json", stream_response).await;

	let request = |i| Ok(anthropic::translate_request(i));
	for r in ALL_REQUESTS {
		test_request("anthropic", r, request);
	}
}
