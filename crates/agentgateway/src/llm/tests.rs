use std::fs;
use std::path::Path;

use agent_core::strng;
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

const ALL_REQUESTS: &[&str] = &["request_basic", "request_full", "request_tool-call"];

#[test]
fn test_bedrock() {
	let response = |i| bedrock::translate_response(i, &strng::new("fake-model"));
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_basic", response);
	test_response::<bedrock::types::ConverseResponse>("response_bedrock_tool", response);
	let provider = bedrock::Provider {
		model: Some(strng::new("test-model")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let request = |i| Ok(bedrock::translate_openai_request(i, &provider, "test-model"));
	for r in ALL_REQUESTS {
		test_request("bedrock", r, request);
	}
}

#[test]
fn test_anthropic() {
	let response = |i| Ok(anthropic::translate_response(i));
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_basic", response);
	test_response::<anthropic::types::MessagesResponse>("response_anthropic_tool", response);

	let request = |i| Ok(anthropic::translate_request(i));
	for r in ALL_REQUESTS {
		test_request("anthropic", r, request);
	}
}
