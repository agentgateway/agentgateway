use serde_json::{Value, json};

use super::*;

fn req(v: Value) -> types::completions::Request {
	serde_json::from_value(v).expect("valid completions request")
}

fn to_gemini(v: Value) -> Value {
	let bytes = from_completions::translate(&req(v), None).expect("translate ok");
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
		.to_llm_response(false)
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

/// Gemini's responseSchema subset rejects $defs/$ref/additionalProperties,
/// so the translator must inline the $ref, drop $defs, and strip additionalProperties before egress.
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
	assert!(
		!s.contains("additionalProperties"),
		"Gemini rejects additionalProperties: {s}"
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
		!s.contains("additionalProperties"),
		"Gemini rejects additionalProperties: {s}"
	);
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

// Case 1: additionalProperties must be dropped at every level, not just the top.
#[test]
fn gemini_schema_drops_nested_additional_properties() {
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
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("additionalProperties"),
		"additionalProperties must be stripped everywhere: {txt}"
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
	assert!(
		!txt.contains("additionalProperties"),
		"tool parameters must drop additionalProperties: {txt}"
	);
	assert_eq!(params["properties"]["events"]["items"]["type"], "object");
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

// Case 8: Dict[str, X] emits a typed `additionalProperties` schema. Gemini does not support it, so
// it must be dropped (the open value typing is lost; that is the documented trade-off).
#[test]
fn gemini_schema_drops_open_dict_additional_properties() {
	let s = response_schema(json!({
		"type": "object",
		"additionalProperties": false,
		"properties": {
			"meta": { "type": "object", "additionalProperties": { "type": "string" } }
		}
	}));
	let txt = serde_json::to_string(&s).unwrap();
	assert!(
		!txt.contains("additionalProperties"),
		"typed additionalProperties (open dict) must be dropped: {txt}"
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
fn cel_usage_fields_match_usage_metadata() {
	// The CEL/log token fields (via to_llm_response) must equal Gemini's usageMetadata exactly,
	// so rate limiting and telemetry see native counts rather than shim numbers.
	let r = llm_resp(json!({
		"candidates": [{ "content": { "role": "model", "parts": [{ "text": "x" }] },
			"finishReason": "STOP" }],
		"usageMetadata": {
			"promptTokenCount": 100, "candidatesTokenCount": 50, "totalTokenCount": 150,
			"cachedContentTokenCount": 30, "thoughtsTokenCount": 20
		}
	}));
	assert_eq!(r.input_tokens, Some(100));
	assert_eq!(r.output_tokens, Some(50));
	assert_eq!(r.total_tokens, Some(150));
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
