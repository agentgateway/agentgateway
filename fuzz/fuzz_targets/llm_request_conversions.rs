#![no_main]

use agentgateway::llm::bedrock::Provider as BedrockProvider;
use agentgateway::llm::{conversion, types};
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;

static BEDROCK_PROVIDER: Lazy<BedrockProvider> = Lazy::new(|| {
	serde_json::from_str(
		r#"{"model":"anthropic.claude-3-5-sonnet-20240620-v1:0","region":"us-east-1"}"#,
	)
	.expect("static Bedrock provider should deserialize")
});

fuzz_target!(|data: &[u8]| {
	if data.len() > 64 * 1024 {
		return;
	}

	if let Ok(req) = serde_json::from_slice::<types::completions::Request>(data) {
		let _ = conversion::messages::from_completions::translate(&req);
		let _ = conversion::bedrock::from_completions::translate(&req, &BEDROCK_PROVIDER, None, None);
	}

	if let Ok(req) = serde_json::from_slice::<types::messages::Request>(data) {
		let _ = conversion::completions::from_messages::translate(&req);
		let _ = conversion::bedrock::from_messages::translate(&req, &BEDROCK_PROVIDER, None);
	}

	if let Ok(req) = serde_json::from_slice::<types::responses::Request>(data) {
		let _ = conversion::openai_compat::from_responses::translate(&req);
	}
});
