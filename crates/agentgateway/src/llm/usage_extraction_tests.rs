//! Buffered cross-format usage extraction regression tests.
//!
//! The buffered path logs usage from the TRANSLATED client-format response.
//! Those translations materialize protocol-mandated usage placeholders
//! (Anthropic `usage.input_tokens` is non-optional; a missing OpenAI usage
//! becomes `unwrap_or(0)`), which fabricate complete zero-token provider
//! attempts. The proxy therefore overlays usage parsed from the
//! upstream-native wire format: real values override, a missing upstream
//! usage clears the placeholder back to unknown (never zero).

use bytes::Bytes;

use crate::llm::AIProvider;

use super::ChatFormat;

fn completions_body(usage_json: &str) -> Bytes {
	Bytes::from(format!(
		r#"{{"id":"cmpl-1","object":"chat.completion","created":1,"model":"accounts/fireworks/models/kimi-k3","choices":[{{"index":0,"message":{{"role":"assistant","content":"hi"}},"finish_reason":"stop"}}],{usage_json}}}"#
	))
}

/// A successful upstream response WITH usage: the overlay carries the real
/// token counts (prompt_tokens semantics — total prompt including cache —
/// consistent with the streaming path).
#[test]
fn native_upstream_usage_present() {
	let bytes = completions_body(
		r#""usage":{"prompt_tokens":1000,"completion_tokens":50,"total_tokens":1050,"prompt_tokens_details":{"cached_tokens":800}}"#,
	);
	let native = AIProvider::native_upstream_llm_response(
		ChatFormat::OpenAICompletions,
		&bytes,
		Default::default(),
	)
	.expect("native parse");
	assert_eq!(native.input_tokens, Some(1000));
	assert_eq!(native.output_tokens, Some(50));
	assert_eq!(native.total_tokens, Some(1050));
	assert_eq!(native.cached_input_tokens, Some(800));
}

/// A successful upstream response WITHOUT usage: every dimension stays
/// unknown. This is the evidence that must CLEAR the translated response's
/// fabricated `usage: {input_tokens: 0, output_tokens: 0}`.
#[test]
fn native_upstream_usage_missing_is_unknown_never_zero() {
	let bytes = completions_body(r#""usage":null"#);
	let native = AIProvider::native_upstream_llm_response(
		ChatFormat::OpenAICompletions,
		&bytes,
		Default::default(),
	)
	.expect("native parse");
	assert_eq!(native.input_tokens, None);
	assert_eq!(native.output_tokens, None);
	assert_eq!(native.total_tokens, None);
	assert_eq!(native.cached_input_tokens, None);
	assert_eq!(native.cache_creation_input_tokens, None);
}

/// A garbage body must not break request processing: the overlay is
/// best-effort and falls back to the translated response evidence.
#[test]
fn native_upstream_parse_failure_is_best_effort() {
	let bytes = Bytes::from_static(b"<not json>");
	let native = AIProvider::native_upstream_llm_response(
		ChatFormat::OpenAICompletions,
		&bytes,
		Default::default(),
	);
	assert!(native.is_none());
}

/// The overlay moves ONLY usage dimensions; provider model (and content,
/// held by the translated response) stay untouched.
#[test]
fn overlay_moves_usage_only() {
	let mut translated = super::LLMResponse {
		input_tokens: Some(0),
		output_tokens: Some(0),
		..Default::default()
	};
	translated.provider_model = Some("client-visible-model".into());
	let native = super::LLMResponse {
		input_tokens: Some(91),
		output_tokens: Some(16),
		total_tokens: Some(107),
		..Default::default()
	};
	AIProvider::overlay_upstream_usage(&mut translated, native);
	assert_eq!(translated.input_tokens, Some(91));
	assert_eq!(translated.output_tokens, Some(16));
	assert_eq!(translated.total_tokens, Some(107));
	assert_eq!(
		translated.provider_model.as_deref(),
		Some("client-visible-model")
	);
}

/// Only true cross-format pairs need the overlay; native passthrough pairs
/// already parse the upstream wire format as the client format.
#[test]
fn crosses_formats_matrix() {
	use super::InputFormat as IF;
	assert!(AIProvider::crosses_chat_formats(
		IF::Messages,
		ChatFormat::OpenAICompletions
	));
	assert!(AIProvider::crosses_chat_formats(
		IF::Completions,
		ChatFormat::AnthropicMessages
	));
	assert!(!AIProvider::crosses_chat_formats(
		IF::Messages,
		ChatFormat::AnthropicMessages
	));
	assert!(!AIProvider::crosses_chat_formats(
		IF::Completions,
		ChatFormat::OpenAICompletions
	));
	assert!(!AIProvider::crosses_chat_formats(
		IF::Responses,
		ChatFormat::OpenAIResponses
	));
	assert!(!AIProvider::crosses_chat_formats(
		IF::Gemini,
		ChatFormat::VertexGemini
	));
}
