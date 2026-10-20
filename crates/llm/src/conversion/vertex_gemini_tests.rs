use serde_json::{Value, json};

use super::*;

fn req(v: Value) -> types::completions::Request {
	serde_json::from_value(v).expect("valid completions request")
}

fn to_gemini(v: Value) -> Value {
	let bytes = from_completions::translate(&req(v), true, None).expect("translate ok");
	serde_json::from_slice(&bytes).expect("valid json")
}

fn to_gemini_api(v: Value) -> Value {
	let bytes = from_completions::translate(&req(v), false, None).expect("translate ok");
	serde_json::from_slice(&bytes).expect("valid json")
}

fn gemini_response_bytes(v: Value) -> bytes::Bytes {
	bytes::Bytes::from(serde_json::to_vec(&v).expect("serialize gemini response"))
}

/// Run a Gemini response through the real `translate_response` entry and return the
/// client-facing JSON (after the untyped completions::Response round-trip + serialize),
/// so tests assert what a client actually receives, not the pre-deserialize intermediate.
fn resp(v: Value) -> Value {
	let out =
		to_completions::translate_response(&gemini_response_bytes(v)).expect("translate_response ok");
	let serialized = out.serialize().expect("serialize completions response");
	serde_json::from_slice(&serialized).expect("valid json")
}

/// Run a Gemini response through `translate_response` and return the `LLMResponse` used to
/// populate CEL/log fields.
fn llm_resp(v: Value) -> crate::LLMResponse {
	to_completions::translate_response(&gemini_response_bytes(v))
		.expect("translate_response ok")
		.to_llm_response(crate::LogContentFields::default())
}

// ---------- Request: roles, system, content ----------

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

#[rstest::rstest]
fn system_messages_keep_the_prefix_stable(#[values("system", "developer")] role: &str) {
	let turn = |n: usize| {
		let messages: Vec<Value> = [system("You are Claude Code."), user("read the files")]
			.into_iter()
			.chain((0..n).flat_map(|i| {
				let id = format!("call_{i}");
				[
					calls(&[&id]),
					result(&id),
					json!({ "role": role, "content": format!("<total_tokens>{i}</total_tokens>") }),
				]
			}))
			.collect();
		to_gemini(json!({ "model": "gemini-3.8-flash", "messages": messages }))
	};
	let (prev, next) = (turn(2), turn(3));
	assert_eq!(prev["systemInstruction"], next["systemInstruction"]);
	let (prev, next) = (
		prev["contents"].as_array().unwrap(),
		next["contents"].as_array().unwrap(),
	);
	assert_eq!(prev[..], next[..prev.len()]);
}

#[rstest::rstest]
#[case::leading(vec![system("s"), user("hi")], &["system:text", "user:text"])]
#[case::between_turns(
	vec![user("hi"), system("s"), assistant("ok")],
	&["user:text,reminder", "model:text"]
)]
#[case::after_single_call(
	vec![user("hi"), calls(&["a"]), system("s"), result("a"), user("next")],
	&["user:text", "model:call", "user:result", "user:reminder,text"]
)]
#[case::between_parallel_results(
	vec![user("hi"), calls(&["a", "b"]), result("a"), system("s"), result("b")],
	&["user:text", "model:call,call", "user:result,result", "user:reminder"]
)]
#[case::trailing(
	vec![user("hi"), calls(&["a"]), result("a"), system("s")],
	&["user:text", "model:call", "user:result", "user:reminder"]
)]
fn system_message_placement(#[case] messages: Vec<Value>, #[case] expected: &[&str]) {
	let g = to_gemini(json!({ "model": "gemini-3.8-flash", "messages": messages }));
	let system = g
		.get("systemInstruction")
		.map(|_| "system:text".to_string());
	let contents = g["contents"].as_array().unwrap().iter().map(|c| {
		let parts: Vec<&str> = c["parts"]
			.as_array()
			.unwrap()
			.iter()
			.map(|p| match p {
				_ if p.get("functionCall").is_some() => "call",
				_ if p.get("functionResponse").is_some() => "result",
				_ if p["text"]
					.as_str()
					.is_some_and(|t| t.starts_with("<system-reminder>")) =>
				{
					"reminder"
				},
				_ => "text",
			})
			.collect();
		format!("{}:{}", c["role"].as_str().unwrap(), parts.join(","))
	});
	assert_eq!(
		system.into_iter().chain(contents).collect::<Vec<_>>(),
		expected
	);
}

fn user(text: &str) -> Value {
	json!({ "role": "user", "content": text })
}

fn assistant(text: &str) -> Value {
	json!({ "role": "assistant", "content": text })
}

fn system(text: &str) -> Value {
	json!({ "role": "system", "content": text })
}

fn calls(ids: &[&str]) -> Value {
	let calls: Vec<Value> = ids
		.iter()
		.map(
			|id| json!({ "id": id, "type": "function", "function": { "name": "Read", "arguments": "{}" } }),
		)
		.collect();
	json!({ "role": "assistant", "tool_calls": calls })
}

fn result(id: &str) -> Value {
	json!({ "role": "tool", "tool_call_id": id, "content": "file" })
}

#[test]
fn empty_messages_get_synthetic_user_entry() {
	let g = to_gemini(json!({ "model": "gemini-2.5-flash", "messages": [] }));
	assert_eq!(g["contents"][0]["role"], "user");
	assert_eq!(g["contents"][0]["parts"][0]["text"], " ");
}

// ---------- Request: content parts / images ----------

#[test]
fn gs_url_without_extension_or_hint_is_rejected() {
	let err = from_completions::translate(
		&req(json!({
			"model": "gemini-2.5-flash",
			"messages": [{ "role": "user", "content": [
				{ "type": "image_url", "image_url": { "url": "gs://bucket/object" } }
			]}]
		})),
		true,
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

// ---------- Request: content parts / files ----------

fn file_content(file: Value) -> Value {
	json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": [
			{ "type": "text", "text": "What is this document about?" },
			{ "type": "file", "file": file }
		]}]
	})
}

#[test]
fn file_data_url_becomes_inline_data() {
	let g = to_gemini(file_content(json!({
		"filename": "report.pdf",
		"file_data": "data:application/pdf;base64,JVBERi0xLjQK"
	})));
	let part = &g["contents"][0]["parts"][1];
	assert_eq!(part["inlineData"]["mimeType"], "application/pdf");
	assert_eq!(part["inlineData"]["data"], "JVBERi0xLjQK");
}

#[test]
fn file_gs_uri_takes_mime_from_filename() {
	// The gs:// object has no extension, so the mime can only come from `filename`.
	let g = to_gemini(file_content(json!({
		"filename": "report.pdf",
		"file_data": "gs://bucket/object"
	})));
	let part = &g["contents"][0]["parts"][1];
	assert_eq!(part["fileData"]["fileUri"], "gs://bucket/object");
	assert_eq!(part["fileData"]["mimeType"], "application/pdf");
}

#[test]
fn file_gs_uri_without_extension_or_hint_is_rejected() {
	let err = from_completions::translate(
		&req(file_content(json!({
			"file_data": "gs://bucket/object"
		}))),
		true,
		None,
	);
	assert!(
		err.is_err(),
		"extension-less gs:// file with no MIME hint must be rejected before egress"
	);
}

#[test]
fn data_url_without_media_type_falls_back_to_filename() {
	let g = to_gemini(file_content(json!({
		"filename": "report.pdf",
		"file_data": "data:;base64,JVBERi0xLjQK"
	})));
	assert_eq!(
		g["contents"][0]["parts"][1]["inlineData"]["mimeType"],
		"application/pdf"
	);
}

#[test]
fn data_url_without_media_type_or_filename_is_rejected() {
	let err = from_completions::translate(
		&req(file_content(
			json!({ "file_data": "data:;base64,JVBERi0xLjQK" }),
		)),
		true,
		None,
	);
	assert!(
		err.is_err(),
		"an empty mimeType is rejected by Vertex, so it must not be sent"
	);
}

#[test]
fn raw_base64_file_data_takes_mime_from_filename() {
	let g = to_gemini(file_content(json!({
		"filename": "report.pdf",
		"file_data": "JVBERi0xLjQK"
	})));
	let part = &g["contents"][0]["parts"][1];
	assert_eq!(part["inlineData"]["mimeType"], "application/pdf");
	assert_eq!(part["inlineData"]["data"], "JVBERi0xLjQK");
}

#[test]
fn raw_base64_file_data_without_a_mime_source_is_rejected() {
	let err = from_completions::translate(
		&req(file_content(json!({ "file_data": "JVBERi0xLjQK" }))),
		true,
		None,
	);
	assert!(
		err.is_err(),
		"raw base64 with no filename or hint cannot yield the mimeType Vertex requires"
	);
}

#[test]
fn file_id_holding_a_gs_uri_becomes_file_data() {
	// Some clients put a bucket URI in `file_id` rather than `file_data`; Vertex can fetch
	// that directly, so it is honoured instead of being treated as an opaque OpenAI id.
	let g = to_gemini(file_content(json!({ "file_id": "gs://bucket/report.pdf" })));
	let part = &g["contents"][0]["parts"][1];
	assert_eq!(part["fileData"]["fileUri"], "gs://bucket/report.pdf");
	assert_eq!(part["fileData"]["mimeType"], "application/pdf");
}

#[test]
fn file_id_is_rejected_rather_than_dropped() {
	// Vertex has no OpenAI Files store, so an opaque file_id cannot be resolved. It must
	// error rather than silently vanish from the request (#3117).
	let err = from_completions::translate(
		&req(file_content(json!({ "file_id": "file-abc123" }))),
		true,
		None,
	);
	let err = err.expect_err("opaque file_id must be rejected");
	// Load-bearing: classify_ai_request maps UnsupportedConversion to 400, InvalidResponse to 503.
	assert!(
		matches!(err, crate::AIError::UnsupportedConversion(_)),
		"bad client input must be a request error, got {err:?}"
	);
	assert!(
		format!("{err:?}").contains("file_id"),
		"error should name the field: {err:?}"
	);
}

#[test]
fn file_part_without_data_or_id_is_rejected() {
	let err = from_completions::translate(
		&req(file_content(json!({ "filename": "report.pdf" }))),
		true,
		None,
	);
	assert!(
		err.is_err(),
		"a file part carrying no payload must be rejected"
	);
}

#[test]
fn rejected_signed_url_omits_query_from_error() {
	let err = from_completions::translate(
		&req(file_content(json!({
			"file_id": "https://storage.googleapis.com/bucket/object?X-Goog-Signature=secret"
		}))),
		true,
		None,
	)
	.expect_err("extension-less URL without a MIME hint must be rejected");
	let msg = err.to_string();
	assert!(!msg.contains("secret"), "{msg}");
	assert!(
		msg.contains("https://storage.googleapis.com/bucket/object"),
		"{msg}"
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
		true,
		None,
	);
	assert!(err.is_err(), "http(s) image_url must be rejected");
}

// ---------- Request: tools ----------

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

#[test]
fn vertex_omits_tool_call_id_on_function_parts() {
	// Vertex generateContent rejects `id` on functionCall/functionResponse (unlike AI Studio):
	// "Invalid JSON payload received. Unknown name \"id\" ... Cannot find field". Encodes the
	// rule so a future blind snapshot accept can't silently reintroduce it.
	let g = to_gemini(json!({
		"model": "gemini-2.5-pro",
		"messages": [
			{ "role": "user", "content": "Weather in Berlin?" },
			{ "role": "assistant", "content": null, "tool_calls": [
				{ "id": "call_1", "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"location\":\"Berlin\"}" } }
			]},
			{ "role": "tool", "tool_call_id": "call_1", "name": "get_weather",
				"content": "{\"temp\":9}" }
		],
		"tools": [{ "type": "function", "function": {
			"name": "get_weather", "description": "Get the current weather in a location",
			"parameters": { "type": "object", "properties": { "location": { "type": "string" } } }
		}}]
	}));

	let fc = &g["contents"][1]["parts"][0]["functionCall"];
	assert_eq!(fc["name"], "get_weather");
	assert!(
		fc.get("id").is_none(),
		"functionCall must not carry `id`: Vertex rejects it"
	);

	let fr = &g["contents"][2]["parts"][0]["functionResponse"];
	assert_eq!(fr["name"], "get_weather");
	assert_eq!(
		g["contents"][2]["parts"].as_array().unwrap().len(),
		1,
		"functionResponse must not have sibling parts"
	);
	assert!(
		fr.get("id").is_none(),
		"functionResponse must not carry `id`: Vertex rejects it"
	);
}

#[test]
fn thought_signature_round_trips_through_tool_call_id() {
	// Gemini 3 thinking models attach a thoughtSignature to functionCall parts and HARD-400 on
	// the next turn if it isn't echoed back ("Function call is missing a thought_signature in
	// functionCall parts"). A non-standard field on the OpenAI tool_call won't survive, since
	// clients drop unknown fields, so the signature is encoded into `tool_call_id` (which clients
	// reliably echo) and recovered before the outbound Vertex request. Mirrors litellm.
	//
	// A realistically long, base64-shaped signature guards against id truncation in the channel.
	let sig = "CqUBAbc123def456GHI789jklMNOpqrSTUvwxYZ0123456789+/aBcDeFgHiJkLmNoPqRsTuVwXyZ==";

	// Decode: the signature must ride inside the client-facing tool_call_id, not a side field a
	// client would strip on the way back.
	let decoded = resp(json!({
		"responseId": "resp-1",
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "name": "get_weather", "args": { "city": "Berlin" } },
				"thoughtSignature": sig }
		]}, "finishReason": "STOP" }]
	}));
	let echoed_id = decoded["choices"][0]["message"]["tool_calls"][0]["id"]
		.as_str()
		.expect("tool call id")
		.to_string();
	assert!(
		echoed_id.contains(sig),
		"thoughtSignature must be encoded into tool_call_id (client-durable channel), got {echoed_id:?}"
	);

	// Encode: a client echoes only standard OpenAI fields (id/type/function). The signature must
	// be recovered from the id onto the Vertex functionCall part, and the raw id must not leak.
	let g = to_gemini(json!({
		"model": "gemini-3-pro",
		"messages": [
			{ "role": "user", "content": "weather in Berlin?" },
			{ "role": "assistant", "content": null, "tool_calls": [
				{ "id": echoed_id, "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Berlin\"}" } }
			]},
		]
	}));

	let fc_part = &g["contents"][1]["parts"][0];
	assert_eq!(fc_part["functionCall"]["name"], "get_weather");
	assert_eq!(
		fc_part["thoughtSignature"], sig,
		"thoughtSignature must be recovered from tool_call_id into the re-encoded request"
	);
	assert!(
		fc_part["functionCall"].get("id").is_none(),
		"raw tool_call_id (with embedded signature) must not leak to the Vertex functionCall"
	);
}

#[test]
fn thought_signature_parallel_first_only_round_trips() {
	// Gemini 3 attaches a thoughtSignature to (often only) the first of several parallel calls
	// (js-genai #1275). Contract is faithful passthrough: we re-send exactly what we received, a
	// signature for call 0 and NONE for call 1 - we never synthesize a missing one. Same function
	// name twice also pins order, which is load-bearing once `id` is stripped (Vertex correlates
	// functionResponse to functionCall positionally).
	let sig = "CqUBAbc123def456GHI789jklMNOpqrSTUvwxYZ0123456789+/aBcDeFgHiJkLmNoPqRsTuVwXyZ==";

	let decoded = resp(json!({
		"responseId": "resp-1",
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "name": "get_weather", "args": { "city": "Columbus" } },
				"thoughtSignature": sig },
			{ "functionCall": { "name": "get_weather", "args": { "city": "Berlin" } } }
		]}, "finishReason": "STOP" }]
	}));
	let calls = decoded["choices"][0]["message"]["tool_calls"]
		.as_array()
		.expect("tool_calls");
	assert_eq!(calls.len(), 2);
	let id0 = calls[0]["id"].as_str().expect("id0").to_string();
	let id1 = calls[1]["id"].as_str().expect("id1").to_string();
	assert!(id0.contains(sig), "first call id must embed its signature");
	assert!(
		!id1.contains(sig),
		"second call received no signature; its id must stay plain (no synthesis)"
	);

	// Client echoes only standard fields + the tool results, in order.
	let g = to_gemini(json!({
		"model": "gemini-3-pro",
		"messages": [
			{ "role": "user", "content": "weather in Columbus and Berlin?" },
			{ "role": "assistant", "content": null, "tool_calls": [
				{ "id": id0, "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Columbus\"}" } },
				{ "id": id1, "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Berlin\"}" } }
			]},
			{ "role": "tool", "tool_call_id": id0, "name": "get_weather", "content": "{\"temp\":15}" },
			{ "role": "tool", "tool_call_id": id1, "name": "get_weather", "content": "{\"temp\":9}" }
		]
	}));

	let model_parts = g["contents"][1]["parts"].as_array().expect("model parts");
	assert_eq!(model_parts.len(), 2);
	// Order preserved (Columbus before Berlin), signature only on the call that had one.
	assert_eq!(model_parts[0]["functionCall"]["args"]["city"], "Columbus");
	assert_eq!(
		model_parts[0]["thoughtSignature"], sig,
		"first call must carry its recovered signature"
	);
	assert!(model_parts[0]["functionCall"].get("id").is_none());
	assert_eq!(model_parts[1]["functionCall"]["args"]["city"], "Berlin");
	assert!(
		model_parts[1].get("thoughtSignature").is_none(),
		"second call had no signature; must not synthesize or emit an empty one"
	);
	assert!(model_parts[1]["functionCall"].get("id").is_none());

	// functionResponse order must mirror the calls (positional correlation, no id).
	let responses: Vec<&Value> = g["contents"][2]["parts"]
		.as_array()
		.expect("response parts")
		.iter()
		.filter_map(|p| p.get("functionResponse"))
		.collect();
	assert_eq!(responses.len(), 2);
	assert_eq!(responses[0]["response"]["content"], "{\"temp\":15}");
	assert_eq!(responses[1]["response"]["content"], "{\"temp\":9}");
	assert!(responses[0].get("id").is_none());
}

#[test]
fn out_of_order_tool_results_reorder_to_call_order() {
	// Vertex rejects `id` and correlates functionResponse to functionCall positionally, so for
	// parallel calls to the SAME function name the tool results must be re-ordered to match the
	// assistant's tool_calls order. An OpenAI client may return `tool` messages in any order (the
	// linkage is tool_call_id); without reordering, Columbus's result would feed the Berlin call.
	let g = to_gemini(json!({
		"model": "gemini-2.5-pro",
		"messages": [
			{ "role": "user", "content": "weather in Columbus and Berlin?" },
			{ "role": "assistant", "content": null, "tool_calls": [
				{ "id": "call_a", "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Columbus\"}" } },
				{ "id": "call_b", "type": "function",
					"function": { "name": "get_weather", "arguments": "{\"city\":\"Berlin\"}" } }
			]},
			// Returned in REVERSE of the call order (allowed by the OpenAI spec).
			{ "role": "tool", "tool_call_id": "call_b", "name": "get_weather", "content": "{\"temp\":9}" },
			{ "role": "tool", "tool_call_id": "call_a", "name": "get_weather", "content": "{\"temp\":15}" }
		]
	}));

	// Calls retain tool_calls order.
	let calls = g["contents"][1]["parts"].as_array().expect("model parts");
	assert_eq!(calls[0]["functionCall"]["args"]["city"], "Columbus");
	assert_eq!(calls[1]["functionCall"]["args"]["city"], "Berlin");

	// Responses must be reordered to match: call_a (Columbus, temp 15) first, call_b (Berlin) second.
	let responses: Vec<&Value> = g["contents"][2]["parts"]
		.as_array()
		.expect("response parts")
		.iter()
		.filter_map(|p| p.get("functionResponse"))
		.collect();
	assert_eq!(responses.len(), 2);
	assert_eq!(
		responses[0]["response"]["content"], "{\"temp\":15}",
		"call_a (Columbus) result must come first to match the call order"
	);
	assert_eq!(responses[1]["response"]["content"], "{\"temp\":9}");
	assert!(responses[0].get("id").is_none());
}

// ---------- Request: generationConfig / structured outputs / thinking ----------

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

/// Gemini's responseSchema subset rejects $defs/$ref, so the translator must inline the $ref and
/// drop $defs before egress. additionalProperties is supported and must be preserved.
#[test]
fn response_format_inlines_pydantic_defs_and_refs() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "list the events" }],
		"response_format": {
			"type": "json_schema",
			"json_schema": {
				"name": "EventsList",
				"strict": true,
				"schema": {
					"$defs": {
						"CalendarEvent": {
							"additionalProperties": false,
							"properties": {
								"name": { "title": "Name", "type": "string" },
								"date": { "title": "Date", "type": "string" },
								"participants": { "items": { "type": "string" }, "title": "Participants", "type": "array" }
							},
							"required": ["name", "date", "participants"],
							"title": "CalendarEvent",
							"type": "object"
						}
					},
					"additionalProperties": false,
					"properties": {
						"events": { "items": { "$ref": "#/$defs/CalendarEvent" }, "title": "Events", "type": "array" }
					},
					"required": ["events"],
					"title": "EventsList",
					"type": "object"
				}
			}
		}
	}));
	let schema = &g["generationConfig"]["responseSchema"];
	let s = serde_json::to_string(schema).unwrap();
	assert!(
		!s.contains("$ref"),
		"Vertex rejects $ref in responseSchema: {s}"
	);
	assert!(
		!s.contains("$defs"),
		"Vertex rejects $defs in responseSchema: {s}"
	);
	// The referenced CalendarEvent must be inlined where the $ref was.
	assert_eq!(schema["properties"]["events"]["items"]["type"], "object");
	assert!(
		schema["properties"]["events"]["items"]["properties"]
			.get("name")
			.is_some(),
		"inlined object lost its properties: {s}"
	);
}

/// Reproduces the exact Vertex 400 verbatim:
///   Unknown name "$defs" at 'generation_config.response_schema'
///   Unknown name "$ref" at 'generation_config.response_schema.properties[3].value.any_of[0].items'
/// `properties[3]` is `options`, whose `anyOf[0].items.$ref` points at `#/$defs/SelectOption`. This
/// one payload exercises every construct the normalizer must handle: top-level `$defs`, a `$ref`
/// nested under `anyOf[0].items`, two `{type: null}` anyOf branches (`options`, `copy_value_from`),
/// and `additionalProperties: false`.
#[test]
fn response_format_inlines_real_dialog_question_schema() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "h" }],
		"response_format": {
			"type": "json_schema",
			"json_schema": {
				"name": "DialogQuestionDynamic",
				"strict": true,
				"schema": {
					"$defs": {
						"SelectOption": {
							"description": "A single option for select/multi_select/boolean questions.",
							"properties": {
								"id": { "title": "Id", "type": "string" },
								"label": { "title": "Label", "type": "string" }
							},
							"required": ["id", "label"],
							"title": "SelectOption",
							"type": "object",
							"additionalProperties": false
						}
					},
					"properties": {
						"information_id": { "enum": ["2l8I2VKmjQ"], "title": "Information Id", "type": "string" },
						"question_text": { "title": "Question Text", "type": "string" },
						"question_type": {
							"enum": ["select", "multi_select", "boolean", "number", "string", "address", "timespan"],
							"title": "Question Type",
							"type": "string"
						},
						"options": {
							"anyOf": [
								{ "items": { "$ref": "#/$defs/SelectOption" }, "type": "array" },
								{ "type": "null" }
							],
							"default": null,
							"title": "Options"
						},
						"copy_value_from": {
							"anyOf": [{ "type": "string" }, { "type": "null" }],
							"default": null,
							"title": "Copy Value From"
						}
					},
					"required": ["copy_value_from", "information_id", "options", "question_text", "question_type"],
					"title": "DialogQuestionDynamic",
					"type": "object",
					"additionalProperties": false
				}
			}
		}
	}));
	let s = serde_json::to_string(&g["generationConfig"]["responseSchema"]).unwrap();
	assert!(
		!s.contains("$ref"),
		"Vertex rejects $ref (here under options.anyOf[0].items): {s}"
	);
	assert!(!s.contains("$defs"), "Vertex rejects $defs: {s}");
	assert!(
		!s.contains("\"type\":\"null\""),
		"anyOf null branches must collapse to nullable; Gemini has no null type: {s}"
	);
	// SelectOption must be inlined (its fields survive even though $defs is gone).
	assert!(
		s.contains("\"label\""),
		"inlined SelectOption fields must survive: {s}"
	);
}

/// Translate a request whose `response_format` wraps `schema`; return the egress `responseSchema`.
fn response_schema(schema: Value) -> Value {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"response_format": {
			"type": "json_schema",
			"json_schema": { "name": "T", "strict": true, "schema": schema }
		}
	}));
	g["generationConfig"]["responseSchema"].clone()
}

// Case 1: additionalProperties: false must be preserved at every level (Gemini supports it).
#[test]
fn gemini_schema_preserves_additional_properties_false() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"inner": {
				"type": "object",
				"additionalProperties": false,
				"properties": { "a": { "type": "string" } }
			}
		}
	}));
	assert_eq!(
		s["additionalProperties"], false,
		"top-level additionalProperties: false must be preserved: {s}"
	);
	assert_eq!(
		s["properties"]["inner"]["additionalProperties"], false,
		"nested additionalProperties: false must be preserved: {s}"
	);
}

// Case 2: the same normalization must run on tool parameters (the ADK path), not just responseSchema.
#[test]
fn tool_parameters_inline_defs_and_refs() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"tools": [{
			"type": "function",
			"function": {
				"name": "save",
				"description": "save events",
				"parameters": {
					"$defs": {
						"Event": {
							"type": "object",
							"additionalProperties": false,
							"properties": { "name": { "type": "string" } },
							"required": ["name"]
						}
					},
					"type": "object",
					"additionalProperties": false,
					"properties": {
						"events": { "type": "array", "items": { "$ref": "#/$defs/Event" } }
					},
					"required": ["events"]
				}
			}
		}]
	}));
	let params = &g["tools"][0]["functionDeclarations"][0]["parameters"];
	let txt = serde_json::to_string(params).unwrap();
	assert!(
		!txt.contains("$ref"),
		"tool parameters must inline $ref: {txt}"
	);
	assert!(
		!txt.contains("$defs"),
		"tool parameters must drop $defs: {txt}"
	);
	// Unlike responseSchema on Vertex, the tool path strips additionalProperties (preserve_ap=false).
	assert!(
		!txt.contains("additionalProperties"),
		"tool parameters must drop additionalProperties: {txt}"
	);
	assert_eq!(params["properties"]["events"]["items"]["type"], "object");
}

#[rstest::rstest]
#[case::minimum(
	json!({ "type": "integer", "exclusiveMinimum": 0 }),
	json!({ "type": "integer", "minimum": 0 })
)]
#[case::maximum(
	json!({ "type": "number", "exclusiveMaximum": 1 }),
	json!({ "type": "number", "maximum": 1 })
)]
#[case::inclusive_maximum_tighter(
	json!({ "type": "number", "exclusiveMaximum": 1, "maximum": 0.5 }),
	json!({ "type": "number", "maximum": 0.5 })
)]
#[case::exclusive_maximum_tighter(
	json!({ "type": "number", "exclusiveMaximum": 1, "maximum": 2 }),
	json!({ "type": "number", "maximum": 1 })
)]
#[case::inclusive_minimum_tighter(
	json!({ "type": "integer", "exclusiveMinimum": 0, "minimum": 1 }),
	json!({ "type": "integer", "minimum": 1 })
)]
#[case::exclusive_minimum_tighter(
	json!({ "type": "integer", "exclusiveMinimum": 5, "minimum": 2 }),
	json!({ "type": "integer", "minimum": 5 })
)]
#[case::draft4_boolean(
	json!({ "type": "integer", "minimum": 0, "exclusiveMinimum": true }),
	json!({ "type": "integer", "minimum": 0 })
)]
fn exclusive_bounds_become_inclusive(#[case] property: Value, #[case] expected: Value) {
	let params = tool_parameters(json!({ "type": "object", "properties": { "x": property } }));
	assert_eq!(params["properties"]["x"], expected);
}

fn tool_parameters(parameters: Value) -> Value {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"tools": [{
			"type": "function",
			"function": { "name": "t", "parameters": parameters }
		}]
	}));
	g["tools"][0]["functionDeclarations"][0]["parameters"].clone()
}

// Case 3: Pydantic wraps a described nested-model field as {allOf:[{$ref}], description} (or a
// $ref with siblings). Gemini supports neither; the single allOf member must be flattened into the
// parent, not dropped (dropping would lose the type).
#[test]
fn gemini_schema_flattens_allof_single_ref() {
	let s = response_schema(json!({
		"$defs": {
			"Inner": {
				"type": "object",
				"additionalProperties": false,
				"properties": { "a": { "type": "string" } }
			}
		},
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"inner": { "allOf": [{ "$ref": "#/$defs/Inner" }], "description": "the inner object" }
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("allOf"),
		"single-member allOf must be flattened: {txt}"
	);
	assert!(
		!txt.contains("$ref"),
		"the ref inside allOf must be inlined: {txt}"
	);
	assert!(!txt.contains("$defs"), "must drop $defs: {txt}");
	assert_eq!(
		s["properties"]["inner"]["type"], "object",
		"flattened type lost: {txt}"
	);
	assert_eq!(
		s["properties"]["inner"]["description"], "the inner object",
		"sibling description must be preserved: {txt}"
	);
}

// Case 3b: a multi-member allOf must merge every member's `properties` and `required` into the
// parent, not keep only the first. JSON-Schema composition (`allOf: [A, B]`) commonly carries
// disjoint property sets; first-wins insertion silently drops all but the first member.
#[test]
fn gemini_schema_merges_multi_member_allof() {
	let s = response_schema(json!({
		"type": "object",
		"allOf": [
			{ "properties": { "a": { "type": "string" } }, "required": ["a"] },
			{ "properties": { "b": { "type": "integer" } }, "required": ["b"] }
		]
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(!txt.contains("allOf"), "allOf must be flattened: {txt}");
	assert_eq!(
		s["properties"]["a"]["type"], "string",
		"first member's property must survive: {txt}"
	);
	assert_eq!(
		s["properties"]["b"]["type"], "integer",
		"second member's property must not be dropped: {txt}"
	);
	let required = s["required"].as_array().expect("required array");
	assert!(
		required.iter().any(|r| r == "a"),
		"first member's required must survive: {txt}"
	);
	assert!(
		required.iter().any(|r| r == "b"),
		"second member's required must be unioned in, not dropped: {txt}"
	);
}

// Case 4: Pydantic `Literal["a"]` emits {const: "a"}. Gemini has no const; preserve the constraint
// as a single-value string enum (litellm drops const, which silently loses it).
#[test]
fn gemini_schema_converts_const_to_enum() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": { "kind": { "const": "a", "title": "Kind" } }
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(!txt.contains("\"const\""), "Gemini rejects const: {txt}");
	assert_eq!(
		s["properties"]["kind"]["enum"][0], "a",
		"const value must be preserved as a single-element enum: {txt}"
	);
	assert_eq!(
		s["properties"]["kind"]["type"], "string",
		"enum needs a string type: {txt}"
	);
}

// Case 4b: JSON Schema `oneOf` is how tagged unions are expressed (zod, TypeBox, OpenAPI). Gemini's
// Schema proto only has `any_of`; it silently ignores `oneOf` rather than rejecting it, and the
// normalizer used to strip it, so the node lost its type and Gemini returned empty objects.
// `oneOf` must be rewritten to `anyOf` with every member intact.
#[test]
fn gemini_schema_rewrites_oneof_to_anyof() {
	let s = response_schema(json!({
		"type": "object",
		"required": ["events"],
		"properties": {
			"events": {
				"type": "array",
				"items": {
					"oneOf": [
						{
							"type": "object",
							"required": ["kind", "x", "y"],
							"properties": {
								"kind": { "type": "string", "enum": ["click"] },
								"x": { "type": "integer" },
								"y": { "type": "integer" }
							}
						},
						{
							"type": "object",
							"required": ["kind", "key"],
							"properties": {
								"kind": { "type": "string", "enum": ["keypress"] },
								"key": { "type": "string" }
							}
						}
					]
				}
			}
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(!txt.contains("oneOf"), "oneOf must be rewritten: {txt}");
	let items = &s["properties"]["events"]["items"];
	let any_of = items["anyOf"]
		.as_array()
		.unwrap_or_else(|| panic!("oneOf must become anyOf, not be dropped: {txt}"));
	assert_eq!(any_of.len(), 2, "both branches must survive: {txt}");
	assert_eq!(any_of[0]["properties"]["kind"]["enum"][0], "click", "{txt}");
	assert_eq!(any_of[0]["properties"]["x"]["type"], "integer", "{txt}");
	assert_eq!(
		any_of[1]["properties"]["kind"]["enum"][0], "keypress",
		"{txt}"
	);
	assert_eq!(any_of[1]["properties"]["key"]["type"], "string", "{txt}");
}

// Case 4c: the same rewrite must apply on the tool-parameters path. This is the exact payload from
// agentgateway/agentgateway#3880, where Gemini answered `{"events":[{},{}]}`.
#[test]
fn tool_parameters_rewrite_oneof_to_anyof() {
	let g = to_gemini(json!({
		"model": "gemini-2.5-flash",
		"tool_choice": "required",
		"messages": [{
			"role": "user",
			"content": "The user clicked at (10, 20), then pressed Enter. Record both events."
		}],
		"tools": [{
			"type": "function",
			"function": {
				"name": "record_events",
				"parameters": {
					"type": "object",
					"required": ["events"],
					"properties": {
						"events": {
							"type": "array",
							"items": {
								"oneOf": [
									{
										"type": "object",
										"required": ["kind", "x", "y"],
										"properties": {
											"kind": { "type": "string", "enum": ["click"] },
											"x": { "type": "integer" },
											"y": { "type": "integer" }
										}
									},
									{
										"type": "object",
										"required": ["kind", "key"],
										"properties": {
											"kind": { "type": "string", "enum": ["keypress"] },
											"key": { "type": "string" }
										}
									}
								]
							}
						}
					}
				}
			}
		}]
	}));
	let params = &g["tools"][0]["functionDeclarations"][0]["parameters"];
	let txt = serde_json::to_string(params).unwrap();
	assert!(!txt.contains("oneOf"), "oneOf must be rewritten: {txt}");
	let items = &params["properties"]["events"]["items"];
	assert_ne!(
		items,
		&json!({}),
		"items must not collapse to an empty schema: {txt}"
	);
	let any_of = items["anyOf"]
		.as_array()
		.unwrap_or_else(|| panic!("oneOf must become anyOf, not be dropped: {txt}"));
	assert_eq!(any_of.len(), 2, "both branches must survive: {txt}");
	assert_eq!(any_of[0]["type"], "object", "{txt}");
	assert_eq!(any_of[1]["type"], "object", "{txt}");
}

// Case 4d: Pydantic emits a discriminated union as `{oneOf: [{$ref}, {$ref}], discriminator: {...}}`.
// The refs inside the rewritten members must still be inlined, the sibling description kept, and
// the unsupported `discriminator` dropped.
#[test]
fn gemini_schema_rewrites_oneof_of_refs() {
	let s = response_schema(json!({
		"$defs": {
			"Click": {
				"type": "object",
				"properties": { "kind": { "const": "click" }, "x": { "type": "integer" } },
				"required": ["kind", "x"]
			},
			"Keypress": {
				"type": "object",
				"properties": { "kind": { "const": "keypress" }, "key": { "type": "string" } },
				"required": ["kind", "key"]
			}
		},
		"type": "object",
		"properties": {
			"event": {
				"oneOf": [{ "$ref": "#/$defs/Click" }, { "$ref": "#/$defs/Keypress" }],
				"discriminator": { "propertyName": "kind" },
				"description": "what happened"
			}
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(!txt.contains("oneOf"), "oneOf must be rewritten: {txt}");
	assert!(
		!txt.contains("$ref"),
		"refs inside oneOf must be inlined: {txt}"
	);
	assert!(!txt.contains("$defs"), "must drop $defs: {txt}");
	assert!(
		!txt.contains("discriminator"),
		"Gemini has no discriminator: {txt}"
	);
	let event = &s["properties"]["event"];
	assert_eq!(
		event["description"], "what happened",
		"sibling description must be preserved: {txt}"
	);
	let any_of = event["anyOf"]
		.as_array()
		.unwrap_or_else(|| panic!("oneOf must become anyOf, not be dropped: {txt}"));
	assert_eq!(any_of.len(), 2, "both branches must survive: {txt}");
	assert_eq!(any_of[0]["properties"]["kind"]["enum"][0], "click", "{txt}");
	assert_eq!(any_of[0]["properties"]["x"]["type"], "integer", "{txt}");
	assert_eq!(
		any_of[1]["properties"]["kind"]["enum"][0], "keypress",
		"{txt}"
	);
	assert_eq!(any_of[1]["properties"]["key"]["type"], "string", "{txt}");
}

// Case 5: only `enum` and `date-time` string formats are safe; others (uri, email, int64, uuid, ...)
// must be stripped.
#[test]
fn gemini_schema_strips_unsupported_formats_keeps_datetime() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"link": { "type": "string", "format": "uri" },
			"when": { "type": "string", "format": "date-time" },
			"big": { "type": "integer", "format": "int64" }
		}
	}));
	assert!(
		s["properties"]["link"].get("format").is_none(),
		"unsupported string format must be dropped: {s}"
	);
	assert_eq!(
		s["properties"]["when"]["format"], "date-time",
		"date-time format must be kept: {s}"
	);
	assert!(
		s["properties"]["big"].get("format").is_none(),
		"unsupported numeric format must be dropped: {s}"
	);
}

// Case 6: JSON Schema `type` arrays are not allowed. A `null` member becomes `nullable`; a genuine
// union becomes `anyOf`.
#[test]
fn gemini_schema_normalizes_multitype_arrays() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"u": { "type": ["string", "integer"] },
			"o": { "type": ["string", "null"] }
		}
	}));
	assert!(
		!s["properties"]["u"]["type"].is_array(),
		"a multi-type union must not stay a type array: {s}"
	);
	assert!(
		s["properties"]["u"].get("anyOf").is_some(),
		"a multi-type union should become anyOf: {s}"
	);
	assert_eq!(
		s["properties"]["o"]["type"], "string",
		"null member should drop to a single type: {s}"
	);
	assert_eq!(
		s["properties"]["o"]["nullable"], true,
		"null member should set nullable: {s}"
	);
}

// Case 7: Gemini requires `items` on arrays; a bare array (List[Any]) must get a default item schema.
#[test]
fn gemini_schema_array_without_items_gets_items() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": { "tags": { "type": "array" } }
	}));
	assert!(
		s["properties"]["tags"].get("items").is_some(),
		"array must have items: {s}"
	);
}

// Case 8: minItems/maxItems on array types must be preserved so callers can enforce response
// length constraints (e.g. "return exactly 3 items"). They were previously stripped because they
// were absent from ALLOWED_SCHEMA_FIELDS.
#[test]
fn gemini_schema_preserves_array_length_constraints() {
	let s = response_schema(json!({
		"type": "object",
		"properties": {
			"tags": {
				"type": "array",
				"items": { "type": "string" },
				"minItems": 2,
				"maxItems": 5
			}
		}
	}));
	assert_eq!(
		s["properties"]["tags"]["minItems"], 2,
		"minItems must be preserved: {s}"
	);
	assert_eq!(
		s["properties"]["tags"]["maxItems"], 5,
		"maxItems must be preserved: {s}"
	);
}

// Case 9: minProperties/maxProperties constrain the number of keys on an object. Like minItems/
// maxItems they were absent from ALLOWED_SCHEMA_FIELDS and were silently stripped.
#[test]
fn gemini_schema_preserves_object_property_count_constraints() {
	let s = response_schema(json!({
		"type": "object",
		"minProperties": 1,
		"maxProperties": 4,
		"properties": { "a": { "type": "string" } }
	}));
	assert_eq!(
		s["minProperties"], 1,
		"minProperties must be preserved: {s}"
	);
	assert_eq!(
		s["maxProperties"], 4,
		"maxProperties must be preserved: {s}"
	);
}

// Case 10: example provides a sample value for a schema node and is passed through to Gemini.
#[test]
fn gemini_schema_preserves_example() {
	let s = response_schema(json!({
		"type": "string",
		"example": "hello"
	}));
	assert_eq!(s["example"], "hello", "example must be preserved: {s}");
}

// Case 11: Dict[str, X] emits a typed `additionalProperties` schema. Gemini supports it, so both
// the boolean false form and the Schema-object form must be preserved.
#[test]
fn gemini_schema_preserves_open_dict_additional_properties() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"meta": { "type": "object", "additionalProperties": { "type": "string" } }
		}
	}));
	assert_eq!(
		s["additionalProperties"], false,
		"top-level additionalProperties: false must be preserved: {s}"
	);
	assert_eq!(
		s["properties"]["meta"]["additionalProperties"]["type"], "string",
		"Schema-object form of additionalProperties must be preserved: {s}"
	);
}

// Case 11b: Dict[str, Model] puts a $ref under additionalProperties. Now that the key survives,
// the subtree must be normalized too — $defs is stripped at the root, so an un-inlined $ref here
// would egress dangling and Vertex 400s.
#[test]
fn gemini_schema_inlines_ref_under_additional_properties() {
	let s = response_schema(json!({
		"$defs": {
			"Person": {
				"type": "object",
				"properties": { "name": { "type": "string" } },
				"required": ["name"]
			}
		},
		"type": "object",
		"properties": {
			"people": { "type": "object", "additionalProperties": { "$ref": "#/$defs/Person" } }
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("$ref"),
		"$ref under additionalProperties must be inlined: {txt}"
	);
	assert_eq!(
		s["properties"]["people"]["additionalProperties"]["properties"]["name"]["type"], "string",
		"the inlined Person must keep its properties: {s}"
	);
}

// Case 11c: Dict[str, Optional[str]] puts an anyOf null branch under additionalProperties. It must
// collapse to nullable like anywhere else; Gemini has no null type.
#[test]
fn gemini_schema_normalizes_nullable_under_additional_properties() {
	let s = response_schema(json!({
		"type": "object",
		"properties": {
			"meta": {
				"type": "object",
				"additionalProperties": { "anyOf": [{ "type": "string" }, { "type": "null" }] }
			}
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("\"type\":\"null\""),
		"null branch under additionalProperties must collapse to nullable: {txt}"
	);
}

// Case 11d: an empty-object additionalProperties means "anything goes" and must stay empty — the
// typeless default must not rewrite it into {"type":"object"}.
#[test]
fn gemini_schema_leaves_empty_additional_properties_alone() {
	let s = response_schema(json!({
		"type": "object",
		"properties": { "meta": { "type": "object", "additionalProperties": {} } }
	}));
	assert_eq!(
		s["properties"]["meta"]["additionalProperties"],
		json!({}),
		"empty additionalProperties must stay empty: {s}"
	);
}

// Case 11e: the Gemini API (generativelanguage.googleapis.com) rejects additionalProperties in
// responseSchema, unlike Vertex AI. With is_vertex=false the key must be stripped.
#[test]
fn gemini_api_response_schema_strips_additional_properties() {
	let g = to_gemini_api(json!({
		"model": "gemini-2.5-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"response_format": {
			"type": "json_schema",
			"json_schema": { "name": "T", "strict": true, "schema": {
				"type": "object",
				"properties": { "answer": { "type": "string", "additionalProperties": false } },
				"required": ["answer"],
				"additionalProperties": false
			}}
		}
	}));
	let s = serde_json::to_string(&g["generationConfig"]["responseSchema"]).unwrap();
	assert!(
		!s.contains("additionalProperties"),
		"Gemini API responseSchema must not contain additionalProperties: {s}"
	);
}

// Case 9: a self-referential model must not make the inliner hang. Recursion cannot be represented
// in Gemini's subset, so $defs is still dropped; the guarantee is termination and a bounded result.
#[test]
fn gemini_schema_recursive_model_terminates() {
	let s = response_schema(json!({
		"$defs": {
			"Node": {
				"type": "object",
				"additionalProperties": false,
				"properties": {
					"value": { "type": "string" },
					"children": { "type": "array", "items": { "$ref": "#/$defs/Node" } }
				}
			}
		},
		"type": "object",
		"additionalProperties": false,
		"properties": { "root": { "$ref": "#/$defs/Node" } }
	}));
	// Reaching this line at all proves the normalizer terminated (no infinite inline loop).
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("$defs"),
		"must drop $defs even for recursive models: {txt}"
	);
}

// Case 10: an object schema that omits `type` must get `type: object`.
#[test]
fn gemini_schema_adds_missing_object_type() {
	let s = response_schema(json!({
		"additionalProperties": false,
		"properties": { "a": { "type": "string" } }
	}));
	assert_eq!(
		s["type"], "object",
		"missing object type must be added: {s}"
	);
}

// Case 11: Gemini's enum applies to string types; an enum on a non-string field must be dropped.
#[test]
fn gemini_schema_drops_enum_on_non_string() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": { "n": { "type": "integer", "enum": [1, 2, 3] } }
	}));
	assert!(
		s["properties"]["n"].get("enum").is_none(),
		"enum on a non-string field must be dropped: {s}"
	);
}

// Optional[Literal["a"]] = {anyOf:[{const:"a"},{type:null}]}. The single-member collapse
// merges `const` into the parent AFTER the const->enum pass already ran, so the literal is dropped by
// the whitelist and the field becomes an object. It must survive as a nullable string enum.
#[test]
fn gemini_schema_anyof_const_member_preserved_as_nullable_enum() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"x": { "anyOf": [{ "const": "a" }, { "type": "null" }], "default": null, "title": "X" }
		}
	}));
	let x = &s["properties"]["x"];
	assert_eq!(
		x["type"], "string",
		"Optional[Literal] must stay a string, not become object: {s}"
	);
	assert_eq!(
		x["enum"][0], "a",
		"the literal value must be preserved: {s}"
	);
	assert_eq!(
		x["nullable"], true,
		"the null branch must become nullable: {s}"
	);
}

// A `type` array inside a collapsed anyOf member escapes the type-array pass (which ran
// before the collapse), shipping an illegal JSON-Schema type array to Gemini.
#[test]
fn gemini_schema_anyof_type_array_member_normalized() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"u": { "anyOf": [{ "type": ["string", "integer"] }, { "type": "null" }] }
		}
	}));
	let u = &s["properties"]["u"];
	assert!(
		!u["type"].is_array(),
		"a type array merged from an anyOf member must be normalized, not shipped: {s}"
	);
	assert_eq!(
		u["nullable"], true,
		"the null branch must become nullable: {s}"
	);
}

// An `allOf` inside a collapsed anyOf member escapes the allOf-flatten (which ran before
// the collapse), so the inlined inner schema is dropped by the whitelist and its fields are lost.
#[test]
fn gemini_schema_anyof_allof_member_flattened() {
	let s = response_schema(json!({
		"$defs": {
			"Inner": {
				"type": "object",
				"additionalProperties": false,
				"properties": { "a": { "type": "string" } }
			}
		},
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"w": { "anyOf": [{ "allOf": [{ "$ref": "#/$defs/Inner" }] }, { "type": "null" }] }
		}
	}));
	let w = &s["properties"]["w"];
	let txt = serde_json::to_string(w).unwrap();
	assert!(
		!txt.contains("allOf"),
		"allOf merged from an anyOf member must be flattened: {s}"
	);
	assert_eq!(
		w["type"], "object",
		"the inlined inner type must survive: {s}"
	);
	assert!(
		w["properties"].get("a").is_some(),
		"the inlined inner properties must survive: {s}"
	);
	assert_eq!(
		w["nullable"], true,
		"the null branch must become nullable: {s}"
	);
}

// A typeless enum ({enum:[...]} with no `type`) is dropped by the enum-on-non-string step
// (which treats an absent type as non-string) and then retyped as an object, losing the constraint.
#[test]
fn gemini_schema_typeless_enum_kept_as_string_enum() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"color": { "enum": ["red", "green"] }
		}
	}));
	let color = &s["properties"]["color"];
	assert_eq!(
		color["type"], "string",
		"a typeless enum must default to a string type, not object: {s}"
	);
	assert_eq!(
		color["enum"][0], "red",
		"the enum values must be preserved: {s}"
	);
}

// A non-string const must be typed by its JSON kind, not forced to `string` (which yields
// an invalid string-typed numeric/boolean enum). A string const stays a string enum.
#[test]
fn gemini_schema_non_string_const_typed_by_value_kind() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"i": { "const": 5 },
			"b": { "const": true },
			"str": { "const": "x" }
		}
	}));
	assert_eq!(
		s["properties"]["i"]["type"], "integer",
		"integer const must be typed integer, not string: {s}"
	);
	assert_eq!(
		s["properties"]["b"]["type"], "boolean",
		"boolean const must be typed boolean, not string: {s}"
	);
	assert_eq!(
		s["properties"]["str"]["type"], "string",
		"string const stays string: {s}"
	);
	assert_eq!(
		s["properties"]["str"]["enum"][0], "x",
		"string const preserved as enum: {s}"
	);
}

#[rstest::rstest]
#[case::none_takes_the_lowest_level("gemini-3-flash-preview", true, Some(("gcp.vertex_ai", &["minimal", "low", "medium", "high"][..])), "none", Some(json!({ "thinkingLevel": "minimal", "includeThoughts": true })))]
#[case::none_falls_back_to_low("gemini-3.8-flash", true, Some(("gcp.vertex_ai", &["low", "medium", "high"][..])), "none", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::no_catalog_none("gemini-3.1-pro-preview", false, None, "none", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::none_on_a_known_model_without_levels("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &[][..])), "none", None)]
#[case::none_on_a_known_budget_only_model("gemini-2.5-flash", true, Some(("gcp.vertex_ai", &[][..])), "none", Some(json!({ "thinkingBudget": 0 })))]
#[case::none_turns_flash_off("gemini-2.5-flash", true, None, "none", Some(json!({ "thinkingBudget": 0 })))]
#[case::none_turns_flash_lite_off("gemini-2.5-flash-lite", false, None, "none", Some(json!({ "thinkingBudget": 0 })))]
#[case::none_on_pro_is_its_minimum_budget("gemini-2.5-pro", false, None, "none", Some(json!({ "thinkingBudget": 128, "includeThoughts": true })))]
#[case::none_on_a_dated_preview("gemini-2.5-flash-lite-preview-06-17", true, None, "none", Some(json!({ "thinkingBudget": 0 })))]
#[case::none_on_a_publisher_path("publishers/google/models/gemini-2.5-pro", true, None, "none", Some(json!({ "thinkingBudget": 128, "includeThoughts": true })))]
#[case::none_on_an_image_model("gemini-2.5-flash-image", false, None, "none", None)]
#[case::none_on_a_tts_model("gemini-2.5-flash-preview-tts", false, None, "none", None)]
#[case::none_on_another_model("gemma-3-27b-it", false, None, "none", None)]
#[case::minimal_kept("gemini-3-flash-preview", true, Some(("gcp.vertex_ai", &["minimal", "low", "medium", "high"][..])), "minimal", Some(json!({ "thinkingLevel": "minimal", "includeThoughts": true })))]
#[case::minimal_rounds_up("gemini-3.8-flash", true, Some(("gcp.vertex_ai", &["low", "medium", "high"][..])), "minimal", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::max_is_high("gemini-3.8-flash", true, Some(("gcp.vertex_ai", &["low", "medium", "high"][..])), "max", Some(json!({ "thinkingLevel": "high", "includeThoughts": true })))]
#[case::gemini_4_gets_a_level("gemini-4-argon", true, Some(("gcp.vertex_ai", &["low", "medium", "high"][..])), "high", Some(json!({ "thinkingLevel": "high", "includeThoughts": true })))]
#[case::gemini_api_provider("gemini-4-argon", false, Some(("gcp.gemini", &["low", "medium", "high"][..])), "low", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::unusable_catalog_list("gemini-4-argon", true, Some(("gcp.vertex_ai", &["max"][..])), "high", None)]
#[case::known_without_levels_low("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &[][..])), "low", None)]
#[case::known_without_levels_medium("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &[][..])), "medium", None)]
#[case::known_without_levels_high("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &[][..])), "high", None)]
#[case::only_high_from_low("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &["high"][..])), "low", Some(json!({ "thinkingLevel": "high", "includeThoughts": true })))]
#[case::only_high_from_medium("gemini-3-pro-image", true, Some(("gcp.vertex_ai", &["high"][..])), "medium", Some(json!({ "thinkingLevel": "high", "includeThoughts": true })))]
#[case::mixed_catalog_list("gemini-4-argon", true, Some(("gcp.vertex_ai", &["low", "max"][..])), "high", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::other_provider_ignored("gemini-3-flash-preview", true, Some(("gcp.gemini", &["minimal", "low", "medium", "high"][..])), "minimal", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::no_catalog_minimal("gemini-3.8-flash", true, None, "minimal", Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::no_catalog_gemini_4("gemini-4-argon", true, None, "high", Some(json!({ "thinkingLevel": "high", "includeThoughts": true })))]
#[case::budget_high("gemini-2.5-flash", true, None, "high", Some(json!({ "thinkingBudget": 4096, "includeThoughts": true })))]
#[case::budget_xhigh("gemini-2.5-flash", true, None, "xhigh", Some(json!({ "thinkingBudget": 8192, "includeThoughts": true })))]
#[case::budget_max("gemini-2.5-flash", true, None, "max", Some(json!({ "thinkingBudget": 16384, "includeThoughts": true })))]
#[case::budget_only_catalog_entry("gemini-2.5-pro", true, Some(("gcp.vertex_ai", &[][..])), "high", Some(json!({ "thinkingBudget": 4096, "includeThoughts": true })))]
fn thinking_config(
	#[case] model: &str,
	#[case] is_vertex: bool,
	#[case] entry: Option<(&str, &[&str])>,
	#[case] effort: &str,
	#[case] expected: Option<Value>,
) {
	let catalog = entry.map(|(provider, efforts)| {
		let efforts: Vec<_> = efforts
			.iter()
			.map(|e| serde_json::from_value(json!(e)).unwrap())
			.collect();
		crate::model_catalog::TestCatalog::new([]).with_efforts(provider, model, efforts)
	});
	let request = req(json!({
		"model": model,
		"messages": [{ "role": "user", "content": "x" }],
		"reasoning_effort": effort
	}));
	let bytes = from_completions::translate(
		&request,
		is_vertex,
		catalog
			.as_ref()
			.map(|c| c as &dyn crate::model_catalog::ModelCatalogHandle),
	)
	.unwrap();
	let g: Value = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(
		g["generationConfig"].get("thinkingConfig").cloned(),
		expected
	);
}

#[test]
fn client_thinking_config_wins_over_the_catalog() {
	let catalog = crate::model_catalog::TestCatalog::new([]).with_efforts(
		"gcp.vertex_ai",
		"gemini-3.8-flash",
		vec![crate::types::completions::typed::ReasoningEffort::Low],
	);
	let request = req(json!({
		"model": "gemini-3.8-flash",
		"messages": [{ "role": "user", "content": "x" }],
		"reasoning_effort": "low",
		"thinkingConfig": { "thinkingLevel": "high" }
	}));
	let bytes = from_completions::translate(&request, true, Some(&catalog)).unwrap();
	let g: Value = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(
		g["generationConfig"]["thinkingConfig"],
		json!({ "thinkingLevel": "high" })
	);
}

// ---------- Request: Messages and Responses through Chat Completions ----------

fn messages_tool_replay(call_id: &str) -> Vec<u8> {
	let req: types::messages::Request = serde_json::from_value(json!({
		"model": "gemini-3.8-flash",
		"max_tokens": 256,
		"messages": [
			{ "role": "user", "content": "Weather in Columbus?" },
			{ "role": "assistant", "content": [
				{ "type": "tool_use", "id": call_id, "name": "get_weather", "input": { "location": "Columbus" } }
			] },
			{ "role": "user", "content": [
				{ "type": "tool_result", "tool_use_id": "call_1", "content": "15C" }
			] }
		],
		"tools": [{ "name": "get_weather", "input_schema": { "type": "object" } }]
	}))
	.unwrap();
	from_messages::translate(&req, true, None).unwrap()
}

fn responses_tool_replay(call_id: &str) -> Vec<u8> {
	let req: types::responses::Request = serde_json::from_value(json!({
		"model": "gemini-3.8-flash",
		"input": [
			{ "role": "user", "content": "Weather in Columbus?" },
			{ "type": "function_call", "call_id": call_id, "name": "get_weather", "arguments": "{\"location\":\"Columbus\"}" },
			{ "type": "function_call_output", "call_id": "call_1", "output": "15C" }
		],
		"tools": [{ "type": "function", "name": "get_weather", "parameters": { "type": "object" } }]
	}))
	.unwrap();
	from_responses::translate(&req, true, None).unwrap().body
}

#[rstest::rstest]
fn replayed_signature_survives_the_chain(
	#[values(messages_tool_replay, responses_tool_replay)] replay: fn(&str) -> Vec<u8>,
) {
	let signature = "AY89".repeat(1024);
	let bytes = replay(&format!("call_1__thought__{signature}"));
	let g: Value = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(
		g["contents"][1]["parts"][0],
		json!({
			"functionCall": { "name": "get_weather", "args": { "location": "Columbus" } },
			"thoughtSignature": signature
		})
	);
	assert_eq!(
		g["contents"][2]["parts"][0]["functionResponse"]["name"],
		"get_weather"
	);
	assert!(!String::from_utf8(bytes).unwrap().contains("__thought__"));
}

/// Explicit off uses the model's minimum; omitting it keeps the default
#[rstest::rstest]
#[case::flash("gemini-2.5-flash", true, Some(json!({ "thinkingBudget": 0 })))]
#[case::gemini_3("gemini-3.8-flash", true, Some(json!({ "thinkingLevel": "low", "includeThoughts": true })))]
#[case::omitted("gemini-2.5-flash", false, None)]
fn an_explicit_off_reaches_gemini(
	#[values("messages", "responses")] client: &str,
	#[case] model: &str,
	#[case] off: bool,
	#[case] expected: Option<Value>,
) {
	let body = match client {
		"messages" => {
			let mut req = json!({
				"model": model,
				"max_tokens": 256,
				"messages": [{ "role": "user", "content": "hi" }]
			});
			if off {
				req["thinking"] = json!({ "type": "disabled" });
			}
			from_messages::translate(&serde_json::from_value(req).unwrap(), true, None).unwrap()
		},
		_ => {
			let mut req = json!({ "model": model, "input": "hi" });
			if off {
				req["reasoning"] = json!({ "effort": "none" });
			}
			from_responses::translate(&serde_json::from_value(req).unwrap(), true, None)
				.unwrap()
				.body
		},
	};
	let g: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		g["generationConfig"].get("thinkingConfig").cloned(),
		expected
	);
}

#[test]
fn messages_top_k_is_dropped_and_the_budget_becomes_an_effort() {
	let req: types::messages::Request = serde_json::from_value(json!({
		"model": "gemini-2.5-flash",
		"max_tokens": 8192,
		"top_k": 40,
		"thinking": { "type": "enabled", "budget_tokens": 3000 },
		"messages": [{ "role": "user", "content": "hi" }]
	}))
	.unwrap();
	let bytes = from_messages::translate(&req, true, None).unwrap();
	let g: Value = serde_json::from_slice(&bytes).unwrap();
	assert_eq!(g["generationConfig"].get("topK"), None);
	assert_eq!(
		g["generationConfig"]["thinkingConfig"],
		json!({ "thinkingBudget": 2048, "includeThoughts": true })
	);
}

#[rstest::rstest]
fn messages_rejects_tool_result_parts_it_would_drop(
	#[values(
		json!({ "type": "image", "source": { "type": "url", "url": "https://example.com/result" } }),
		json!({ "type": "document", "source": { "type": "url", "url": "https://example.com/result" } }),
		json!({
			"type": "search_result",
			"source": "https://example.com/result",
			"title": "Result",
			"content": [{ "type": "text", "text": "Paris is 21°C." }]
		})
	)]
	part: Value,
	#[values(false, true)] is_vertex: bool,
) {
	let mut input: Value = serde_json::from_str(include_str!(
		"../tests/requests/messages/tool_result_image.json"
	))
	.unwrap();
	input["model"] = json!("gemini-3.8-flash");
	input["messages"][2]["content"][0]["content"] =
		json!([{ "type": "text", "text": "Tool output:" }, part]);
	let request = serde_json::from_value(input).unwrap();
	let error = from_messages::translate(&request, is_vertex, None).unwrap_err();
	assert!(matches!(error, AIError::UnsupportedConversion(_)));
}

#[rstest::rstest]
fn responses_rejects_tool_output_parts_it_would_drop(
	#[values(
		json!({ "type": "input_image", "image_url": "https://example.com/result.png" }),
		json!({ "type": "input_file", "file_id": "file_1" })
	)]
	part: Value,
	#[values(false, true)] is_vertex: bool,
) {
	let error = from_responses::translate(&tool_output_request(part), is_vertex, None)
		.err()
		.expect("media output is rejected");
	assert!(matches!(error, AIError::UnsupportedConversion(_)));
}

#[test]
fn responses_keeps_text_tool_output() {
	let request = tool_output_request(json!({ "type": "input_text", "text": "15C" }));
	let g: Value = serde_json::from_slice(
		&from_responses::translate(&request, true, None)
			.unwrap()
			.body,
	)
	.unwrap();
	assert_eq!(
		g["contents"][2]["parts"][0]["functionResponse"]["response"],
		json!({ "content": "Tool output:\n15C" })
	);
}

fn tool_output_request(part: Value) -> types::responses::Request {
	serde_json::from_value(json!({
		"model": "gemini-3.8-flash",
		"input": [
			{ "role": "user", "content": "Weather in Columbus?" },
			{ "type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}" },
			{ "type": "function_call_output", "call_id": "call_1", "output": [
				{ "type": "input_text", "text": "Tool output:" },
				part
			] }
		]
	}))
	.unwrap()
}

#[test]
fn only_decoded_carriers_are_consumed() {
	let mut content: Vec<types::messages::typed::ContentBlock> = serde_json::from_value(json!([
		{ "type": "thinking", "thinking": "plan", "signature": "genuine" },
		{ "type": "thinking", "thinking": "", "signature": ThoughtSignature::encode("call_1", "sig") },
		{ "type": "tool_use", "id": "call_1", "name": "f", "input": {} },
		{ "type": "tool_use", "id": "call_2", "name": "f", "input": {} }
	]))
	.unwrap();
	from_messages::restore_signatures(&mut content);
	assert_eq!(
		serde_json::to_value(&content).unwrap(),
		json!([
			{ "type": "thinking", "thinking": "plan", "signature": "genuine" },
			{ "type": "tool_use", "id": "call_1__thought__sig", "name": "f", "input": {} },
			{ "type": "tool_use", "id": "call_2", "name": "f", "input": {} }
		])
	);
}

#[test]
fn responses_carriers_apply_within_their_turn() {
	let mut input: Vec<types::responses::typed::InputItem> = serde_json::from_value(json!([
		{ "type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "genuine" },
		{ "type": "reasoning", "id": "rs_2", "summary": [], "content": null,
			"encrypted_content": ThoughtSignature::encode("call_1", "sig") },
		{ "type": "function_call", "call_id": "call_1", "name": "f", "arguments": "{}" },
		{ "type": "function_call_output", "call_id": "call_1", "output": "ok" },
		{ "type": "function_call", "call_id": "call_1", "name": "f", "arguments": "{}" }
	]))
	.unwrap();
	from_responses::restore_signatures(&mut input);
	let input = serde_json::to_value(&input).unwrap();
	let summary: Vec<_> = input
		.as_array()
		.unwrap()
		.iter()
		.map(|i| (&i["type"], i.get("call_id").or(i.get("encrypted_content"))))
		.collect();
	assert_eq!(
		summary,
		[
			(&json!("reasoning"), Some(&json!("genuine"))),
			(
				&json!("function_call"),
				Some(&json!("call_1__thought__sig"))
			),
			(&json!("function_call_output"), Some(&json!("call_1"))),
			(&json!("function_call"), Some(&json!("call_1"))),
		]
	);
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
fn response_reasoning_keys_off_thought_flag_only() {
	// Reasoning is identified solely by `thought: true`. A plain text part is content even if it
	// happens to start with "THOUGHT:" (no literal-prefix heuristic).
	let r = resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [
			{ "text": "the plan", "thought": true },
			{ "text": "THOUGHT: not a marker" }
		]}, "finishReason": "STOP" }]
	}));
	assert_eq!(r["choices"][0]["message"]["reasoning_content"], "the plan");
	assert_eq!(
		r["choices"][0]["message"]["content"], "THOUGHT: not a marker",
		"a literal THOUGHT: prefix without the thought flag must stay as content"
	);
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

// ---------- Response: finishReason table / usage ----------

#[rstest::rstest]
#[case("STOP", "stop")]
#[case("MAX_TOKENS", "length")]
#[case("CONTINUATION", "length")]
#[case("SAFETY", "content_filter")]
#[case("RECITATION", "content_filter")]
#[case("LANGUAGE", "content_filter")]
#[case("BLOCKLIST", "content_filter")]
#[case("PROHIBITED_CONTENT", "content_filter")]
#[case("SPII", "content_filter")]
#[case("IMAGE_SAFETY", "content_filter")]
#[case("IMAGE_PROHIBITED_CONTENT", "content_filter")]
#[case("IMAGE_RECITATION", "content_filter")]
#[case("MODEL_ARMOR", "content_filter")]
#[case("ESCALATION", "content_filter")]
fn finish_reasons(#[case] gemini: &str, #[case] openai: &str) {
	let reply = json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": gemini }]
	});
	assert_eq!(resp(reply.clone())["choices"][0]["finish_reason"], openai);
	assert!(to_completions::failure(&gemini_response_bytes(reply)).is_none());
}

#[rstest::rstest]
fn a_tool_call_reply_finishes_with_tool_calls(#[values(Some("STOP"), None)] reason: Option<&str>) {
	let mut cand = json!({ "content": { "role": "model", "parts": [
		{ "functionCall": { "name": "a", "args": {} } }
	]}});
	if let Some(reason) = reason {
		cand["finishReason"] = reason.into();
	}
	let r = resp(json!({ "candidates": [cand] }));
	assert_eq!(r["choices"][0]["finish_reason"], "tool_calls");
}

#[rstest::rstest]
#[case("MALFORMED_FUNCTION_CALL", 502, "INTERNAL")]
#[case("UNEXPECTED_TOOL_CALL", 502, "INTERNAL")]
#[case("TOO_MANY_TOOL_CALLS", 502, "INTERNAL")]
#[case("MALFORMED_RESPONSE", 502, "INTERNAL")]
#[case("OTHER", 502, "INTERNAL")]
#[case("NO_IMAGE", 502, "INTERNAL")]
#[case("IMAGE_OTHER", 502, "INTERNAL")]
#[case("FINISH_REASON_UNSPECIFIED", 502, "INTERNAL")]
#[case("SOME_FUTURE_VALUE", 502, "INTERNAL")]
#[case("MISSING_THOUGHT_SIGNATURE", 400, "INVALID_ARGUMENT")]
#[case("PUP_LIMITED_DISABLED", 403, "PERMISSION_DENIED")]
fn failed_replies_become_errors(#[case] gemini: &str, #[case] code: u16, #[case] status: &str) {
	let reply = json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": gemini, "finishMessage": "details" }]
	});
	let (got, body) = to_completions::failure(&gemini_response_bytes(reply)).expect("a failure");
	assert_eq!(got.as_u16(), code);
	assert_eq!(
		serde_json::from_slice::<Value>(&body).unwrap(),
		json!({ "error": {
			"code": code,
			"message": format!("Gemini stopped with {gemini}: details"),
			"status": status,
		}})
	);
}

#[test]
fn a_failure_without_a_finish_message_names_the_reason() {
	let reply = json!({ "candidates": [{ "finishReason": "OTHER" }] });
	let (_, body) = to_completions::failure(&gemini_response_bytes(reply)).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(body["error"]["message"], "Gemini stopped with OTHER");
}

#[test]
fn a_blocked_reply_is_a_refusal() {
	let reply = |extra: Value| {
		let mut cand = json!({ "finishReason": "SAFETY" });
		cand
			.as_object_mut()
			.unwrap()
			.extend(extra.as_object().unwrap().clone());
		resp(json!({ "candidates": [cand] }))["choices"][0]["message"]["refusal"].clone()
	};
	assert_eq!(
		reply(json!({ "finishMessage": "Unsafe reply" })),
		"Unsafe reply"
	);
	assert_eq!(reply(json!({})), "Gemini blocked the reply: SAFETY");
	assert_eq!(
		resp(
			json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": "STOP" }] })
		)["choices"][0]["message"]["refusal"],
		Value::Null
	);
}

#[test]
fn a_blocked_prompt_is_a_refusal() {
	let refusal = |feedback: Value| {
		resp(json!({ "candidates": [], "promptFeedback": feedback }))["choices"][0]["message"]
			["refusal"]
			.clone()
	};
	assert_eq!(
		refusal(json!({ "blockReason": "SAFETY", "blockReasonMessage": "Unsafe prompt" })),
		"Unsafe prompt"
	);
	assert_eq!(
		refusal(json!({ "blockReason": "SAFETY" })),
		"Gemini blocked the prompt: SAFETY"
	);
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
fn cel_usage_fields_match_usage_metadata() {
	// The CEL/log token fields (via to_llm_response) come from Gemini's usageMetadata, with
	// output normalized to the cross-provider convention: candidates + thoughts (Gemini
	// reports them disjointly, other providers include reasoning in output).
	let r = llm_resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": "STOP" }],
		"usageMetadata": {
			"promptTokenCount": 100, "candidatesTokenCount": 50, "totalTokenCount": 170,
			"cachedContentTokenCount": 30, "thoughtsTokenCount": 20
		}
	}));
	assert_eq!(r.input_tokens, Some(100));
	assert_eq!(r.output_tokens, Some(70));
	assert_eq!(r.total_tokens, Some(170));
	assert_eq!(r.cached_input_tokens, Some(30));
	assert_eq!(r.reasoning_tokens, Some(20));
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
fn streaming_tool_call_id_embeds_thought_signature() {
	// Streaming must embed each functionCall's thoughtSignature into its tool_call id exactly like
	// the non-streaming path, or the next turn 400s ("Function call is missing a thought_signature
	// ... position N"). Parallel calls in one chunk mirror the production failure. Cf. litellm #16895.
	let sig0 = "STREAM_SIG_ZERO_abc123==";
	let sig1 = "STREAM_SIG_ONE_def456==";
	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "content": { "role": "model", "parts": [
			{ "functionCall": { "name": "get_weather", "args": { "city": "Columbus" } },
				"thoughtSignature": sig0 },
			{ "functionCall": { "name": "get_weather", "args": { "city": "Berlin" } },
				"thoughtSignature": sig1 }
		]}}]}),
	)
	.unwrap();
	let tcs = c["choices"][0]["delta"]["tool_calls"]
		.as_array()
		.expect("tool_calls");
	assert_eq!(tcs.len(), 2);
	let id0 = tcs[0]["id"].as_str().expect("id0");
	let id1 = tcs[1]["id"].as_str().expect("id1");
	assert!(
		id0.contains(sig0),
		"first streamed tool_call id must embed its signature, got {id0:?}"
	);
	assert!(
		id1.contains(sig1),
		"second streamed tool_call id must embed its signature, got {id1:?}"
	);
}

#[test]
fn streaming_blocks_are_refusals() {
	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({ "candidates": [{ "finishReason": "SAFETY", "finishMessage": "Unsafe reply" }] }),
	)
	.unwrap();
	assert_eq!(c["choices"][0]["finish_reason"], "content_filter");
	assert_eq!(c["choices"][0]["delta"]["refusal"], "Unsafe reply");

	let mut s = to_completions::StreamState::new();
	let c = stream_chunk(
		&mut s,
		json!({ "promptFeedback": { "blockReason": "PROHIBITED_CONTENT" } }),
	)
	.unwrap();
	assert_eq!(c["choices"][0]["finish_reason"], "content_filter");
	assert_eq!(
		c["choices"][0]["delta"]["refusal"],
		"Gemini blocked the prompt: PROHIBITED_CONTENT"
	);
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
	// completion_tokens includes thoughts (OpenAI semantics); the breakdown stays in details.
	assert_eq!(c["usage"]["completion_tokens"], 3);
	assert_eq!(c["usage"]["total_tokens"], 7);
	assert_eq!(
		c["usage"]["completion_tokens_details"]["reasoning_tokens"],
		1
	);
	assert_eq!(c["usage"]["prompt_tokens_details"]["cached_tokens"], 3);
}

#[test]
fn streaming_usage_suppressed_on_interim_content_chunks() {
	let mut s = to_completions::StreamState::new();
	// Real Gemini shape: cumulative usageMetadata rides on an interim content chunk. The client must
	// not see usage there, or clients that sum per-chunk usage over-count.
	let c1 = stream_chunk(
		&mut s,
		json!({
			"candidates": [{ "content": { "role": "model", "parts": [{ "text": "hi" }] } }],
			"usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 1, "totalTokenCount": 6 }
		}),
	)
	.unwrap();
	assert!(
		c1["usage"].is_null(),
		"interim content chunk must not carry usage"
	);
	assert_eq!(c1["choices"][0]["delta"]["content"], "hi");

	// The final chunk (carrying finish_reason) surfaces the single, final cumulative usage.
	let c2 = stream_chunk(
		&mut s,
		json!({
			"candidates": [{ "content": { "role": "model", "parts": [{ "text": "!" }] },
				"finishReason": "STOP" }],
			"usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 2, "totalTokenCount": 7 }
		}),
	)
	.unwrap();
	assert_eq!(c2["usage"]["total_tokens"], 7);
	assert_eq!(c2["choices"][0]["finish_reason"], "stop");
}

// ---------- Native Gemini inbound: SSE passthrough with usage extraction ----------

mod passthrough {
	use std::sync::{Arc, Mutex};

	use http_body_util::BodyExt;

	use super::*;
	use crate::{
		CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields,
		StreamingUsageGuard, StreamingUsageReporter,
	};

	struct Capture(Arc<Mutex<LLMInfo>>);

	impl StreamingUsageReporter for Capture {
		fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
			f(&mut self.0.lock().unwrap())
		}
		fn report_usage(&mut self) {}
	}

	pub(super) fn captured_info() -> Arc<Mutex<LLMInfo>> {
		Arc::new(Mutex::new(LLMInfo {
			request: LLMRequest {
				input_tokens: None,
				input_format: InputFormat::Gemini,
				cache_convention: CacheTokenConvention::pending(),
				request_model: "gemini-2.5-flash".into(),
				provider: "gcp.vertex_ai".into(),
				streaming: true,
				params: Default::default(),
				prompt: None,
				provider_state: None,
			},
			response: LLMResponse::default(),
		}))
	}

	#[tokio::test]
	async fn passthrough_stream_forwards_bytes_and_extracts_final_usage() {
		// Cumulative usageMetadata on every chunk; the last SSE event carries the totals.
		let input = concat!(
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"reason\",\"thought\":true},{\"text\":\"Hel\"}]}}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8},",
			"\"modelVersion\":\"gemini-2.5-flash\",\"responseId\":\"r1\",\"someNewField\":true}\n\n",
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"lo\"}]},\"finishReason\":\"STOP\"}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":4,\"totalTokenCount\":13,\"thoughtsTokenCount\":2},",
			"\"modelVersion\":\"gemini-2.5-flash\",\"responseId\":\"r1\"}\n\n",
		);
		let captured = captured_info();
		let out = passthrough_stream(
			agent_http::Body::from(input),
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

		assert_eq!(
			out.as_ref(),
			input.as_bytes(),
			"stream must pass through byte-for-byte"
		);
		let info = captured.lock().unwrap();
		assert_eq!(info.response.input_tokens, Some(7));
		assert_eq!(
			info.response.output_tokens,
			Some(6),
			"last event wins; candidates + thoughts"
		);
		assert_eq!(info.response.total_tokens, Some(13));
		assert_eq!(info.response.reasoning_tokens, Some(2));
		assert_eq!(
			info.response.provider_model.as_deref(),
			Some("gemini-2.5-flash")
		);
		assert!(info.response.first_token.is_some());
		assert_eq!(
			info.response.completion,
			Some(vec!["Hello".to_string()]),
			"visible text accumulates across chunks, thought text excluded"
		);
	}

	#[tokio::test]
	async fn passthrough_stream_keeps_usage_from_before_disconnect() {
		// Simulates an early cut: only the first (interim) event arrives.
		let input = concat!(
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hi\"}]}}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8}}\n\n",
		);
		let captured = captured_info();
		let out = passthrough_stream(
			agent_http::Body::from(input),
			1024 * 1024,
			StreamingUsageGuard::new(Box::new(Capture(captured.clone()))),
			LogContentFields::default(),
		)
		.collect()
		.await
		.expect("collect stream")
		.to_bytes();

		assert_eq!(out.as_ref(), input.as_bytes());
		let info = captured.lock().unwrap();
		assert_eq!(info.response.input_tokens, Some(7));
		assert_eq!(info.response.output_tokens, Some(1));
		assert_eq!(info.response.total_tokens, Some(8));
		assert_eq!(
			info.response.completion, None,
			"completion logging disabled"
		);
	}

	fn body_from_frames(frames: &[&str]) -> agent_http::Body {
		let frames: Vec<Result<bytes::Bytes, std::convert::Infallible>> = frames
			.iter()
			.map(|f| Ok(bytes::Bytes::copy_from_slice(f.as_bytes())))
			.collect();
		agent_http::Body::from_stream(futures_util::stream::iter(frames))
	}

	async fn run_passthrough(body: agent_http::Body, captured: &Arc<Mutex<LLMInfo>>) -> bytes::Bytes {
		passthrough_stream(
			body,
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
		.to_bytes()
	}

	/// Real Vertex chunking splits SSE events across TCP frames; the passthrough must reassemble
	/// events for usage extraction without altering what the client receives.
	#[tokio::test]
	async fn passthrough_stream_reassembles_events_split_across_frames() {
		let frames = [
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hel\"}]}}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8},",
			"\"modelVersion\":\"gemini-2.5-flash\"}\n",
			"\ndata: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[",
			"{\"functionCall\":{\"name\":\"get_weather\",\"id\":\"fc_1\",\"args\":{\"city\":\"Berlin\"}},",
			"\"thoughtSignature\":\"sig\"},{\"text\":\"lo\"}]},\"finishReason\":\"STOP\"}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":5,\"totalTokenCount\":12}}\n\n",
		];
		let captured = captured_info();
		let out = run_passthrough(body_from_frames(&frames), &captured).await;

		assert_eq!(out.as_ref(), frames.concat().as_bytes());
		let info = captured.lock().unwrap();
		assert_eq!(info.response.output_tokens, Some(5));
		assert_eq!(info.response.total_tokens, Some(12));
		assert_eq!(info.response.completion, Some(vec!["Hello".to_string()]));
	}

	/// An unparseable event must not poison the stream: the bytes still reach the client and the
	/// usage from the events around it is still recorded.
	#[tokio::test]
	async fn passthrough_stream_tolerates_malformed_events() {
		let input = concat!(
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hi\"}]}}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8}}\n\n",
			"data: {not json at all\n\n",
			"data: {\"candidates\":[{\"finishReason\":\"STOP\"}],",
			"\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n",
		);
		let captured = captured_info();
		let out = run_passthrough(agent_http::Body::from(input), &captured).await;

		assert_eq!(out.as_ref(), input.as_bytes());
		let info = captured.lock().unwrap();
		assert_eq!(info.response.total_tokens, Some(9), "last valid event wins");
		assert_eq!(info.response.completion, Some(vec!["Hi".to_string()]));
	}

	/// A body that never completes an SSE event — the JSON-array variant Google serves without
	/// `alt=sse`, or a stream cut mid-event — surfaces as a stream error at EOF rather than as a
	/// silently-truncated success. Same strict `json_passthrough` behaviour as the completions,
	/// messages, and responses passthroughs; requests are what keep it unreachable, since
	/// `process_gemini_request` rejects `:streamGenerateContent` without `alt=sse` with a 400.
	#[tokio::test]
	async fn passthrough_stream_errors_on_bodies_that_are_not_sse() {
		let non_sse = r#"[{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]}}]}]"#;
		let truncated = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\"";
		for input in [non_sse, truncated] {
			let captured = captured_info();
			let err = passthrough_stream(
				agent_http::Body::from(input),
				1024 * 1024,
				StreamingUsageGuard::new(Box::new(Capture(captured.clone()))),
				LogContentFields::default(),
			)
			.collect()
			.await
			.expect_err("a body with no complete SSE event must not report success");
			assert!(
				err.to_string().contains("unexpected end of stream"),
				"{err}"
			);
			assert_eq!(captured.lock().unwrap().response.total_tokens, None);
		}
	}
}

// ---------- Messages and Responses streams through Chat Completions ----------

mod chain {
	use std::sync::{Arc, Mutex};
	use std::time::Duration;

	use agent_http::Body;
	use http_body_util::BodyExt;

	use super::passthrough::captured_info;
	use super::*;
	use crate::conversion::stream_tests::{End, drain, last_event, upstream};
	use crate::{
		LLMInfo, LLMResponse, LogContentFields, StreamingUsageGuard, StreamingUsageReporter,
	};

	type Chain = fn(Body, StreamingUsageGuard) -> Body;
	type Reports = Arc<Mutex<Vec<LLMResponse>>>;

	fn completions(b: Body, log: StreamingUsageGuard) -> Body {
		to_completions::translate_stream(
			b,
			1 << 20,
			strng::new("gemini-3.8-flash"),
			log,
			LogContentFields::default(),
		)
	}

	fn messages(b: Body, log: StreamingUsageGuard) -> Body {
		from_messages::translate_stream(
			b,
			1 << 20,
			strng::new("gemini-3.8-flash"),
			log,
			LogContentFields::default(),
		)
	}

	fn responses(b: Body, log: StreamingUsageGuard) -> Body {
		from_responses::translate_stream(
			b,
			1 << 20,
			strng::new("gemini-3.8-flash"),
			log,
			LogContentFields::default(),
			None,
		)
	}

	/// Settles once, on the first report or on drop, like the gateway's reporter
	struct Recorder {
		info: Arc<Mutex<LLMInfo>>,
		reports: Reports,
		reported: bool,
	}

	impl StreamingUsageReporter for Recorder {
		fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
			f(&mut self.info.lock().unwrap())
		}
		fn report_usage(&mut self) {
			if !std::mem::replace(&mut self.reported, true) {
				let response = self.info.lock().unwrap().response.clone();
				self.reports.lock().unwrap().push(response);
			}
		}
	}

	impl Drop for Recorder {
		fn drop(&mut self) {
			self.report_usage();
		}
	}

	fn recorder() -> (StreamingUsageGuard, Arc<Mutex<LLMInfo>>, Reports) {
		let info = captured_info();
		let reports = Arc::new(Mutex::new(Vec::new()));
		let recorder = Recorder {
			info: info.clone(),
			reports: reports.clone(),
			reported: false,
		};
		(StreamingUsageGuard::new(Box::new(recorder)), info, reports)
	}

	/// One upstream frame per SSE event, so the chain sees the chunks arrive separately
	fn sse_frames(input: &'static str) -> Body {
		let frames = input
			.split_inclusive("\n\n")
			.map(|event| Ok::<_, std::convert::Infallible>(bytes::Bytes::from(event)));
		Body::from_stream(futures_util::stream::iter(frames.collect::<Vec<_>>()))
	}

	fn client_events(body: &[u8]) -> Vec<Value> {
		std::str::from_utf8(body)
			.unwrap()
			.lines()
			.filter_map(|l| l.strip_prefix("data: "))
			.filter(|d| *d != "[DONE]")
			.map(|d| serde_json::from_str(d).unwrap())
			.collect()
	}

	fn gaps(r: &LLMResponse) -> u64 {
		r.inter_chunk_latencies.iter().map(|(count, _)| count).sum()
	}

	#[rstest::rstest]
	#[tokio::test]
	async fn reports_native_usage_once(#[values(messages, responses)] chain: Chain) {
		let (log, info, reports) = recorder();
		let input = include_str!("../tests/response/vertex-gemini/stream_tool.json");
		let mut body = chain(sse_frames(input), log);
		body.frame().await.expect("first frame").unwrap();
		let first_token = info.lock().unwrap().response.first_token;
		assert!(first_token.is_some());
		body.collect().await.unwrap();

		let reports = reports.lock().unwrap();
		let [r] = reports.as_slice() else {
			panic!("want one report, got {}", reports.len())
		};
		assert_eq!(
			(
				r.input_tokens,
				r.output_tokens,
				r.total_tokens,
				r.reasoning_tokens
			),
			(Some(39), Some(132), Some(171), Some(100))
		);
		assert_eq!(r.service_tier.as_deref(), Some("ON_DEMAND"));
		assert_eq!(r.provider_model.as_deref(), Some("gemini-3.8-flash"));
		assert_eq!(
			r.first_token, first_token,
			"later chunks keep the first-token time"
		);
		assert_eq!(gaps(r), 3, "one gap per native chunk after the first");
	}

	#[rstest::rstest]
	#[case::messages(
		messages,
		"message_delta",
		"/usage",
		json!({ "input_tokens": 2, "output_tokens": 3, "cache_read_input_tokens": 3 })
	)]
	#[case::responses(
		responses,
		"response.completed",
		"/response/usage",
		json!({
			"input_tokens": 5, "input_tokens_details": { "cached_tokens": 3 },
			"output_tokens": 3, "output_tokens_details": { "reasoning_tokens": 1 },
			"total_tokens": 8
		})
	)]
	#[tokio::test]
	async fn records_a_usage_only_final_chunk(
		#[case] chain: Chain,
		#[case] terminal: &str,
		#[case] usage: &str,
		#[case] expected: Value,
	) {
		let (log, _, reports) = recorder();
		let input = concat!(
			"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"hi\"}]},\"finishReason\":\"STOP\"}]}\n\n",
			"data: {\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":2,\"totalTokenCount\":8,",
			"\"thoughtsTokenCount\":1,\"cachedContentTokenCount\":3}}\n\n",
		);
		let out = chain(Body::from(input), log)
			.collect()
			.await
			.unwrap()
			.to_bytes();

		let events = client_events(&out);
		let terminals: Vec<_> = events.iter().filter(|e| e["type"] == terminal).collect();
		let [end] = terminals.as_slice() else {
			panic!("want one {terminal}, got {}", terminals.len())
		};
		assert_eq!(end.pointer(usage), Some(&expected));

		let reports = reports.lock().unwrap();
		let [r] = reports.as_slice() else {
			panic!("want one report, got {}", reports.len())
		};
		assert_eq!(
			(
				r.input_tokens,
				r.output_tokens,
				r.cached_input_tokens,
				r.reasoning_tokens,
				r.total_tokens
			),
			(Some(5), Some(3), Some(3), Some(1), Some(8))
		);
	}

	#[rstest::rstest]
	#[tokio::test]
	async fn reports_usage_seen_before_a_disconnect(#[values(messages, responses)] chain: Chain) {
		let (log, _, reports) = recorder();
		let first = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hi\"}]}}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8}}\n\n";
		let upstream = futures_util::StreamExt::chain(
			futures_util::stream::iter([Ok::<_, std::convert::Infallible>(bytes::Bytes::from(first))]),
			futures_util::stream::pending(),
		);
		let mut body = chain(Body::from_stream(upstream), log);
		tokio::time::timeout(Duration::from_secs(5), body.frame())
			.await
			.expect("first frame in time")
			.expect("stream still open")
			.unwrap();
		assert!(reports.lock().unwrap().is_empty());
		drop(body);

		let reports = reports.lock().unwrap();
		let [r] = reports.as_slice() else {
			panic!("want one report, got {}", reports.len())
		};
		assert_eq!(
			(r.input_tokens, r.output_tokens, r.total_tokens),
			(Some(7), Some(1), Some(8))
		);
		assert!(r.first_token.is_some());
	}

	/// Codex replays the final event's output, so the carrier has to survive there on a cut-off
	/// reply, and come back to Gemini on the signed call only
	#[tokio::test]
	async fn responses_replay_of_a_cut_off_output_restores_the_signature() {
		let request: Value = serde_json::from_str(include_str!(
			"../tests/requests/responses/namespace-tools.json"
		))
		.unwrap();
		let translated = from_responses::translate(
			&serde_json::from_value(request.clone()).unwrap(),
			true,
			None,
		)
		.unwrap();
		let native_request: Value = serde_json::from_slice(&translated.body).unwrap();
		let aliases: Vec<String> = native_request["tools"][0]["functionDeclarations"]
			.as_array()
			.unwrap()
			.iter()
			.take(2)
			.map(|d| d["name"].as_str().unwrap().to_string())
			.collect();

		let signature = "AY89".repeat(256);
		let chunk = json!({
			"candidates": [{ "content": { "role": "model", "parts": [
				{ "functionCall": { "id": "call_a", "name": aliases[0], "args": { "code": "1 + 1" } },
					"thoughtSignature": signature },
				{ "functionCall": { "id": "call_b", "name": aliases[1], "args": { "code": "2 + 2" } } }
			]}, "finishReason": "MAX_TOKENS" }],
			"usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 2, "totalTokenCount": 7 },
			"modelVersion": "gemini-3.8-flash"
		});
		let body = from_responses::translate_stream(
			Body::from(format!("data: {chunk}\n\n")),
			1 << 20,
			strng::new("gemini-3.8-flash"),
			StreamingUsageGuard::default(),
			LogContentFields::default(),
			Some(Arc::new(translated.namespaces)),
		)
		.collect()
		.await
		.unwrap()
		.to_bytes();
		assert!(!std::str::from_utf8(&body).unwrap().contains("__thought__"));

		let events = client_events(&body);
		let last = events.last().unwrap();
		assert_eq!(last["type"], "response.incomplete");
		let output = last["response"]["output"].as_array().unwrap();
		let items: Vec<_> = output
			.iter()
			.map(|i| (i["type"].as_str().unwrap(), i["status"].as_str().unwrap()))
			.collect();
		assert_eq!(
			items,
			[
				("message", "incomplete"),
				("reasoning", "completed"),
				("function_call", "incomplete"),
				("function_call", "incomplete")
			]
		);
		let added = events
			.iter()
			.find(|e| e["type"] == "response.output_item.added" && e["item"]["type"] == "reasoning")
			.unwrap();
		assert_eq!(added["output_index"], 1);
		assert_eq!(added["item"]["id"], output[1]["id"]);

		let mut replay = request;
		let mut input = vec![json!({ "role": "user", "content": "Run JavaScript in both kernels." })];
		input.extend(output.iter().cloned());
		input[2]["content"] = Value::Null;
		input.extend(output.iter().filter(|i| i["type"] == "function_call").map(
			|call| json!({ "type": "function_call_output", "call_id": call["call_id"], "output": "ok" }),
		));
		replay["input"] = Value::Array(input);
		let replayed =
			from_responses::translate(&serde_json::from_value(replay).unwrap(), true, None).unwrap();
		let native_replay: Value = serde_json::from_slice(&replayed.body).unwrap();
		let calls: Vec<_> = native_replay["contents"]
			.as_array()
			.unwrap()
			.iter()
			.flat_map(|c| c["parts"].as_array().unwrap())
			.filter(|p| p.get("functionCall").is_some())
			.map(|p| (&p["functionCall"]["name"], p.get("thoughtSignature")))
			.collect();
		assert_eq!(
			calls,
			[
				(&json!(aliases[0]), Some(&json!(signature))),
				(&json!(aliases[1]), None)
			]
		);
		assert!(
			!String::from_utf8(replayed.body)
				.unwrap()
				.contains("__thought__")
		);
	}

	const FIRST: &str = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hi\"}]}}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":1,\"totalTokenCount\":8}}\n\n";
	const ERROR: &str = "data: {\"error\":{\"code\":503,\"message\":\"The model is overloaded.\",\"status\":\"UNAVAILABLE\"}}\n\n";
	const LAST: &str = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"!\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n";

	const SUCCESS_ENDINGS: [&str; 4] = [
		"[DONE]",
		"message_stop",
		"response.completed",
		"\"finish_reason\":\"stop\"",
	];

	/// Gemini's error ends the body after its error event, however upstream then ends. A stream cut
	/// off without one ends with a body error. Usage seen before the failure is reported once
	#[rstest::rstest]
	#[case::error_event(&[FIRST, ERROR, LAST], End::Close)]
	#[case::error_on_an_open_stream(&[FIRST, ERROR], End::Hang)]
	#[case::error_then_connection_drop(&[FIRST, ERROR], End::Reset)]
	#[case::no_finish_reason(&[FIRST], End::Close)]
	#[case::connection_drop(&[FIRST], End::Reset)]
	#[tokio::test]
	async fn a_failed_stream_never_ends_as_a_success(
		#[values(completions, messages, responses)] chain: Chain,
		#[case] frames: &[&'static str],
		#[case] end: End,
	) {
		let (log, _, reports) = recorder();
		let (text, failed) = tokio::time::timeout(
			Duration::from_secs(5),
			drain(chain(upstream(frames, end), log)),
		)
		.await
		.expect("the stream should end once Gemini reports an error");
		let reported = frames.contains(&ERROR);
		assert_eq!(failed, !reported, "body error:\n{text}");
		for ending in SUCCESS_ENDINGS {
			assert!(!text.contains(ending), "{ending} in:\n{text}");
		}
		assert!(
			!text.contains('!'),
			"a reply after the error reached the client:\n{text}"
		);
		if reported {
			assert!(
				last_event(&text).contains("The model is overloaded."),
				"Gemini's error should be the last event:\n{text}"
			);
		}

		let reports = reports.lock().unwrap();
		let [r] = reports.as_slice() else {
			panic!("want one report, got {}", reports.len())
		};
		assert_eq!(
			(r.input_tokens, r.output_tokens, r.total_tokens),
			(Some(7), Some(1), Some(8))
		);
	}

	#[tokio::test]
	async fn completions_clients_get_geminis_error() {
		let (text, _) = drain(completions(
			upstream(&[FIRST, ERROR], End::Close),
			StreamingUsageGuard::default(),
		))
		.await;
		assert_eq!(
			last_event(&text),
			r#"data: {"event_id":null,"error":{"type":"api_error","message":"The model is overloaded.","code":"UNAVAILABLE"}}"#
		);
	}

	const MALFORMED: &str = "data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"!\"}]},\"finishReason\":\"MALFORMED_FUNCTION_CALL\",\"finishMessage\":\"Bad call\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2,\"totalTokenCount\":9}}\n\n";

	/// The failed reply's own text is dropped, and its usage still counts
	#[rstest::rstest]
	#[tokio::test]
	async fn a_failed_reply_ends_with_an_error(
		#[values(completions, messages, responses)] chain: Chain,
	) {
		let (log, _, reports) = recorder();
		let (text, failed) = drain(chain(upstream(&[FIRST, MALFORMED], End::Close), log)).await;
		assert!(!failed, "{text}");
		for ending in SUCCESS_ENDINGS {
			assert!(!text.contains(ending), "{ending} in:\n{text}");
		}
		assert!(!text.contains('!'), "{text}");
		assert!(
			last_event(&text).contains("Gemini stopped with MALFORMED_FUNCTION_CALL: Bad call"),
			"{text}"
		);
		let reports = reports.lock().unwrap();
		let [r] = reports.as_slice() else {
			panic!("want one report, got {}", reports.len())
		};
		assert_eq!(r.total_tokens, Some(9));
	}

	#[tokio::test]
	async fn completions_clients_get_the_failed_reason() {
		let (text, _) = drain(completions(
			upstream(&[FIRST, MALFORMED], End::Close),
			StreamingUsageGuard::default(),
		))
		.await;
		assert_eq!(
			last_event(&text),
			r#"data: {"event_id":null,"error":{"type":"api_error","message":"Gemini stopped with MALFORMED_FUNCTION_CALL: Bad call","code":"INTERNAL"}}"#
		);
	}

	#[rstest::rstest]
	#[case::unavailable(ERROR, "overloaded_error", "The model is overloaded.")]
	#[case::permission_denied(
		"data: {\"error\":{\"code\":403,\"message\":\"Permission denied.\",\"status\":\"PERMISSION_DENIED\"}}\n\n",
		"permission_error",
		"Permission denied."
	)]
	#[case::unauthenticated(
		"data: {\"error\":{\"code\":401,\"message\":\"API key not valid.\",\"status\":\"UNAUTHENTICATED\"}}\n\n",
		"authentication_error",
		"API key not valid."
	)]
	#[tokio::test]
	async fn messages_clients_get_geminis_error(
		#[case] error: &'static str,
		#[case] r#type: &str,
		#[case] message: &str,
	) {
		let (text, _) = drain(messages(
			upstream(&[FIRST, error], End::Close),
			StreamingUsageGuard::default(),
		))
		.await;
		assert_eq!(
			last_event(&text),
			format!(
				"event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{type}\",\"message\":\"{message}\"}}}}"
			)
		);
	}
}
