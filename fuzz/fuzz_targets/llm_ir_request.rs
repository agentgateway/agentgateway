#![no_main]

//! Converting any request body never panics, a converted body is JSON, the one-pass and
//! two-pass parsers agree, and the model pre-read finds the model a full JSON read finds.

#[path = "llm_ir_common.rs"]
mod common;

use std::num::NonZeroU64;

use agent_llm_ir::{
	ClientEndpoint, Delivery, RenderSettings, Upstream, UpstreamProtocol, prepare, prompt, read_model,
};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
	if data.len() > 64 * 1024 {
		return;
	}
	// The pre-read checks values other than `model` only as JSON, so a full read can still
	// refuse a body it accepts; when both read it, they find the same model.
	if let (Ok(model), Ok(value)) = (read_model(data), serde_json::from_slice::<Value>(data)) {
		assert_eq!(model.as_deref(), value["model"].as_str());
	}
	let responses = Upstream {
		identifier_pseudonymizer: &common::Hashed,
		scope: common::SCOPE,
		protocol: UpstreamProtocol::Responses {
			safety_identifier: true,
			reasoning_context: false,
		},
		default_max_tokens: None,
	};
	let messages = Upstream {
		identifier_pseudonymizer: &common::Hashed,
		scope: common::SCOPE,
		protocol: UpstreamProtocol::Messages,
		default_max_tokens: NonZeroU64::new(4096),
	};
	let chat = Upstream {
		identifier_pseudonymizer: &common::Hashed,
		scope: common::SCOPE,
		protocol: UpstreamProtocol::ChatCompletions,
		default_max_tokens: NonZeroU64::new(4096),
	};
	let gemini = Upstream {
		identifier_pseudonymizer: &common::Hashed,
		scope: common::SCOPE,
		protocol: UpstreamProtocol::Gemini,
		default_max_tokens: NonZeroU64::new(4096),
	};
	for (client, upstream) in [
		(ClientEndpoint::Messages, responses),
		(ClientEndpoint::Responses, messages),
		(ClientEndpoint::Messages, gemini),
		(ClientEndpoint::Responses, gemini),
		(ClientEndpoint::Gemini(Delivery::Buffered), messages),
		(ClientEndpoint::Gemini(Delivery::Streaming), responses),
		(ClientEndpoint::Gemini(Delivery::Buffered), chat),
		(ClientEndpoint::Messages, chat),
		(ClientEndpoint::Responses, chat),
		(ClientEndpoint::ChatCompletions, messages),
		(ClientEndpoint::ChatCompletions, responses),
		(ClientEndpoint::ChatCompletions, gemini),
	] {
		if let Ok(converted) = prepare(data, client, upstream)
			.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		{
			common::assert_json_body(converted.body.as_bytes());
		}
		assert!(prompt::one_pass_parse_agrees(data, client, upstream));
	}
});
