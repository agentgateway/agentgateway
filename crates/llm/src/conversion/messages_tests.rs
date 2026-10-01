//! Native Messages client ↔ Anthropic Messages upstream streaming regression
//! coverage.
//!
//! Provider wire reality (live-verified against the Fireworks
//! Anthropic-compatible `/inference/v1/messages` endpoint, 2026-10-01):
//! `message_start` carries an ALL-ZERO usage placeholder
//! (`{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,
//! "cache_read_input_tokens":0}`) and the real cumulative usage arrives only
//! on the final `message_delta` (`{"input_tokens":91,"output_tokens":16,...}`).
//!
//! The passthrough usage extraction must therefore treat `message_start`
//! usage as a provisional placeholder and end with the `message_delta`
//! values. Ending with the `message_start` zeros fabricates a
//! zero-token-but-complete provider attempt for every such stream — the
//! exact production defect signature (succeeded attempts with
//! input=output=cache_read=cache_write=0 and usage_complete=true).

use std::sync::{Arc, Mutex};

use http_body_util::BodyExt;

use super::passthrough_stream;
use agent_http::Body;
use crate::{
	CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields,
	StreamingUsageGuard, StreamingUsageReporter,
};

struct Capture(Arc<Mutex<LLMInfo>>);

impl StreamingUsageReporter for Capture {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		f(&mut self.0.lock().unwrap());
	}
	fn report_usage(&mut self) {}
}

fn captured_info() -> Arc<Mutex<LLMInfo>> {
	Arc::new(Mutex::new(LLMInfo {
		request: LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Messages,
			cache_convention: CacheTokenConvention::pending(),
			request_model: "accounts/fireworks/models/kimi-k3".into(),
			provider: "custom".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		response: LLMResponse::default(),
	}))
}

/// The exact event sequence the Fireworks Anthropic-compatible endpoint
/// emits for a small kimi-k3 streaming request (field names and zero
/// placeholders byte-faithful; content replaced).
fn fireworks_style_stream() -> String {
	let events = [
		r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"accounts/fireworks/models/kimi-k3","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":null,"service_tier":null,"inference_geo":null}}}"#,
		r#"{"type":"ping"}"#,
		r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
		r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
		r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
		r#"{"type":"content_block_stop","index":0}"#,
		r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"input_tokens":91,"output_tokens":16,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#,
		r#"{"type":"message_stop"}"#,
	];
	events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

async fn run_passthrough(input: String) -> LLMInfo {
	let captured = captured_info();
	let _ = passthrough_stream(
		Body::from(input.into_bytes()),
		1024 * 1024,
		StreamingUsageGuard::new(Box::new(Capture(captured.clone()))),
		LogContentFields {
			completion: true,
			tool_calls: false,
		},
	)
	.collect()
	.await
	.expect("collect stream")
	.to_bytes();
	captured.lock().unwrap().clone()
}

/// `message_delta` usage is cumulative-final: it must override the
/// `message_start` zero placeholder on every dimension.
#[tokio::test]
async fn passthrough_stream_final_usage_overrides_message_start_placeholder() {
	let info = run_passthrough(fireworks_style_stream()).await;
	assert_eq!(
		info.response.input_tokens,
		Some(91),
		"input from message_delta"
	);
	assert_eq!(
		info.response.output_tokens,
		Some(16),
		"output from message_delta"
	);
	assert_eq!(info.response.total_tokens, Some(107));
	assert_eq!(
		info.response.cached_input_tokens,
		Some(0),
		"cache dimensions are present-as-zero on this wire"
	);
	assert_eq!(info.response.cache_creation_input_tokens, Some(0));
	assert_eq!(
		info.response.provider_model.as_deref(),
		Some("accounts/fireworks/models/kimi-k3")
	);
}

/// A stream whose `message_delta` carries NO usage fields must NOT end with
/// the `message_start` zero placeholder either: unknown is never zero. The
/// message_start handler eagerly materializes the non-optional placeholder;
/// the final state must keep the *presence* of evidence, not the
/// placeholder value, when the provider never confirmed usage.
#[tokio::test]
async fn passthrough_stream_message_delta_without_usage_leaves_no_zero_fabrication() {
	let input = format!(
		"{}{}",
		r#"data: {"type":"message_start","message":{"id":"msg_2","type":"message","role":"assistant","model":"accounts/fireworks/models/kimi-k3","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
		"\n\n",
	);
	// ... no content deltas, no message_delta usage, straight to stop.
	let input = format!("{input}data: {{\"type\":\"message_stop\"}}\n\n");
	let info = run_passthrough(input).await;
	// The stream never proved usage: input/output must not be Some(0).
	assert_eq!(
		info.response.input_tokens, None,
		"message_start placeholder zeros must not become final usage evidence"
	);
	assert_eq!(info.response.output_tokens, None);
}
