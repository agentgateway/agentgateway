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
//!
//! The placeholder is ALL-ZERO only: Anthropic `input_tokens` excludes
//! cached tokens, so a fully cached `message_start` (0/0 with positive
//! cache-read/creation) is real billing evidence and must survive even
//! when the final `message_delta` does not repeat the cache dimensions.

use std::sync::{Arc, Mutex};

use agent_http::Body;
use http_body_util::BodyExt;

use super::passthrough_stream;
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

fn message_start_with_usage(usage_json: &str) -> String {
	format!(
		r#"data: {{"type":"message_start","message":{{"id":"msg_c","type":"message","role":"assistant","model":"accounts/fireworks/models/kimi-k3","content":[],"stop_reason":null,"stop_sequence":null,"usage":{usage_json}}}}}"#,
	)
}

/// Anthropic `input_tokens` EXCLUDES cached tokens, so a fully cached
/// request legitimately reports `input_tokens: 0` / `output_tokens: 0` with
/// a positive `cache_read_input_tokens`. That is real billing evidence —
/// NOT a placeholder — and must survive even when no `message_delta`
/// follows. The provider-reported zeros become evidence only because they
/// come with cache counts, unlike the all-zero case above.
#[tokio::test]
async fn passthrough_stream_fully_cached_message_start_keeps_cache_read_evidence() {
	let input = format!(
		"{}\n\n{}\n\n",
		message_start_with_usage(
			r#"{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":2048}"#
		),
		r#"data: {"type":"message_stop"}"#
	);
	let info = run_passthrough(input).await;
	assert_eq!(
		info.response.cached_input_tokens,
		Some(2048),
		"cache-read tokens are billing evidence even with 0/0 input/output"
	);
	assert_eq!(info.response.cache_creation_input_tokens, Some(0));
	assert_eq!(info.response.input_tokens, Some(0));
	assert_eq!(info.response.output_tokens, Some(0));
}

/// Same fully-cached shape, but the prompt WROTE a new cache entry
/// (`cache_creation_input_tokens` only, cache-read absent).
#[tokio::test]
async fn passthrough_stream_fully_cached_message_start_keeps_cache_creation_evidence() {
	let input = format!(
		"{}\n\n{}\n\n",
		message_start_with_usage(
			r#"{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":512}"#
		),
		r#"data: {"type":"message_stop"}"#
	);
	let info = run_passthrough(input).await;
	assert_eq!(info.response.cache_creation_input_tokens, Some(512));
	assert_eq!(
		info.response.cached_input_tokens, None,
		"absent cache-read stays unknown, not zero"
	);
	assert_eq!(info.response.input_tokens, Some(0));
	assert_eq!(info.response.output_tokens, Some(0));
}

/// A `message_delta` that reports only the token counts and does NOT repeat
/// the cache dimensions must not wipe the `message_start` cache evidence.
#[tokio::test]
async fn passthrough_stream_delta_without_cache_fields_keeps_message_start_cache() {
	let input = format!(
		"{}\n\n{}\n\n{}\n\n{}\n\n",
		message_start_with_usage(
			r#"{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":2048}"#
		),
		r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
		r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":8,"output_tokens":20}}"#,
		r#"data: {"type":"message_stop"}"#
	);
	let info = run_passthrough(input).await;
	assert_eq!(info.response.input_tokens, Some(8), "delta override wins");
	assert_eq!(info.response.output_tokens, Some(20));
	assert_eq!(
		info.response.cached_input_tokens,
		Some(2048),
		"cache evidence from message_start must survive a cache-less delta"
	);
	assert_eq!(info.response.total_tokens, Some(28));
}

/// A `message_start` with regular non-zero token usage and no later
/// `message_delta` remains direct usage evidence (unchanged behavior).
#[tokio::test]
async fn passthrough_stream_nonzero_message_start_usage_is_evidence() {
	let input = format!(
		"{}\n\n{}\n\n",
		message_start_with_usage(r#"{"input_tokens":91,"output_tokens":1}"#),
		r#"data: {"type":"message_stop"}"#
	);
	let info = run_passthrough(input).await;
	assert_eq!(info.response.input_tokens, Some(91));
	assert_eq!(info.response.output_tokens, Some(1));
}

/// Drive the Messages→completions streaming translation (the cross-format
/// path an OpenAI client sees with an Anthropic Messages upstream) and
/// return both the captured telemetry info and the client-visible bytes.
async fn run_translate(input: String) -> (LLMInfo, Vec<u8>) {
	let captured = captured_info();
	let out = super::from_completions::translate_stream(
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
	(captured.lock().unwrap().clone(), out.to_vec())
}

/// Fully-cached `message_start` on the translation path: the cache
/// dimension is telemetry evidence, and — unchanged behavior — the client
/// stream carries no usage chunk because none arrived via `message_delta`.
#[tokio::test]
async fn translate_stream_fully_cached_message_start_keeps_cache_evidence() {
	let input = format!(
		"{}\n\n{}\n\n{}\n\n{}\n\n",
		message_start_with_usage(
			r#"{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":2048}"#
		),
		r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
		r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
		r#"data: {"type":"message_stop"}"#
	);
	let (info, out) = run_translate(input).await;
	assert_eq!(info.response.cached_input_tokens, Some(2048));
	assert_eq!(info.response.input_tokens, Some(0));
	assert_eq!(info.response.output_tokens, Some(0));
	assert!(
		!String::from_utf8_lossy(&out).contains("prompt_tokens"),
		"no usage chunk is synthesized for the client without a message_delta"
	);
}

/// Translation path, `message_delta` without cache fields: the client's
/// final usage chunk must be built from the `message_start` cache evidence
/// (completions `prompt_tokens` includes cache tokens by convention) and
/// the telemetry keeps both sources.
#[tokio::test]
async fn translate_stream_delta_without_cache_fields_builds_cache_aware_usage() {
	let input = format!(
		"{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n",
		message_start_with_usage(
			r#"{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":2048}"#
		),
		r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
		r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
		r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"input_tokens":8,"output_tokens":20}}"#,
		r#"data: {"type":"message_stop"}"#
	);
	let (info, out) = run_translate(input).await;
	assert_eq!(info.response.input_tokens, Some(8));
	assert_eq!(info.response.output_tokens, Some(20));
	assert_eq!(info.response.cached_input_tokens, Some(2048));
	assert_eq!(info.response.total_tokens, Some(28));

	let out_str = String::from_utf8_lossy(&out).into_owned();
	let usage_chunk = out_str
		.lines()
		.map(|l| l.trim_start_matches("data: "))
		.find(|l| l.contains("prompt_tokens"))
		.expect("final usage chunk");
	let v: serde_json::Value = serde_json::from_str(usage_chunk).expect("usage chunk json");
	let usage = v.get("usage").expect("usage field");
	assert_eq!(
		usage["prompt_tokens"],
		8 + 2048,
		"prompt_tokens includes cache-read"
	);
	assert_eq!(usage["completion_tokens"], 20);
	assert_eq!(usage["total_tokens"], 8 + 2048 + 20);
	assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 2048);
}
