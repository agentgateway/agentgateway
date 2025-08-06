use std::fs;
use std::path::Path;

use agent_core::strng;
use serde::de::DeserializeOwned;

use super::*;

fn test_response<T: DeserializeOwned>(
	test_name: &str,
	xlate: impl Fn(T) -> Result<universal::ChatCompletionResponse, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let provider_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let provider_raw: Value =
		serde_json::from_str(provider_str).expect("Failed to parse provider json");
	let provider: T = serde_json::from_str(provider_str).expect("Failed to parse provider JSON");

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
	xlate: impl Fn(universal::ChatCompletionRequest) -> Result<T, AIError>,
) {
	let test_dir = Path::new("src/llm/tests");

	// Read input JSON
	let input_path = test_dir.join(format!("{test_name}.json"));
	let openai_str = &fs::read_to_string(&input_path).expect("Failed to read input file");
	let openai_raw: Value = serde_json::from_str(openai_str).expect("Failed to parse openai json");
	let openai: universal::ChatCompletionRequest =
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

#[test]
fn test_bedrock() {
	let response = |i| bedrock::translate_response(i, &strng::new("fake-model"));
	test_response::<bedrock::types::ConverseResponse>("basic_bedrock", response);
	let provider = bedrock::Provider {
		model: Some(strng::new("test-model")),
		region: strng::new("us-east-1"),
		guardrail_identifier: None,
		guardrail_version: None,
	};
	let request = |i| Ok(bedrock::translate_request(i, &provider));
	test_request("bedrock", "basic_input", request);
	test_request("bedrock", "full_input", request);
}

#[test]
fn test_anthropic() {
	let response = |i| Ok(anthropic::translate_response(i));
	test_response::<anthropic::types::MessagesResponse>("basic_anthropic", response);

	let request = |i| Ok(anthropic::translate_request(i));
	test_request("anthropic", "basic_input", request);
	test_request("anthropic", "full_input", request);
}
