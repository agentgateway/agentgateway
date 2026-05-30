use serde_json::{Value, json};

use super::*;

fn req(v: Value) -> types::completions::Request {
	serde_json::from_value(v).expect("valid completions request")
}

fn to_gemini(v: Value) -> Value {
	let bytes = from_completions::translate(&req(v), None).expect("translate ok");
	serde_json::from_slice(&bytes).expect("valid json")
}

/// Run a Gemini response through the real `translate_response` entry and return the
/// client-facing JSON (after the untyped completions::Response round-trip + serialize),
/// so tests assert what a client actually receives, not the pre-deserialize intermediate.
fn resp(v: Value) -> Value {
	let bytes = bytes::Bytes::from(serde_json::to_vec(&v).expect("serialize gemini response"));
	let out = to_completions::translate_response(&bytes).expect("translate_response ok");
	let serialized = out.serialize().expect("serialize completions response");
	serde_json::from_slice(&serialized).expect("valid json")
}

// ---------- Request: roles, system, content ----------

#[test]
fn system_message_becomes_system_instruction() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [
			{ "role": "system", "content": "be terse" },
			{ "role": "user", "content": "hi" }
		]
	}));
	assert_eq!(g["systemInstruction"]["parts"][0]["text"], "be terse");
	assert_eq!(g["contents"][0]["role"], "user");
	assert_eq!(g["contents"][0]["parts"][0]["text"], "hi");
}

#[test]
fn assistant_role_maps_to_model() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [
			{ "role": "user", "content": "hi" },
			{ "role": "assistant", "content": "hello" }
		]
	}));
	assert_eq!(g["contents"][1]["role"], "model");
	assert_eq!(g["contents"][1]["parts"][0]["text"], "hello");
}

#[test]
fn consecutive_same_role_messages_merge() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [
			{ "role": "user", "content": "a" },
			{ "role": "user", "content": "b" },
			{ "role": "assistant", "content": "c" }
		]
	}));
	// Two entries, not three: [user(a,b), model(c)]
	assert_eq!(g["contents"].as_array().unwrap().len(), 2);
	assert_eq!(g["contents"][0]["role"], "user");
	assert_eq!(g["contents"][0]["parts"].as_array().unwrap().len(), 2);
	assert_eq!(g["contents"][1]["role"], "model");
}

#[test]
fn empty_messages_get_synthetic_user_entry() {
	let g = to_gemini(json!({ "model": "gemini-2.5-flash", "messages": [] }));
	assert_eq!(g["contents"][0]["role"], "user");
	assert_eq!(g["contents"][0]["parts"][0]["text"], " ");
}

// ---------- Request: content parts / images ----------

#[test]
fn data_url_image_becomes_inline_data() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": [
			{ "type": "text", "text": "what is this" },
			{ "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw0KG" } }
		]}]
	}));
	let parts = g["contents"][0]["parts"].as_array().unwrap();
	assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
	assert_eq!(parts[1]["inlineData"]["data"], "iVBORw0KG");
}

#[test]
fn jpg_mime_is_canonicalized_to_jpeg() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": [
			{ "type": "image_url", "image_url": { "url": "data:image/jpg;base64,AAAA" } }
		]}]
	}));
	assert_eq!(
		g["contents"][0]["parts"][0]["inlineData"]["mimeType"],
		"image/jpeg"
	);
}

#[test]
fn gs_url_image_becomes_file_data() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": [
			{ "type": "image_url", "image_url": { "url": "gs://bucket/cat.png" } }
		]}]
	}));
	// image-only user content gets a synthetic blank text part appended.
	let parts = g["contents"][0]["parts"].as_array().unwrap();
	assert_eq!(parts[0]["fileData"]["fileUri"], "gs://bucket/cat.png");
	// Vertex requires a mimeType on gs:// fileData; inferred from the .png extension.
	assert_eq!(parts[0]["fileData"]["mimeType"], "image/png");
	assert!(parts.iter().any(|p| p.get("text").is_some()));
}

#[test]
fn gs_url_without_extension_or_hint_is_rejected() {
	let err = from_completions::translate(
		&req(json!({
			"model": "gemini-2.5-flash",
			"messages": [{ "role": "user", "content": [
				{ "type": "image_url", "image_url": { "url": "gs://bucket/object" } }
			]}]
		})),
		None,
	);
	assert!(
		err.is_err(),
		"extension-less gs:// with no MIME hint must be rejected before egress"
	);
}

#[test]
fn gs_url_uses_explicit_mime_hint() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": [
			{ "type": "image_url",
				"image_url": { "url": "gs://bucket/object", "format": "image/webp" } }
		]}]
	}));
	assert_eq!(
		g["contents"][0]["parts"][0]["fileData"]["mimeType"],
		"image/webp"
	);
}

#[test]
fn empty_string_user_content_is_preserved() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "" }]
	}));
	// Distinct from the synthetic " " filler: an explicit "" round-trips as {text: ""}.
	assert_eq!(g["contents"][0]["role"], "user");
	assert_eq!(g["contents"][0]["parts"][0]["text"], "");
}

#[test]
fn http_image_url_is_rejected() {
	let err = from_completions::translate(
		&req(json!({
			"model": "gemini-2.5-flash",
			"messages": [{ "role": "user", "content": [
				{ "type": "image_url", "image_url": { "url": "https://example.com/cat.png" } }
			]}]
		})),
		None,
	);
	assert!(err.is_err(), "http(s) image_url must be rejected");
}

// ---------- Request: tools ----------

#[test]
fn tool_calls_become_function_call_parts() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [
			{ "role": "user", "content": "weather?" },
			{ "role": "assistant", "tool_calls": [
				{ "id": "call_1", "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Berlin\"}" } }
			]}
		]
	}));
	let fc = &g["contents"][1]["parts"][0]["functionCall"];
	assert_eq!(fc["name"], "get_weather");
	assert_eq!(fc["args"]["city"], "Berlin");
}

#[test]
fn tool_result_becomes_function_response_with_user_role() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [
			{ "role": "user", "content": "weather?" },
			{ "role": "assistant", "tool_calls": [
				{ "id": "call_1", "type": "function",
					"function": { "name": "get_weather", "arguments": "{}" } }
			]},
			{ "role": "tool", "tool_call_id": "call_1", "content": "12C" }
		]
	}));
	assert_eq!(g["contents"][2]["role"], "user");
	let fr = &g["contents"][2]["parts"][0]["functionResponse"];
	assert_eq!(fr["name"], "get_weather");
	assert_eq!(fr["response"]["content"], "12C");
}

#[test]
fn tools_become_function_declarations() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "hi" }],
		"tools": [{ "type": "function", "function": {
			"name": "get_weather", "description": "weather",
			"parameters": { "type": "object", "properties": {} }
		}}]
	}));
	let d = &g["tools"][0]["functionDeclarations"][0];
	assert_eq!(d["name"], "get_weather");
	assert_eq!(d["description"], "weather");
}

#[test]
fn tool_choice_mapping() {
	let auto = to_gemini(json!({
		"model": "gemini-2.5-flash", "messages": [{ "role": "user", "content": "x" }],
		"tool_choice": "auto"
	}));
	assert_eq!(auto["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");

	let none = to_gemini(json!({
		"model": "gemini-2.5-flash", "messages": [{ "role": "user", "content": "x" }],
		"tool_choice": "none"
	}));
	assert_eq!(none["toolConfig"]["functionCallingConfig"]["mode"], "NONE");

	let required = to_gemini(json!({
		"model": "gemini-2.5-flash", "messages": [{ "role": "user", "content": "x" }],
		"tool_choice": "required"
	}));
	assert_eq!(
		required["toolConfig"]["functionCallingConfig"]["mode"],
		"ANY"
	);

	let named = to_gemini(json!({
		"model": "gemini-2.5-flash", "messages": [{ "role": "user", "content": "x" }],
		"tool_choice": { "type": "function", "function": { "name": "f" } }
	}));
	assert_eq!(named["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
	assert_eq!(
		named["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
		"f"
	);
}

// ---------- Request: generationConfig / structured outputs / thinking ----------

#[test]
fn generation_config_fields_map() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"temperature": 0.5, "top_p": 0.9, "frequency_penalty": 0.1, "presence_penalty": 0.2,
		"max_completion_tokens": 256, "stop": ["STOP"], "seed": 42, "n": 2, "top_k": 40
	}));
	let gc = &g["generationConfig"];
	assert_eq!(gc["temperature"], 0.5);
	assert_eq!(gc["topP"], 0.9);
	assert_eq!(gc["frequencyPenalty"], 0.1);
	assert_eq!(gc["presencePenalty"], 0.2);
	assert_eq!(gc["maxOutputTokens"], 256);
	assert_eq!(gc["stopSequences"][0], "STOP");
	assert_eq!(gc["seed"], 42);
	assert_eq!(gc["candidateCount"], 2);
	assert_eq!(gc["topK"], 40);
}

#[test]
fn bare_request_emits_no_generation_config() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }]
	}));
	assert!(g.get("generationConfig").is_none() || g["generationConfig"].is_null());
}

#[test]
fn response_format_json_schema_unwraps_to_response_schema() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"response_format": { "type": "json_schema", "json_schema": {
			"name": "out", "strict": true,
			"schema": { "type": "object", "properties": { "a": { "type": "string" } } }
		}}
	}));
	assert_eq!(
		g["generationConfig"]["responseMimeType"],
		"application/json"
	);
	assert_eq!(g["generationConfig"]["responseSchema"]["type"], "object");
	// The wrapper fields (name/strict) must be dropped.
	assert!(
		g["generationConfig"]["responseSchema"]
			.get("strict")
			.is_none()
	);
}

#[test]
fn response_format_json_object_sets_mime_only() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"response_format": { "type": "json_object" }
	}));
	assert_eq!(
		g["generationConfig"]["responseMimeType"],
		"application/json"
	);
	assert!(g["generationConfig"].get("responseSchema").is_none());
}

#[test]
fn reasoning_effort_maps_to_thinking_level_on_gemini_3() {
	let g = to_gemini(json!({
		"model": "gemini-3-pro",
		"messages": [{ "role": "user", "content": "x" }],
		"reasoning_effort": "high"
	}));
	assert_eq!(
		g["generationConfig"]["thinkingConfig"]["thinkingLevel"],
		"high"
	);
}

#[test]
fn reasoning_effort_maps_to_thinking_budget_on_gemini_25() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"reasoning_effort": "high"
	}));
	assert_eq!(
		g["generationConfig"]["thinkingConfig"]["thinkingBudget"],
		4096
	);
}

#[test]
fn reasoning_effort_none_omits_thinking_config() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"reasoning_effort": "none"
	}));
	let gc = g.get("generationConfig");
	assert!(gc.is_none() || gc.unwrap().get("thinkingConfig").is_none());
}

// ---------- Request: cachedContent / labels ----------

#[test]
fn cached_content_strips_conflicting_fields() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "system", "content": "sys" }, { "role": "user", "content": "x" }],
		"tools": [{ "type": "function", "function": { "name": "f" } }],
		"tool_choice": "auto",
		"cachedContent": "projects/p/locations/l/cachedContents/abc"
	}));
	assert_eq!(
		g["cachedContent"],
		"projects/p/locations/l/cachedContents/abc"
	);
	assert!(g.get("systemInstruction").is_none() || g["systemInstruction"].is_null());
	assert!(g["tools"].as_array().map(|a| a.is_empty()).unwrap_or(true));
	assert!(g.get("toolConfig").is_none() || g["toolConfig"].is_null());
}

#[test]
fn labels_pass_through_at_top_level() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"labels": { "team": "ai" }
	}));
	assert_eq!(g["labels"]["team"], "ai");
}

// ---------- Response: content / reasoning / tool calls ----------

#[test]
fn response_text_maps_to_content() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "hello" }] },
			"finishReason": "STOP" }],
		"usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 2, "totalTokenCount": 7 }
	}));
	assert_eq!(r["choices"][0]["message"]["content"], "hello");
	assert_eq!(r["choices"][0]["finish_reason"], "stop");
	assert_eq!(r["usage"]["prompt_tokens"], 5);
	assert_eq!(r["usage"]["completion_tokens"], 2);
	assert_eq!(r["usage"]["total_tokens"], 7);
}

#[test]
fn response_thought_parts_map_to_reasoning_content() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "text": "thinking...", "thought": true },
			{ "text": "answer" }
		]}, "finishReason": "STOP" }]
	}));
	assert_eq!(
		r["choices"][0]["message"]["reasoning_content"],
		"thinking..."
	);
	assert_eq!(r["choices"][0]["message"]["content"], "answer");
}

#[test]
fn response_thought_prefix_workaround() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "text": "THOUGHT: reasoning here" },
			{ "text": "the answer" }
		]}, "finishReason": "STOP" }]
	}));
	assert_eq!(
		r["choices"][0]["message"]["reasoning_content"],
		"reasoning here"
	);
	assert_eq!(r["choices"][0]["message"]["content"], "the answer");
}

#[test]
fn response_function_call_overrides_finish_reason_to_tool_calls() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "name": "get_weather", "args": { "city": "Berlin" } } }
		]}, "finishReason": "STOP" }]
	}));
	assert_eq!(r["choices"][0]["finish_reason"], "tool_calls");
	let tc = &r["choices"][0]["message"]["tool_calls"][0];
	assert_eq!(tc["function"]["name"], "get_weather");
	assert_eq!(tc["index"], 0);
}

#[test]
fn response_synthesizes_tool_call_id_when_absent() {
	let r = resp(json!({
		"responseId": "resp-abc",
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "name": "a", "args": {} } },
			{ "functionCall": { "name": "a", "args": {} } }
		]}, "finishReason": "STOP" }]
	}));
	let calls = r["choices"][0]["message"]["tool_calls"].as_array().unwrap();
	// Parallel identical calls get distinct positional ids.
	assert_eq!(calls[0]["id"], "call_resp-abc_0");
	assert_eq!(calls[1]["id"], "call_resp-abc_1");
}

#[test]
fn response_preserves_native_function_call_id() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "id": "fc_native", "name": "a", "args": {} } }
		]}, "finishReason": "STOP" }]
	}));
	assert_eq!(
		r["choices"][0]["message"]["tool_calls"][0]["id"],
		"fc_native"
	);
}

// ---------- Response: finishReason table / usage ----------

#[test]
fn finish_reason_mapping_table() {
	let cases = [
		("MAX_TOKENS", "length"),
		("SAFETY", "content_filter"),
		("RECITATION", "content_filter"),
		("LANGUAGE", "content_filter"),
		("BLOCKLIST", "content_filter"),
		("PROHIBITED_CONTENT", "content_filter"),
		("SPII", "content_filter"),
		("UNEXPECTED_TOOL_CALL", "content_filter"),
		("TOO_MANY_TOOL_CALLS", "content_filter"),
		("IMAGE_SAFETY", "content_filter"),
		("MALFORMED_FUNCTION_CALL", "stop"),
		("OTHER", "stop"),
		("FINISH_REASON_UNSPECIFIED", "stop"),
		("SOME_FUTURE_VALUE", "stop"),
	];
	for (gemini, openai) in cases {
		let r = resp(json!({
			"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
				"finishReason": gemini }]
		}));
		assert_eq!(
			r["choices"][0]["finish_reason"], openai,
			"{gemini} should map to {openai}"
		);
	}
}

#[test]
fn usage_maps_cached_and_reasoning_tokens() {
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": "STOP" }],
		"usageMetadata": {
			"promptTokenCount": 100, "candidatesTokenCount": 50, "totalTokenCount": 150,
			"cachedContentTokenCount": 30, "thoughtsTokenCount": 20
		}
	}));
	assert_eq!(r["usage"]["prompt_tokens_details"]["cached_tokens"], 30);
	assert_eq!(
		r["usage"]["completion_tokens_details"]["reasoning_tokens"],
		20
	);
}

#[test]
fn prompt_block_synthesizes_content_filter_choice() {
	let r = resp(json!({
		"promptFeedback": { "blockReason": "SAFETY" },
		"usageMetadata": { "promptTokenCount": 12, "totalTokenCount": 12 }
	}));
	assert_eq!(r["choices"][0]["finish_reason"], "content_filter");
	assert_eq!(r["choices"][0]["message"]["content"], "");
	assert_eq!(r["usage"]["prompt_tokens"], 12);
}

// ---------- Streaming ----------

/// Feed one Gemini stream chunk through the per-stream translator and return the
/// emitted OpenAI chunk as JSON (`None` when the chunk produces nothing).
fn stream_chunk(state: &mut to_completions::StreamState, v: Value) -> Option<Value> {
	let chunk: vg::GenerateContentResponse =
		serde_json::from_value(v).expect("valid gemini stream chunk");
	state
		.translate(&chunk)
		.map(|sr| serde_json::to_value(sr).expect("serialize stream response"))
}

#[test]
fn streaming_role_emitted_once() {
	let mut s = to_completions::StreamState::new();
	let c1 = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "a" }] } }] }),
	)
	.unwrap();
	let c2 = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "b" }] } }] }),
	)
	.unwrap();
	assert_eq!(c1["object"], "chat.completion.chunk");
	assert_eq!(c1["choices"][0]["delta"]["role"], "assistant");
	assert_eq!(c1["choices"][0]["delta"]["content"], "a");
	// role appears on the first chunk only.
	assert!(c2["choices"][0]["delta"].get("role").is_none());
	assert_eq!(c2["choices"][0]["delta"]["content"], "b");
}

#[test]
fn streaming_thought_and_answer_split() {
	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [
			{ "text": "thinking", "thought": true },
			{ "text": "answer" }
		]}}]}),
	)
	.unwrap();
	assert_eq!(c["choices"][0]["delta"]["reasoning_content"], "thinking");
	assert_eq!(c["choices"][0]["delta"]["content"], "answer");
}

#[test]
fn streaming_tool_call_has_id_index_and_overrides_finish() {
	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({
			"responseId": "r1",
			"candidates": [{ "content": { "role": "model", "parts": [
				{ "functionCall": { "name": "get_weather", "args": { "city": "Berlin" } } }
			]}, "finishReason": "STOP" }]
		}),
	)
	.unwrap();
	let tc = &c["choices"][0]["delta"]["tool_calls"][0];
	assert_eq!(tc["index"], 0);
	assert_eq!(tc["id"], "call_r1_0");
	assert_eq!(tc["function"]["name"], "get_weather");
	assert_eq!(tc["function"]["arguments"], "{\"city\":\"Berlin\"}");
	// STOP is overridden to tool_calls when the candidate carries a function call.
	assert_eq!(c["choices"][0]["finish_reason"], "tool_calls");
}

#[test]
fn streaming_preserves_native_tool_call_id() {
	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "id": "fc_native", "name": "a", "args": {} } }
		]}}]}),
	)
	.unwrap();
	assert_eq!(c["choices"][0]["delta"]["tool_calls"][0]["id"], "fc_native");
}

#[test]
fn streaming_trailing_usage_chunk_has_empty_choices() {
	let mut s = to_completions::StreamState::new();
	// Consume the role on a content chunk, then a usage-only trailing chunk.
	stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "hi" }] } }] }),
	);
	let c = stream_chunk(
		&mut s,
		json!({ "usageMetadata": {
			"promptTokenCount": 5, "candidatesTokenCount": 2, "totalTokenCount": 7,
			"thoughtsTokenCount": 1, "cachedContentTokenCount": 3
		}}),
	)
	.unwrap();
	assert!(c["choices"].as_array().unwrap().is_empty());
	assert_eq!(c["usage"]["prompt_tokens"], 5);
	assert_eq!(c["usage"]["completion_tokens"], 2);
	assert_eq!(c["usage"]["total_tokens"], 7);
	assert_eq!(
		c["usage"]["completion_tokens_details"]["reasoning_tokens"],
		1
	);
	assert_eq!(c["usage"]["prompt_tokens_details"]["cached_tokens"], 3);
}
