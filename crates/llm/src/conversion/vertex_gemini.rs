use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;

use crate::conversion::completions::parse_data_url;
use crate::types::{ResponseType, vertex_gemini as vg};
use crate::{AIError, logged_response_parsing, types};

#[cfg(test)]
#[path = "vertex_gemini_tests.rs"]
mod tests;

/// Vertex rejects an `id` on functionCall/functionResponse parts and Gemini 3 hard-400s if a functionCall's signature is
/// not echoed back, but OpenAI clients drop unknown fields while reliably echoing `tool_call_id`.
/// So the signature rides inside the id as `<base-id>__thought__<signature>`, recovered before the
/// outbound Vertex request. Split on the first separator: the synthesized base id never contains it.
const THOUGHT_SIGNATURE_SEPARATOR: &str = "__thought__";

fn split_tool_call_id(raw: &str) -> (&str, Option<&str>) {
	match raw.split_once(THOUGHT_SIGNATURE_SEPARATOR) {
		Some((base, sig)) if !sig.is_empty() => (base, Some(sig)),
		_ => (raw, None),
	}
}

/// Embed an optional thoughtSignature into a tool_call id for the client to echo back. A call with
/// no signature keeps a plain id (no trailing separator), matching faithful passthrough.
fn join_tool_call_id(base: String, signature: Option<&str>) -> String {
	match signature {
		Some(sig) if !sig.is_empty() => format!("{base}{THOUGHT_SIGNATURE_SEPARATOR}{sig}"),
		_ => base,
	}
}

fn canonical_mime(mime: &str) -> &str {
	match mime {
		"image/jpg" => "image/jpeg",
		other => other,
	}
}

fn mime_from_ext_token(ext: &str) -> Option<&'static str> {
	Some(match ext.to_ascii_lowercase().as_str() {
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"webp" => "image/webp",
		"gif" => "image/gif",
		"heic" => "image/heic",
		"heif" => "image/heif",
		"pdf" => "application/pdf",
		"mp3" => "audio/mpeg",
		"wav" => "audio/wav",
		"mp4" => "video/mp4",
		"mov" => "video/quicktime",
		"webm" => "video/webm",
		"txt" => "text/plain",
		_ => return None,
	})
}

fn mime_from_extension(uri: &str) -> Option<&'static str> {
	let (_, ext) = uri.rsplit('/').next()?.rsplit_once('.')?;
	mime_from_ext_token(ext)
}

fn explicit_mime_hint(image_url: Option<&serde_json::Value>) -> Option<String> {
	let obj = image_url?;
	let hint = ["format", "mime_type", "content_type"]
		.into_iter()
		.find_map(|k| {
			obj
				.get(k)
				.and_then(|v| v.as_str())
				.filter(|h| !h.is_empty())
		})?;
	if hint.contains('/') {
		Some(hint.to_string())
	} else {
		mime_from_ext_token(hint).map(str::to_string)
	}
}

fn image_part(image_url: Option<&serde_json::Value>) -> Result<vg::Part, AIError> {
	let url = image_url
		.and_then(|u| u.get("url"))
		.and_then(|v| v.as_str())
		.unwrap_or_default();

	if let Some((mime, data)) = parse_data_url(url) {
		return Ok(vg::Part::InlineData(vg::InlineDataPart {
			inline_data: vg::Blob {
				mime_type: canonical_mime(mime).to_string(),
				data: data.to_string(),
				rest: serde_json::Value::Null,
			},
			rest: serde_json::Value::Null,
		}));
	}

	if url.starts_with("gs://") {
		let Some(mime) =
			explicit_mime_hint(image_url).or_else(|| mime_from_extension(url).map(str::to_string))
		else {
			return Err(AIError::InvalidResponse(strng::new(format!(
				"gs:// image_url ({url}) has no recognised extension or MIME hint; pass image_url.format (or mime_type/content_type), or use an object with a known extension"
			))));
		};
		return Ok(vg::Part::FileData(vg::FileDataPart {
			file_data: vg::FileData {
				mime_type: Some(canonical_mime(&mime).to_string()),
				file_uri: url.to_string(),
				rest: serde_json::Value::Null,
			},
			rest: serde_json::Value::Null,
		}));
	}

	Err(AIError::InvalidResponse(strng::new(format!(
		"native Gemini path rejects http(s) image_url ({url}); upload to gs:// or send inline data:"
	))))
}

fn text_part(text: &str) -> vg::Part {
	vg::Part::Text(vg::TextPart {
		text: text.to_string(),
		thought: None,
		thought_signature: None,
		rest: serde_json::Value::Null,
	})
}

fn is_text_part(p: &vg::Part) -> bool {
	matches!(p, vg::Part::Text(_))
}

fn push_content(contents: &mut Vec<vg::Content>, role: &str, mut parts: Vec<vg::Part>) {
	if parts.is_empty() {
		return;
	}
	if let Some(last) = contents.last_mut()
		&& last.role.as_deref() == Some(role)
	{
		if role == "user" && !last.parts.iter().any(is_text_part) && !parts.iter().any(is_text_part) {
			parts.push(text_part(" "));
		}
		last.parts.extend(parts);
		return;
	}
	if role == "user" && !parts.iter().any(is_text_part) {
		parts.push(text_part(" "));
	}
	contents.push(vg::Content {
		role: Some(role.to_string()),
		parts,
		rest: serde_json::Value::Null,
	});
}

/// Gemini 3.x takes a `thinkingLevel` string; Gemini 2.5 takes an integer
/// `thinkingBudget`. Detected by model name.
fn uses_thinking_levels(model: &str) -> bool {
	model.contains("gemini-3")
}

// Conservative `reasoning_effort` -> Gemini 2.5 `thinkingBudget` mapping, chosen to
// be valid for both Flash and Pro (Pro's documented range is 128..=32768).
const THINKING_BUDGET_LOW: i32 = 1024;
const THINKING_BUDGET_MEDIUM: i32 = 2048;
const THINKING_BUDGET_HIGH: i32 = 4096;

fn apply_rest_extras(
	rest: &serde_json::Value,
) -> (
	Option<String>,
	Vec<vg::SafetySetting>,
	Option<serde_json::Map<String, serde_json::Value>>,
) {
	use serde::Deserialize as _;

	let cached_content = rest
		.get("cachedContent")
		.or_else(|| rest.get("cached_content"))
		.and_then(serde_json::Value::as_str)
		.map(str::to_string);

	let safety_settings = match rest
		.get("safetySettings")
		.or_else(|| rest.get("safety_settings"))
	{
		Some(v) => Vec::<vg::SafetySetting>::deserialize(v).unwrap_or_else(|e| {
			tracing::warn!(error = %e, "ignoring malformed safetySettings");
			Vec::new()
		}),
		None => Vec::new(),
	};

	let labels = rest.get("labels").and_then(|v| v.as_object().cloned());

	(cached_content, safety_settings, labels)
}

fn drop_if_cached(
	cached_content: &Option<String>,
	system_instruction: Option<vg::Content>,
	tools: Vec<vg::Tool>,
	tool_config: Option<vg::ToolConfig>,
) -> (Option<vg::Content>, Vec<vg::Tool>, Option<vg::ToolConfig>) {
	if cached_content.is_none() {
		return (system_instruction, tools, tool_config);
	}
	let dropped: Vec<&str> = [
		("systemInstruction", system_instruction.is_some()),
		("tools", !tools.is_empty()),
		("toolConfig", tool_config.is_some()),
	]
	.into_iter()
	.filter_map(|(name, present)| present.then_some(name))
	.collect();
	if !dropped.is_empty() {
		tracing::warn!(dropped = ?dropped, "cachedContent is set; dropped cache-incompatible fields");
	}
	(None, Vec::new(), None)
}

fn reorder_function_responses(
	contents: &mut [vg::Content],
	call_meta: &std::collections::HashMap<String, (String, usize)>,
) {
	for content in contents.iter_mut() {
		let mut ordered: Vec<vg::Part> = content
			.parts
			.iter()
			.filter(|p| matches!(p, vg::Part::FunctionResponse(_)))
			.cloned()
			.collect();
		if ordered.is_empty() {
			continue;
		}
		ordered.sort_by_key(|p| match p {
			vg::Part::FunctionResponse(fr) => fr
				.function_response
				.id
				.as_deref()
				.and_then(|id| call_meta.get(id))
				.map(|(_, idx)| *idx)
				.unwrap_or(usize::MAX),
			_ => usize::MAX,
		});
		for p in &mut ordered {
			if let vg::Part::FunctionResponse(fr) = p {
				fr.function_response.id = None;
			}
		}
		let mut ordered = ordered.into_iter();
		for p in &mut content.parts {
			if matches!(p, vg::Part::FunctionResponse(_)) {
				*p = ordered
					.next()
					.expect("one reordered response per functionResponse slot");
			}
		}
	}
}

fn wrap_tool_declarations(decls: Vec<vg::FunctionDeclaration>) -> Vec<vg::Tool> {
	if decls.is_empty() {
		Vec::new()
	} else {
		vec![vg::Tool {
			function_declarations: decls,
		}]
	}
}

pub mod from_completions {
	use serde::Deserialize;
	use serde_json::{Value, json};

	use super::*;

	pub fn translate(
		req: &types::completions::Request,
		configured_model: Option<&str>,
	) -> Result<Vec<u8>, AIError> {
		let out = build_request(req, configured_model)?;
		serde_json::to_vec(&out).map_err(AIError::RequestMarshal)
	}

	fn build_request(
		req: &types::completions::Request,
		configured_model: Option<&str>,
	) -> Result<vg::GenerateContentRequest, AIError> {
		let model = configured_model
			.or(req.model.as_deref())
			.unwrap_or_default()
			.to_string();

		let (system_text, contents) = messages_to_contents(&req.messages)?;

		// Invariant: empty contents to Vertex returns "contents is required".
		let contents = if contents.is_empty() {
			vec![vg::Content {
				role: Some("user".to_string()),
				parts: vec![text_part(" ")],
				rest: Value::Null,
			}]
		} else {
			contents
		};

		let system_instruction = (!system_text.is_empty()).then(|| vg::Content {
			role: None,
			parts: vec![text_part(&system_text.join("\n"))],
			rest: Value::Null,
		});

		let tools = build_tools(req);
		let tool_config = build_tool_config(req);
		let generation_config = build_generation_config(req, &model);

		let (cached_content, safety_settings, labels) = super::apply_rest_extras(&req.rest);
		let (system_instruction, tools, tool_config) =
			super::drop_if_cached(&cached_content, system_instruction, tools, tool_config);

		Ok(vg::GenerateContentRequest {
			contents,
			system_instruction,
			tools,
			tool_config,
			generation_config,
			safety_settings,
			cached_content,
			labels,
		})
	}

	fn messages_to_contents(
		messages: &[types::completions::RequestMessage],
	) -> Result<(Vec<String>, Vec<vg::Content>), AIError> {
		use types::completions::Content;

		// base tool_call id -> (function name, position in the assistant's tool_calls). The index
		// lets the post-loop pass restore call order on the functionResponse parts.
		let mut call_meta: std::collections::HashMap<String, (String, usize)> = Default::default();
		let mut system_text: Vec<String> = Vec::new();
		let mut contents: Vec<vg::Content> = Vec::new();
		for m in messages {
			match m.role.as_str() {
				"system" | "developer" => {
					system_text.extend(content_text(&m.content).filter(|t| !t.is_empty()));
				},
				"user" => push_content(&mut contents, "user", user_parts(&m.content)?),
				"assistant" => {
					if let Some(calls) = &m.tool_calls {
						for (idx, c) in calls.iter().enumerate() {
							if let (Some(id), Some(name)) = (
								c.get("id").and_then(Value::as_str),
								c.get("function")
									.and_then(|f| f.get("name"))
									.and_then(Value::as_str),
							) {
								call_meta.insert(
									split_tool_call_id(id).0.to_string(),
									(name.to_string(), idx),
								);
							}
						}
					}
					let mut parts: Vec<_> = match &m.content {
						Some(Content::Text(t)) if !t.is_empty() => vec![text_part(t)],
						Some(Content::Array(arr)) => arr
							.iter()
							.filter(|p| p.r#type == "text")
							.filter_map(|p| p.text.as_deref().map(text_part))
							.collect(),
						_ => vec![],
					};
					parts.extend(m.tool_calls.iter().flatten().map(function_call_part));
					push_content(&mut contents, "model", parts);
				},
				"tool" | "function" => {
					let base_id = m
						.tool_call_id
						.as_deref()
						.map(|id| split_tool_call_id(id).0.to_string());
					let name = base_id
						.as_deref()
						.and_then(|id| call_meta.get(id))
						.map(|(name, _)| name.clone())
						.or_else(|| m.name.clone())
						.unwrap_or_default();

					let response = content_text(&m.content)
						.map(|t| json!({ "content": t }))
						.unwrap_or_else(|| json!({}));
					// Carry the base id as a transient correlation key, the post-loop pass orders the
					// responses to match the call order, then strips it.
					let part = vg::Part::FunctionResponse(vg::FunctionResponsePart {
						function_response: vg::FunctionResponse {
							name,
							id: base_id,
							response,
							rest: Value::Null,
						},
						rest: Value::Null,
					});
					push_content(&mut contents, "user", vec![part]);
				},
				_ => {},
			}
		}

		// Vertex rejects `id` and correlates functionResponse to functionCall positionally, so each
		// response group must follow the assistant's tool_calls order even when a client returns the
		// `tool` messages out of order.
		super::reorder_function_responses(&mut contents, &call_meta);
		Ok((system_text, contents))
	}

	fn content_text(content: &Option<types::completions::Content>) -> Option<String> {
		use types::completions::Content;
		match content {
			Some(Content::Text(t)) => Some(t.clone()),
			Some(Content::Array(parts)) => Some(
				parts
					.iter()
					.filter(|p| p.r#type == "text")
					.filter_map(|p| p.text.as_deref())
					.collect::<String>(),
			),
			None => None,
		}
	}

	fn user_parts(content: &Option<types::completions::Content>) -> Result<Vec<vg::Part>, AIError> {
		use types::completions::Content;
		let mut parts = Vec::new();
		match content {
			// Preserve an explicit empty string as {text: ""} (distinct from the synthetic
			// " " filler, which only fires when a user turn has no text part at all).
			Some(Content::Text(t)) => parts.push(text_part(t)),
			Some(Content::Array(arr)) => {
				for p in arr {
					match p.r#type.as_str() {
						"text" => {
							if let Some(t) = &p.text {
								parts.push(text_part(t));
							}
						},
						"image_url" => {
							parts.push(image_part(p.rest.get("image_url"))?);
						},
						_ => {},
					}
				}
			},
			_ => {},
		}
		Ok(parts)
	}

	fn function_call_part(call: &Value) -> vg::Part {
		let func = call.get("function");
		let name = func
			.and_then(|f| f.get("name"))
			.and_then(Value::as_str)
			.unwrap_or_default()
			.to_string();
		let args = func
			.and_then(|f| f.get("arguments"))
			.and_then(Value::as_str)
			.and_then(|s| serde_json::from_str::<Value>(s).ok())
			.unwrap_or_else(|| json!({}));
		// Vertex rejects `id` on functionCall parts; recover any embedded thoughtSignature and drop the id
		let thought_signature = call
			.get("id")
			.and_then(Value::as_str)
			.and_then(|raw| split_tool_call_id(raw).1)
			.map(str::to_string);
		vg::Part::FunctionCall(vg::FunctionCallPart {
			function_call: vg::FunctionCall {
				name,
				id: None,
				args,
				rest: Value::Null,
			},
			thought: None,
			thought_signature,
			rest: Value::Null,
		})
	}

	fn build_tools(req: &types::completions::Request) -> Vec<vg::Tool> {
		let Some(tools) = &req.tools else {
			return Vec::new();
		};
		let decls: Vec<vg::FunctionDeclaration> = tools
			.iter()
			.filter_map(|t| t.get("function"))
			.map(|f| vg::FunctionDeclaration {
				name: f
					.get("name")
					.and_then(Value::as_str)
					.unwrap_or_default()
					.to_string(),
				description: f
					.get("description")
					.and_then(Value::as_str)
					.map(str::to_string),
				parameters: f.get("parameters").map(normalize_gemini_schema),
			})
			.collect();
		super::wrap_tool_declarations(decls)
	}

	fn build_tool_config(req: &types::completions::Request) -> Option<vg::ToolConfig> {
		let tc = req.tool_choice.as_ref()?;
		let cfg = match tc {
			Value::String(s) => match s.as_str() {
				"none" => vg::FunctionCallingConfig {
					mode: Some("NONE".into()),
					..Default::default()
				},
				"required" => vg::FunctionCallingConfig {
					mode: Some("ANY".into()),
					..Default::default()
				},
				_ => vg::FunctionCallingConfig {
					mode: Some("AUTO".into()),
					..Default::default()
				},
			},
			Value::Object(_) => {
				let name = tc
					.get("function")
					.and_then(|f| f.get("name"))
					.and_then(Value::as_str);
				vg::FunctionCallingConfig {
					mode: Some("ANY".into()),
					allowed_function_names: name.map(|n| vec![n.to_string()]).unwrap_or_default(),
				}
			},
			_ => return None,
		};
		Some(vg::ToolConfig {
			function_calling_config: Some(cfg),
		})
	}

	fn build_generation_config(
		req: &types::completions::Request,
		model: &str,
	) -> Option<vg::GenerationConfig> {
		let stop_sequences = match &req.stop {
			Some(Value::String(s)) => vec![s.clone()],
			Some(Value::Array(a)) => a
				.iter()
				.filter_map(Value::as_str)
				.map(str::to_string)
				.collect(),
			_ => Vec::new(),
		};

		let (response_mime_type, response_schema) = response_format(req);
		let thinking_config = thinking_config(req, model);

		let cfg = vg::GenerationConfig {
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: req
				.rest
				.get("top_k")
				.and_then(Value::as_u64)
				.map(|v| v as u32),
			frequency_penalty: req.frequency_penalty,
			presence_penalty: req.presence_penalty,
			max_output_tokens: req.max_completion_tokens.or(req.max_tokens),
			stop_sequences,
			candidate_count: req.rest.get("n").and_then(Value::as_u64).map(|v| v as u32),
			seed: req.seed,
			response_mime_type,
			response_schema,
			thinking_config,
		};

		if cfg == vg::GenerationConfig::default() {
			None
		} else {
			Some(cfg)
		}
	}

	fn response_format(req: &types::completions::Request) -> (Option<String>, Option<Value>) {
		let Some(rf) = req.rest.get("response_format") else {
			return (None, None);
		};
		match rf.get("type").and_then(Value::as_str) {
			Some("json_object") => (Some("application/json".into()), None),
			Some("json_schema") => {
				// Unwrap OpenAI's {schema, strict, name, description} and normalize the bare schema.
				let schema = rf
					.get("json_schema")
					.and_then(|js| js.get("schema"))
					.map(normalize_gemini_schema);
				(Some("application/json".into()), schema)
			},
			_ => (None, None),
		}
	}

	// Gemini's responseSchema / functionDeclarations[].parameters accept only a subset of JSON Schema.
	// The normalization below is ported from litellm's `_build_vertex_schema` (BerriAI/litellm, MIT).

	/// Schema fields Gemini accepts. `format` is further pruned to enum/date-time and `enum` is
	/// dropped on non-string types.
	const ALLOWED_SCHEMA_FIELDS: &[&str] = &[
		"type",
		"format",
		"description",
		"title",
		"nullable",
		"enum",
		"items",
		"properties",
		"required",
		"anyOf",
		"default",
		"minLength",
		"maxLength",
		"pattern",
		"minimum",
		"maximum",
		"exclusiveMinimum",
		"exclusiveMaximum",
		"propertyOrdering",
	];

	/// Normalize an OpenAI/Pydantic JSON Schema into Gemini's responseSchema subset.
	pub(super) fn normalize_gemini_schema(schema: &Value) -> Value {
		let mut out = schema.clone();
		let defs = take_defs(&mut out);
		inline_refs(&mut out, &defs, &mut Vec::new());
		clean_schema_node(&mut out);
		out
	}

	/// Move the top-level `$defs`/`definitions` out of `root` into a lookup map.
	fn take_defs(root: &mut Value) -> serde_json::Map<String, Value> {
		let mut defs = serde_json::Map::new();
		if let Value::Object(map) = root {
			for key in ["$defs", "definitions"] {
				if let Some(Value::Object(obj)) = map.remove(key) {
					defs.extend(obj);
				}
			}
		}
		defs
	}

	/// Visit each direct child schema (`items`, `properties`, `anyOf`, `allOf`), shared by both passes
	/// so they recurse the same keywords. (`clean_schema_node` flattens `allOf` first, so it is a no-op here.)
	fn for_each_child_schema(
		map: &mut serde_json::Map<String, Value>,
		mut f: impl FnMut(&mut Value),
	) {
		if let Some(items) = map.get_mut("items") {
			f(items);
		}
		if let Some(Value::Object(props)) = map.get_mut("properties") {
			for v in props.values_mut() {
				f(v);
			}
		}
		for key in ["anyOf", "allOf"] {
			if let Some(Value::Array(arr)) = map.get_mut(key) {
				for v in arr.iter_mut() {
					f(v);
				}
			}
		}
	}

	/// Inline `$ref` against `defs` (sibling keys win over the target) and drop `$defs`/`definitions`.
	/// A `$ref` already on the resolution chain is left in place: Gemini cannot represent recursion,
	/// but we must not loop.
	fn inline_refs(node: &mut Value, defs: &serde_json::Map<String, Value>, chain: &mut Vec<String>) {
		let ref_name = node
			.get("$ref")
			.and_then(Value::as_str)
			.map(|r| r.rsplit('/').next().unwrap_or(r).to_string());
		if let Some(name) = ref_name {
			if chain.contains(&name) {
				return;
			}
			if let Some(target) = defs.get(&name) {
				let mut resolved = target.clone();
				if let (Value::Object(rmap), Value::Object(orig)) = (&mut resolved, &*node) {
					for (k, v) in orig.iter() {
						if k != "$ref" {
							rmap.insert(k.clone(), v.clone());
						}
					}
				}
				chain.push(name);
				inline_refs(&mut resolved, defs, chain);
				chain.pop();
				*node = resolved;
			}
			// Unknown ref: leave the node untouched.
			return;
		}

		match node {
			Value::Object(map) => {
				// Nested $defs (top-level ones already taken).
				map.remove("$defs");
				map.remove("definitions");
				for_each_child_schema(map, |v| inline_refs(v, defs, chain));
			},
			Value::Array(arr) => {
				for v in arr.iter_mut() {
					inline_refs(v, defs, chain);
				}
			},
			_ => {},
		}
	}

	/// One pass of the structural rewrites that can each surface further structural keywords on the
	/// node: flatten `allOf`, collapse a nullable/single-member `anyOf`, lift `const` to an `enum`
	/// typed by its value kind, and split a `type` array. Returns whether it changed the node, so the
	/// caller can re-run it until stable (an anyOf member may carry an allOf, an allOf member a const).
	fn rewrite_structural(map: &mut serde_json::Map<String, Value>) -> bool {
		let mut changed = false;

		// Flatten allOf into the parent. `properties` is unioned by name (first definition wins on a
		// duplicate key, no recursive merge of the colliding sub-schemas) and `required` is unioned,
		// so a multi-member allOf with disjoint fields keeps them all. Every other key is first-wins:
		// a member carrying its own anyOf/oneOf/allOf can't be merged into a single node, so only the
		// first is kept (best-effort, since Gemini has no allOf). The value type is checked before
		// touching the parent so a malformed member (e.g. `properties: null`) can't inject an empty
		// container.
		if let Some(Value::Array(members)) = map.remove("allOf") {
			changed = true;
			for m in members {
				let Value::Object(mm) = m else { continue };
				for (k, v) in mm {
					if k == "properties"
						&& let Value::Object(src) = v
					{
						if let Value::Object(dst) = map.entry("properties").or_insert_with(|| json!({})) {
							for (pk, pv) in src {
								dst.entry(pk).or_insert(pv);
							}
						}
					} else if k == "required"
						&& let Value::Array(src) = v
					{
						if let Value::Array(dst) = map.entry("required").or_insert_with(|| json!([])) {
							for item in src {
								if !dst.contains(&item) {
									dst.push(item);
								}
							}
						}
					} else {
						map.entry(k).or_insert(v);
					}
				}
			}
		}

		// const -> single-value enum typed by the const's JSON kind (Gemini has no const). A
		// non-string enum is dropped later by the enum-on-non-string rule, but the inferred type stays.
		if let Some(c) = map.remove("const") {
			changed = true;
			let ty = const_value_type(&c);
			map.insert("enum".to_string(), Value::Array(vec![c]));
			map.entry("type".to_string()).or_insert_with(|| ty.into());
		}

		if let Some(Value::Array(types)) = map.get("type").cloned() {
			changed = true;
			let names: Vec<String> = types
				.iter()
				.filter_map(|t| t.as_str().map(String::from))
				.collect();
			let has_null = names.iter().any(|t| t == "null");
			let non_null: Vec<String> = names.into_iter().filter(|t| t != "null").collect();
			map.remove("type");
			if has_null {
				map.insert("nullable".to_string(), true.into());
			}
			match non_null.as_slice() {
				[] => {},
				[one] => {
					map.insert("type".to_string(), one.clone().into());
				},
				many => {
					let any_of = many.iter().map(|t| json!({ "type": t })).collect();
					map.insert("anyOf".to_string(), Value::Array(any_of));
				},
			}
		}

		// anyOf with a {type:null} branch -> nullable; collapse a single remaining member up. Only act
		// when collapsible (a null branch or a single member) so genuine multi-member unions are left
		// alone and the fixpoint terminates.
		let collapsible = map
			.get("anyOf")
			.and_then(Value::as_array)
			.map(|members| {
				members.len() == 1
					|| members
						.iter()
						.any(|m| m.get("type").and_then(Value::as_str) == Some("null"))
			})
			.unwrap_or(false);
		if collapsible && let Some(Value::Array(members)) = map.remove("anyOf") {
			changed = true;
			let had_null = members
				.iter()
				.any(|m| m.get("type").and_then(Value::as_str) == Some("null"));
			let non_null: Vec<Value> = members
				.into_iter()
				.filter(|m| m.get("type").and_then(Value::as_str) != Some("null"))
				.collect();
			if had_null {
				map.insert("nullable".to_string(), true.into());
			}
			match non_null.len() {
				0 => {},
				1 => {
					// Merge the lone member up; existing parent keys win (consistent with allOf).
					if let Some(Value::Object(member)) = non_null.into_iter().next() {
						for (k, v) in member {
							map.entry(k).or_insert(v);
						}
					}
				},
				_ => {
					map.insert("anyOf".to_string(), Value::Array(non_null));
				},
			}
		}

		changed
	}

	/// Gemini Schema `type` matching the JSON kind of a `const` value.
	fn const_value_type(v: &Value) -> &'static str {
		match v {
			Value::String(_) => "string",
			Value::Bool(_) => "boolean",
			Value::Number(n) if n.is_f64() => "number",
			Value::Number(_) => "integer",
			Value::Array(_) => "array",
			_ => "object",
		}
	}

	/// Rewrite a single schema node and its children into Gemini's accepted shape.
	fn clean_schema_node(node: &mut Value) {
		let map = match node {
			Value::Object(map) => map,
			Value::Array(arr) => {
				for v in arr.iter_mut() {
					clean_schema_node(v);
				}
				return;
			},
			_ => return,
		};

		// A structural rewrite can surface further structural keywords on a merged node, so run them to
		// a fixpoint before the scalar cleanups. Depth is bounded by serde_json's parse limit; the guard
		// is only a safety net.
		let mut guard = 0;
		while rewrite_structural(map) && guard < 64 {
			guard += 1;
		}

		// Gemini requires items on arrays.
		if map.get("type").and_then(Value::as_str) == Some("array") && !map.contains_key("items") {
			map.insert("items".to_string(), json!({ "type": "object" }));
		}

		// enum applies to string types only: drop it on an explicitly non-string type, but default a
		// typeless enum to string rather than dropping the constraint and mistyping the node.
		if map.contains_key("enum") {
			match map.get("type").and_then(Value::as_str) {
				Some("string") => {},
				Some(_) => {
					map.remove("enum");
				},
				None => {
					map.insert("type".to_string(), "string".into());
				},
			}
		}

		// additionalProperties is unsupported (boolean form and open-dict form alike).
		map.remove("additionalProperties");

		// Default any remaining typeless, non-union, non-enum node to an object.
		if !map.contains_key("type") && !map.contains_key("anyOf") && !map.contains_key("enum") {
			map.insert("type".to_string(), "object".into());
		}

		let drop_format = map
			.get("format")
			.and_then(Value::as_str)
			.map(|f| f != "enum" && f != "date-time")
			.unwrap_or(false);
		if drop_format {
			map.remove("format");
		}

		for_each_child_schema(map, clean_schema_node);
		map.retain(|k, _| ALLOWED_SCHEMA_FIELDS.contains(&k.as_str()));
	}

	fn thinking_config(req: &types::completions::Request, model: &str) -> Option<vg::ThinkingConfig> {
		if let Some(tc) = req
			.rest
			.get("thinking_config")
			.or_else(|| req.rest.get("thinkingConfig"))
		{
			return vg::ThinkingConfig::deserialize(tc).ok();
		}

		let effort = req.rest.get("reasoning_effort").and_then(Value::as_str)?;
		if effort == "none" {
			// Omit thinkingConfig; on Gemini 2.5 Pro emitting budget 0 is rejected.
			return None;
		}

		if uses_thinking_levels(model) {
			let level = match effort {
				"minimal" | "low" | "medium" | "high" => effort,
				_ => "medium",
			};
			Some(vg::ThinkingConfig {
				thinking_level: Some(level.to_string()),
				thinking_budget: None,
				include_thoughts: Some(true),
			})
		} else {
			// Gemini 2.5: map to a conservative integer budget valid for Flash and Pro.
			// "minimal" is coerced to "low" (no 2.5 analogue).
			let budget = match effort {
				"minimal" | "low" => THINKING_BUDGET_LOW,
				"medium" => THINKING_BUDGET_MEDIUM,
				"high" => THINKING_BUDGET_HIGH,
				_ => THINKING_BUDGET_MEDIUM,
			};
			Some(vg::ThinkingConfig {
				thinking_level: None,
				thinking_budget: Some(budget),
				include_thoughts: Some(true),
			})
		}
	}
}

pub mod from_messages {
	use std::collections::HashMap;

	use serde_json::{Value, json};

	use super::*;
	use crate::conversion::completions::from_messages::anthropic_source_to_url;

	pub fn translate(
		req: &types::messages::Request,
		configured_model: Option<&str>,
	) -> Result<Vec<u8>, AIError> {
		let typed: types::messages::typed::Request =
			crate::json::convert(req).map_err(AIError::RequestParsing)?;
		let out = build_request(&typed, configured_model, &req.rest)?;
		serde_json::to_vec(&out).map_err(AIError::RequestMarshal)
	}

	fn build_request(
		req: &types::messages::typed::Request,
		configured_model: Option<&str>,
		rest: &Value,
	) -> Result<vg::GenerateContentRequest, AIError> {
		use types::messages::typed as mt;

		let model = configured_model.unwrap_or(&req.model).to_string();
		let contents = messages_to_contents(&req.messages)?;

		let contents = if contents.is_empty() {
			vec![vg::Content {
				role: Some("user".to_string()),
				parts: vec![text_part(" ")],
				rest: Value::Null,
			}]
		} else {
			contents
		};

		use types::messages::typed::{ContentBlock, Role};
		let mut system_parts: Vec<&str> = Vec::new();
		if let Some(sys) = &req.system {
			match sys {
				mt::SystemPrompt::Text(t) if !t.is_empty() => system_parts.push(t),
				mt::SystemPrompt::Blocks(blocks) => {
					system_parts.extend(blocks.iter().filter_map(|b| match b {
						mt::SystemContentBlock::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
						_ => None,
					}))
				},
				_ => {},
			}
		}
		system_parts.extend(
			req
				.messages
				.iter()
				.filter(|m| m.role == Role::System)
				.flat_map(|m| m.content.iter())
				.filter_map(|b| {
					if let ContentBlock::Text(t) = b {
						Some(t.text.as_str())
					} else {
						None
					}
				})
				.filter(|s| !s.is_empty()),
		);
		let system_instruction = (!system_parts.is_empty()).then(|| vg::Content {
			role: None,
			parts: vec![text_part(&system_parts.join("\n"))],
			rest: Value::Null,
		});

		let tools = build_tools(req);
		let tool_config = build_tool_config(req);
		let generation_config = build_generation_config(req, &model);

		let (cached_content, safety_settings, labels) = super::apply_rest_extras(rest);
		let (system_instruction, tools, tool_config) =
			super::drop_if_cached(&cached_content, system_instruction, tools, tool_config);

		Ok(vg::GenerateContentRequest {
			contents,
			system_instruction,
			tools,
			tool_config,
			generation_config,
			safety_settings,
			cached_content,
			labels,
		})
	}

	fn messages_to_contents(
		messages: &[types::messages::typed::Message],
	) -> Result<Vec<vg::Content>, AIError> {
		use types::messages::typed::{ContentBlock, Role};

		// Prepass: build base_id -> (name, call_index) from assistant ToolUse blocks.
		// The base id is after splitting off any embedded __thought__ signature suffix.
		let call_meta: HashMap<String, (String, usize)> = messages
			.iter()
			.filter(|m| m.role == Role::Assistant)
			.flat_map(|m| {
				m.content.iter().filter_map(|b| {
					if let ContentBlock::ToolUse { id, name, .. } = b {
						Some((id, name))
					} else {
						None
					}
				})
			})
			.enumerate()
			.map(|(idx, (id, name))| (split_tool_call_id(id).0.to_string(), (name.clone(), idx)))
			.collect();

		let mut contents: Vec<vg::Content> = Vec::new();

		for m in messages {
			match m.role {
				Role::User => {
					let mut parts: Vec<vg::Part> = Vec::new();
					for block in &m.content {
						match block {
							ContentBlock::Text(t) => parts.push(text_part(&t.text)),
							ContentBlock::Image(img) => {
								if let Some(url) = anthropic_source_to_url(&img.source) {
									parts.push(
										image_part(Some(&json!({ "url": url }))).map_err(|e| match e {
											AIError::InvalidResponse(m) => AIError::BadRequest(m),
											other => other,
										})?,
									);
								}
							},
							ContentBlock::ToolResult {
								tool_use_id,
								content,
								is_error,
								..
							} => {
								let base_id = split_tool_call_id(tool_use_id).0.to_string();
								// T3.3: fail loudly on an unknown tool_use_id. Sending the raw id
								// as `functionResponse.name` matches no declared function and
								// causes Gemini to silently ignore the result.
								let Some((name, _)) = call_meta.get(&base_id) else {
									return Err(AIError::BadRequest(strng::new(format!(
										"tool_result references unknown tool_use_id '{tool_use_id}'; \
										 all assistant turns containing the matching tool_use block \
										 must be included in the request"
									))));
								};
								let name = name.clone();
								let text = tool_result_text(content)?;
								let mut response = json!({ "content": text });
								if *is_error == Some(true) {
									response["is_error"] = json!(true);
								}
								// Carry base_id as a transient correlation key; stripped by
								// reorder_function_responses after reordering.
								parts.push(vg::Part::FunctionResponse(vg::FunctionResponsePart {
									function_response: vg::FunctionResponse {
										name,
										id: Some(base_id),
										response,
										rest: Value::Null,
									},
									rest: Value::Null,
								}));
							},
							_ => {},
						}
					}
					push_content(&mut contents, "user", parts);
				},
				Role::Assistant => {
					let mut parts: Vec<vg::Part> = Vec::new();
					for block in &m.content {
						match block {
							ContentBlock::Text(t) if !t.text.is_empty() => {
								parts.push(text_part(&t.text));
							},
							ContentBlock::Thinking {
								thinking,
								signature,
							} => {
								parts.push(vg::Part::Text(vg::TextPart {
									text: thinking.clone(),
									thought: Some(true),
									thought_signature: if signature.is_empty() {
										None
									} else {
										Some(signature.clone())
									},
									rest: Value::Null,
								}));
							},
							ContentBlock::ToolUse {
								id, name, input, ..
							} => {
								let (_, thought_signature) = split_tool_call_id(id);
								// Invariant: Vertex rejects `id` on functionCall parts.
								parts.push(vg::Part::FunctionCall(vg::FunctionCallPart {
									function_call: vg::FunctionCall {
										name: name.clone(),
										id: None,
										args: input.clone(),
										rest: Value::Null,
									},
									thought: None,
									thought_signature: thought_signature.map(str::to_string),
									rest: Value::Null,
								}));
							},
							_ => {},
						}
					}
					push_content(&mut contents, "model", parts);
				},
				Role::System => {
					// Collected into systemInstruction in build_request; skip here.
				},
			}
		}

		// Vertex correlates functionResponse to functionCall positionally (no id), so responses must
		// follow the call order even when the client returns them out of order.
		super::reorder_function_responses(&mut contents, &call_meta);

		Ok(contents)
	}

	fn tool_result_text(
		content: &types::messages::typed::ToolResultContent,
	) -> Result<String, AIError> {
		use types::messages::typed::{ToolResultContent, ToolResultContentPart};
		match content {
			ToolResultContent::Text(s) => Ok(s.clone()),
			ToolResultContent::Array(parts) => {
				if parts
					.iter()
					.any(|p| !matches!(p, ToolResultContentPart::Text { .. }))
				{
					return Err(AIError::BadRequest(strng::new(
						"tool_result with image or document content is not supported on this provider",
					)));
				}
				Ok(
					parts
						.iter()
						.filter_map(|p| match p {
							ToolResultContentPart::Text { text, .. } => Some(text.as_str()),
							_ => None,
						})
						.collect::<String>(),
				)
			},
		}
	}

	fn build_tools(req: &types::messages::typed::Request) -> Vec<vg::Tool> {
		let Some(tools) = &req.tools else {
			return Vec::new();
		};
		let decls: Vec<vg::FunctionDeclaration> = tools
			.iter()
			.map(|t| vg::FunctionDeclaration {
				name: t.name.clone(),
				description: t.description.clone(),
				parameters: Some(super::from_completions::normalize_gemini_schema(
					&t.input_schema,
				)),
			})
			.collect();
		super::wrap_tool_declarations(decls)
	}

	fn build_tool_config(req: &types::messages::typed::Request) -> Option<vg::ToolConfig> {
		use types::messages::typed::ToolChoice;
		let tc = req.tool_choice.as_ref()?;
		let (disable_parallel, cfg) = match tc {
			ToolChoice::None {} => (
				None,
				vg::FunctionCallingConfig {
					mode: Some("NONE".into()),
					..Default::default()
				},
			),
			ToolChoice::Auto {
				disable_parallel_tool_use,
			} => (
				disable_parallel_tool_use.as_ref(),
				vg::FunctionCallingConfig {
					mode: Some("AUTO".into()),
					..Default::default()
				},
			),
			ToolChoice::Any {
				disable_parallel_tool_use,
			} => (
				disable_parallel_tool_use.as_ref(),
				vg::FunctionCallingConfig {
					mode: Some("ANY".into()),
					..Default::default()
				},
			),
			ToolChoice::Tool {
				name,
				disable_parallel_tool_use,
			} => (
				disable_parallel_tool_use.as_ref(),
				vg::FunctionCallingConfig {
					mode: Some("ANY".into()),
					allowed_function_names: vec![name.clone()],
				},
			),
		};
		if disable_parallel.copied().unwrap_or(false) {
			tracing::warn!("disable_parallel_tool_use is not supported on Vertex Gemini; ignored");
		}
		Some(vg::ToolConfig {
			function_calling_config: Some(cfg),
		})
	}

	fn build_generation_config(
		req: &types::messages::typed::Request,
		model: &str,
	) -> Option<vg::GenerationConfig> {
		use types::messages::typed as mt;

		let (response_mime_type, response_schema) = req
			.output_config
			.as_ref()
			.and_then(|oc| oc.format.as_ref())
			.map(|fmt| match fmt {
				mt::OutputFormat::JsonSchema { schema } => (
					Some("application/json".to_string()),
					Some(super::from_completions::normalize_gemini_schema(schema)),
				),
			})
			.unwrap_or((None, None));

		let thinking_config = build_thinking_config(req, model);

		let cfg = vg::GenerationConfig {
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: req.top_k.map(|v| v as u32),
			frequency_penalty: None,
			presence_penalty: None,
			max_output_tokens: Some(u32::try_from(req.max_tokens).unwrap_or(u32::MAX)),
			stop_sequences: req.stop_sequences.clone(),
			candidate_count: None,
			seed: None,
			response_mime_type,
			response_schema,
			thinking_config,
		};

		if cfg == vg::GenerationConfig::default() {
			None
		} else {
			Some(cfg)
		}
	}

	fn build_thinking_config(
		req: &types::messages::typed::Request,
		model: &str,
	) -> Option<vg::ThinkingConfig> {
		use types::messages::typed as mt;

		// output_config.effort takes precedence when present.
		if let Some(effort) = req.output_config.as_ref().and_then(|oc| oc.effort) {
			return Some(effort_to_thinking_config(effort, model));
		}

		match req.thinking.as_ref()? {
			mt::ThinkingInput::Disabled {} => None,
			mt::ThinkingInput::Adaptive {} => {
				if uses_thinking_levels(model) {
					// Gemini 3: omit thinkingConfig; model adapts on its own.
					None
				} else {
					// Gemini 2.5: -1 signals dynamic budget.
					Some(vg::ThinkingConfig {
						thinking_level: None,
						thinking_budget: Some(-1),
						include_thoughts: Some(true),
					})
				}
			},
			mt::ThinkingInput::Enabled { budget_tokens } => {
				if uses_thinking_levels(model) {
					let level = if *budget_tokens <= THINKING_BUDGET_LOW as u64 {
						"low"
					} else if *budget_tokens <= THINKING_BUDGET_MEDIUM as u64 {
						"medium"
					} else {
						"high"
					};
					Some(vg::ThinkingConfig {
						thinking_level: Some(level.to_string()),
						thinking_budget: None,
						include_thoughts: Some(true),
					})
				} else {
					Some(vg::ThinkingConfig {
						thinking_level: None,
						thinking_budget: Some(i32::try_from(*budget_tokens).unwrap_or(i32::MAX)),
						include_thoughts: Some(true),
					})
				}
			},
		}
	}

	fn effort_to_thinking_config(
		effort: types::messages::typed::ThinkingEffort,
		model: &str,
	) -> vg::ThinkingConfig {
		use types::messages::typed::ThinkingEffort;
		if uses_thinking_levels(model) {
			let level = match effort {
				ThinkingEffort::Low => "low",
				ThinkingEffort::Medium => "medium",
				ThinkingEffort::High | ThinkingEffort::Xhigh | ThinkingEffort::Max => "high",
			};
			vg::ThinkingConfig {
				thinking_level: Some(level.to_string()),
				thinking_budget: None,
				include_thoughts: Some(true),
			}
		} else {
			let budget = match effort {
				ThinkingEffort::Low => THINKING_BUDGET_LOW,
				ThinkingEffort::Medium => THINKING_BUDGET_MEDIUM,
				ThinkingEffort::High | ThinkingEffort::Xhigh | ThinkingEffort::Max => THINKING_BUDGET_HIGH,
			};
			vg::ThinkingConfig {
				thinking_level: None,
				thinking_budget: Some(budget),
				include_thoughts: Some(true),
			}
		}
	}
}

#[derive(Default)]
struct DecodedParts<'a> {
	content: String,
	reasoning: String,
	calls: Vec<DecodedCall<'a>>,
	/// The `thoughtSignature` from the last thought `TextPart` with a non-empty signature.
	/// Last non-empty wins: Gemini 3 places the signature on the final thought part of a turn.
	/// `to_completions` ignores this field; it exists for `to_messages`.
	reasoning_signature: Option<&'a str>,
}

/// Borrows the function-call fields from the source content; the callers own the single
/// `String`/`Value` allocation the typed message requires, so the decode itself clones nothing.
struct DecodedCall<'a> {
	id: Option<&'a str>,
	name: &'a str,
	args: &'a serde_json::Value,
	thought_signature: Option<&'a str>,
}

fn decode_parts<'a>(content: Option<&'a vg::Content>) -> DecodedParts<'a> {
	let mut out = DecodedParts::default();
	let Some(content) = content else {
		return out;
	};
	for part in &content.parts {
		match part {
			vg::Part::Text(t) if t.thought == Some(true) => {
				out.reasoning.push_str(&t.text);
				if let Some(sig) = t.thought_signature.as_deref()
					&& !sig.is_empty()
				{
					out.reasoning_signature = Some(sig);
				}
			},
			vg::Part::Text(t) => out.content.push_str(&t.text),
			vg::Part::FunctionCall(fc) => out.calls.push(DecodedCall {
				id: fc.function_call.id.as_deref(),
				name: &fc.function_call.name,
				args: &fc.function_call.args,
				thought_signature: fc.thought_signature.as_deref(),
			}),
			_ => {},
		}
	}
	out
}

fn map_finish_reason(reason: Option<&str>) -> crate::types::completions::typed::FinishReason {
	use crate::types::completions::typed::FinishReason;
	match reason {
		Some("MAX_TOKENS") => FinishReason::Length,
		Some(
			"SAFETY"
			| "RECITATION"
			| "LANGUAGE"
			| "BLOCKLIST"
			| "PROHIBITED_CONTENT"
			| "SPII"
			| "UNEXPECTED_TOOL_CALL"
			| "TOO_MANY_TOOL_CALLS"
			| "IMAGE_SAFETY"
			| "IMAGE_PROHIBITED_CONTENT"
			| "IMAGE_RECITATION",
		) => FinishReason::ContentFilter,
		// STOP, MALFORMED_FUNCTION_CALL, IMAGE_OTHER, NO_IMAGE, OTHER,
		// FINISH_REASON_UNSPECIFIED, None, and any future value.
		_ => FinishReason::Stop,
	}
}

/// Prompt, completion, and total token counts from Gemini usage metadata
/// (total falls back to prompt + completion when absent).
fn usage_counts(um: &vg::UsageMetadata) -> (u64, u64, u64) {
	let prompt = um.prompt_token_count.unwrap_or(0);
	let completion = um.candidates_token_count.unwrap_or(0);
	let total = um.total_token_count.unwrap_or(prompt + completion);
	(prompt, completion, total)
}

pub mod to_completions {
	use std::time::Instant;

	use axum_core::body::Body;
	use futures_util::StreamExt;
	use futures_util::stream::{self, BoxStream};
	use serde_json::Value;

	use super::*;
	use crate::types::completions::typed as completions;
	use crate::{StreamingUsageGuard, json, parse};

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp: vg::GenerateContentResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		let typed = build_response(&resp);
		let inner =
			json::convert::<_, types::completions::Response>(&typed).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(inner))
	}

	pub(super) fn encode_args(args: &Value) -> String {
		if args.is_null() {
			return "{}".to_string();
		}
		serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
	}

	fn tool_call_id(native: Option<&str>, seed: &str, index: u32) -> String {
		native
			.map(str::to_string)
			.unwrap_or_else(|| format!("call_{seed}_{index}"))
	}

	fn assistant_message(
		content: Option<String>,
		reasoning_content: Option<String>,
		tool_calls: Option<Vec<completions::MessageToolCalls>>,
	) -> completions::ResponseMessage {
		completions::ResponseMessage {
			role: completions::Role::Assistant,
			content,
			reasoning_content,
			// Known limitation: a Gemini-3 `thoughtSignature` on a reasoning-only turn (no tool call
			// to carry it in a tool_call id) is dropped, since the OpenAI shape has no signature slot.
			// Tool-call signatures still round-trip via the tool_call id.
			reasoning_signature: None,
			tool_calls,
			#[allow(deprecated)]
			function_call: None,
			refusal: None,
			audio: None,
			extra: None,
		}
	}

	fn build_response(resp: &vg::GenerateContentResponse) -> completions::Response {
		let model = resp.model_version.clone().unwrap_or_default();
		let id = resp
			.response_id
			.clone()
			.unwrap_or_else(|| format!("vertex-gemini-{}", chrono::Utc::now().timestamp_millis()));
		let created = chrono::Utc::now().timestamp() as u32;

		let choices = if resp.candidates.is_empty() {
			let blocked = resp
				.prompt_feedback
				.as_ref()
				.and_then(|pf| pf.block_reason.as_ref())
				.is_some();
			let finish = if blocked {
				completions::FinishReason::ContentFilter
			} else {
				completions::FinishReason::Stop
			};
			vec![completions::ChatChoice {
				index: 0,
				message: assistant_message(Some(String::new()), None, None),
				finish_reason: Some(finish),
				logprobs: None,
			}]
		} else {
			resp
				.candidates
				.iter()
				.enumerate()
				.map(|(i, cand)| build_choice(i as u32, cand, &id))
				.collect()
		};

		completions::Response {
			id,
			object: "chat.completion".to_string(),
			created,
			model,
			choices,
			usage: resp.usage_metadata.as_ref().map(build_usage),
			service_tier: None,
			system_fingerprint: None,
		}
	}

	fn build_choice(index: u32, cand: &vg::Candidate, request_id: &str) -> completions::ChatChoice {
		let decoded = decode_parts(cand.content.as_ref());

		// as parallel candidates reuse call indices starting at 0, incorporate the candidate index into the seed
		let seed = if index == 0 {
			request_id.to_string()
		} else {
			format!("{request_id}_{index}")
		};
		let tool_calls: Vec<completions::MessageToolCalls> = decoded
			.calls
			.iter()
			.enumerate()
			.map(|(idx, call)| {
				completions::MessageToolCalls::Function(completions::MessageToolCall {
					// Embed any thoughtSignature into the id so the client echoes it back (Gemini 3
					// requires it on the next turn) recovered before the outbound Vertex request.
					id: join_tool_call_id(
						tool_call_id(call.id, &seed, idx as u32),
						call.thought_signature,
					),
					function: completions::FunctionCall {
						name: call.name.to_string(),
						arguments: encode_args(call.args),
					},
				})
			})
			.collect();

		let has_tool_calls = !tool_calls.is_empty();
		let finish = finish_with_tool_override(cand.finish_reason.as_deref(), has_tool_calls);
		let content = if decoded.content.is_empty() && (has_tool_calls || !decoded.reasoning.is_empty())
		{
			None
		} else {
			Some(decoded.content)
		};
		let reasoning = (!decoded.reasoning.is_empty()).then_some(decoded.reasoning);
		let tool_calls = has_tool_calls.then_some(tool_calls);

		completions::ChatChoice {
			index,
			message: assistant_message(content, reasoning, tool_calls),
			finish_reason: Some(finish),
			logprobs: None,
		}
	}

	pub(super) fn finish_with_tool_override(
		reason: Option<&str>,
		saw_tool_call: bool,
	) -> completions::FinishReason {
		let mapped = map_finish_reason(reason);
		if saw_tool_call && matches!(mapped, completions::FinishReason::Stop) {
			completions::FinishReason::ToolCalls
		} else {
			mapped
		}
	}

	/// Per-stream state for translating native Gemini SSE chunks into OpenAI
	/// `chat.completion.chunk`s. Carries the cross-chunk invariants: `role` is emitted
	/// once, tool-call ids/indices are assigned in order, and the finish reason gets the
	/// tool-call override if any function call was seen in the stream.
	pub(super) struct StreamState {
		created: u32,
		stream_id: Option<String>,
		model_version: String,
		role_emitted: bool,
		saw_function_call: bool,
		tool_index: u32,
	}

	impl StreamState {
		pub(super) fn new() -> Self {
			Self {
				created: chrono::Utc::now().timestamp() as u32,
				stream_id: None,
				model_version: String::new(),
				role_emitted: false,
				saw_function_call: false,
				tool_index: 0,
			}
		}
		pub(super) fn translate(
			&mut self,
			chunk: &vg::GenerateContentResponse,
		) -> Option<completions::StreamResponse> {
			let id = self
				.stream_id
				.get_or_insert_with(|| {
					chunk
						.response_id
						.clone()
						.unwrap_or_else(|| format!("vertex-gemini-{}", self.created))
				})
				.clone();
			if self.model_version.is_empty()
				&& let Some(m) = &chunk.model_version
			{
				self.model_version = m.clone();
			}

			let mut delta = completions::StreamResponseDelta::default();
			if !self.role_emitted {
				self.role_emitted = true;
				delta.role = Some(completions::Role::Assistant);
			}

			// Use only the first answer; streaming multiple `candidates` is rare (most models return
			// one, some reject asking for more) and unsupported here. Non-streaming returns all.
			let cand = chunk.candidates.first();
			let decoded = decode_parts(cand.and_then(|c| c.content.as_ref()));

			let mut tool_calls = Vec::new();
			for call in &decoded.calls {
				self.saw_function_call = true;
				let idx = self.tool_index;
				self.tool_index += 1;
				tool_calls.push(completions::ChatCompletionMessageToolCallChunk {
					index: idx,
					id: Some(join_tool_call_id(
						tool_call_id(call.id, &id, idx),
						call.thought_signature,
					)),
					r#type: Some(completions::FunctionType::Function),
					function: Some(completions::FunctionCallStream {
						name: Some(call.name.to_string()),
						arguments: Some(encode_args(call.args)),
					}),
				});
			}

			// `saw_function_call` carries across chunks, so a finish reason in a later chunk still
			// upgrades to `tool_calls` when an earlier chunk emitted the call.
			let finish = match cand.and_then(|c| c.finish_reason.as_deref()) {
				Some(reason) => Some(finish_with_tool_override(
					Some(reason),
					self.saw_function_call,
				)),
				None
					if chunk.candidates.is_empty()
						&& chunk
							.prompt_feedback
							.as_ref()
							.and_then(|pf| pf.block_reason.as_ref())
							.is_some() =>
				{
					Some(completions::FinishReason::ContentFilter)
				},
				None => None,
			};

			if !decoded.content.is_empty() {
				delta.content = Some(decoded.content);
			}
			if !decoded.reasoning.is_empty() {
				delta.reasoning_content = Some(decoded.reasoning);
			}
			if !tool_calls.is_empty() {
				delta.tool_calls = Some(tool_calls);
			}

			let has_delta = delta.role.is_some()
				|| delta.content.is_some()
				|| delta.reasoning_content.is_some()
				|| delta.tool_calls.is_some();
			// Gemini attaches cumulative usageMetadata to interim content chunks too; only surface it
			// on a terminal chunk (one carrying finish_reason, or a usage-only chunk with no delta) so
			// the client sees a single OpenAI-style final usage rather than a growing total on every
			// chunk. Telemetry and rate-limit accounting read the cumulative counts separately in
			// translate_stream, so suppressing it here does not affect them.
			let usage = chunk
				.usage_metadata
				.as_ref()
				.filter(|_| finish.is_some() || !has_delta)
				.map(build_usage);
			let choices = if has_delta || finish.is_some() {
				vec![completions::ChatChoiceStream {
					index: 0,
					delta,
					finish_reason: finish,
					logprobs: None,
				}]
			} else {
				vec![]
			};
			if choices.is_empty() && usage.is_none() {
				return None;
			}

			Some(completions::StreamResponse {
				id,
				choices,
				created: self.created,
				model: self.model_version.clone(),
				service_tier: None,
				system_fingerprint: None,
				object: "chat.completion.chunk".to_string(),
				usage,
			})
		}
	}

	/// Translate a native Gemini `:streamGenerateContent?alt=sse` stream into OpenAI
	/// `chat.completion.chunk` SSE. Gemini ends the HTTP stream without a `[DONE]`
	/// sentinel, so one is appended on successful close.
	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		model: Strng,
		log: StreamingUsageGuard,
	) -> Body {
		let mut state = StreamState::new();
		let mut saw_token = false;
		let body = parse::sse::json_transform_multi::<
			vg::GenerateContentResponse,
			completions::StreamResponse,
			_,
		>(b, buffer_limit, move |ev| {
			let chunk = match ev {
				parse::sse::SseJsonEvent::Data(Ok(c)) => c,
				parse::sse::SseJsonEvent::Data(Err(e)) => {
					tracing::debug!("failed to parse gemini stream chunk: {e}");
					return vec![];
				},
				parse::sse::SseJsonEvent::Done => return vec![],
			};

			if !saw_token {
				saw_token = true;
				log.update(|r| r.response.first_token = Some(Instant::now()));
			}
			if let Some(m) = &chunk.model_version {
				log.update(|r| {
					if r.response.provider_model.is_none() {
						r.response.provider_model = Some(strng::new(m));
					}
				});
			}
			if let Some(um) = &chunk.usage_metadata {
				let (prompt, completion, total) = usage_counts(um);
				log.update(|r| {
					r.response.input_tokens = Some(prompt);
					r.response.output_tokens = Some(completion);
					r.response.total_tokens = Some(total);
					r.response.cached_input_tokens = um.cached_content_token_count;
					r.response.reasoning_tokens = um.thoughts_token_count;
				});
			}

			match state.translate(&chunk) {
				// Gemini may omit modelVersion on a chunk
				Some(mut sr) => {
					if sr.model.is_empty() {
						sr.model = model.to_string();
					}
					vec![("", sr)]
				},
				None => vec![],
			}
		});
		append_done_on_close(body.into_data_stream())
	}

	/// Gemini ends the HTTP stream without a `[DONE]` sentinel; append one on successful close
	/// (mirrors `conversion::bedrock::from_completions::append_done_on_success`).
	pub(super) fn append_done_on_close<S>(stream: S) -> Body
	where
		S: futures_core::Stream<Item = Result<Bytes, axum_core::Error>> + Send + 'static,
	{
		let done = crate::parse::encode_sse_event("", Bytes::from_static(b"[DONE]"));
		let stream = stream::unfold(
			(Some(stream.boxed()), Some(done)),
			|(stream, done): (
				Option<BoxStream<'static, Result<Bytes, axum_core::Error>>>,
				Option<Bytes>,
			)| async move {
				let mut stream = stream?;
				match stream.next().await {
					Some(Ok(chunk)) => Some((Ok(chunk), (Some(stream), done))),
					Some(Err(err)) => Some((Err(err), (None, None))),
					None => done.map(|done| (Ok(done), (None, None))),
				}
			},
		);
		Body::from_stream(stream)
	}

	fn build_usage(um: &vg::UsageMetadata) -> completions::Usage {
		let (prompt, completion, total) = usage_counts(um);
		completions::Usage {
			prompt_tokens: prompt as u32,
			completion_tokens: completion as u32,
			total_tokens: total as u32,
			prompt_tokens_details: um.cached_content_token_count.map(|c| {
				completions::UsagePromptDetails {
					cached_tokens: Some(c),
					audio_tokens: None,
					cache_write_tokens: None,
					rest: Value::Null,
				}
			}),
			completion_tokens_details: um.thoughts_token_count.map(|t| {
				completions::UsageCompletionDetails {
					reasoning_tokens: Some(t),
					audio_tokens: None,
					rest: Value::Null,
				}
			}),
			cache_read_input_tokens: None,
			cache_creation_input_tokens: None,
		}
	}
}

pub mod to_messages {
	use std::time::Instant;

	use agent_core::strng;
	use axum_core::body::Body;
	use bytes::Bytes;

	use super::*;
	use crate::types::messages::typed as messages;
	use crate::{AIError, StreamingUsageGuard, json, logged_response_parsing, parse, types};

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp: vg::GenerateContentResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		let typed = build_response(&resp);
		let inner =
			json::convert::<_, types::messages::Response>(&typed).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(inner))
	}

	fn build_response(resp: &vg::GenerateContentResponse) -> messages::MessagesResponse {
		let model = resp.model_version.clone().unwrap_or_default();
		let id = resp
			.response_id
			.clone()
			.unwrap_or_else(|| format!("msg-vertex-{}", chrono::Utc::now().timestamp_millis()));

		let (content, stop_reason) = if resp.candidates.is_empty() {
			let blocked = resp
				.prompt_feedback
				.as_ref()
				.and_then(|pf| pf.block_reason.as_ref())
				.is_some();
			let reason = if blocked {
				messages::StopReason::Refusal
			} else {
				messages::StopReason::EndTurn
			};
			(vec![], Some(reason))
		} else {
			let cand = &resp.candidates[0];
			let decoded = decode_parts(cand.content.as_ref());
			let has_calls = !decoded.calls.is_empty();

			let finish =
				to_completions::finish_with_tool_override(cand.finish_reason.as_deref(), has_calls);
			let stop_reason = crate::conversion::messages::finish_reason_to_stop_reason(finish);

			let seed = id.as_str();
			let mut blocks: Vec<messages::ContentBlock> = Vec::new();

			// Block emission order: Thinking → Text → ToolUse
			if !decoded.reasoning.is_empty() {
				blocks.push(messages::ContentBlock::Thinking {
					thinking: decoded.reasoning,
					signature: decoded.reasoning_signature.unwrap_or("").to_string(),
				});
			}
			if !decoded.content.is_empty() {
				blocks.push(messages::ContentBlock::Text(messages::ContentTextBlock {
					text: decoded.content,
					citations: None,
					cache_control: None,
				}));
			}
			blocks.extend(decoded.calls.iter().enumerate().map(|(idx, call)| {
				let base_id = call
					.id
					.map(str::to_string)
					.unwrap_or_else(|| format!("toolu_{seed}_{idx}"));
				let id = join_tool_call_id(base_id, call.thought_signature);
				messages::ContentBlock::ToolUse {
					id,
					name: call.name.to_string(),
					input: if call.args.is_null() {
						serde_json::Value::Object(Default::default())
					} else {
						call.args.clone()
					},
					cache_control: None,
				}
			}));

			(blocks, Some(stop_reason))
		};

		messages::MessagesResponse {
			id,
			r#type: "message".to_string(),
			role: messages::Role::Assistant,
			content,
			model,
			stop_reason,
			stop_sequence: None,
			usage: build_usage_messages(resp.usage_metadata.as_ref()),
			input_audio_tokens: None,
			output_audio_tokens: None,
		}
	}

	fn build_usage_messages(um: Option<&vg::UsageMetadata>) -> messages::Usage {
		let Some(um) = um else {
			return messages::Usage {
				input_tokens: 0,
				output_tokens: 0,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				service_tier: None,
			};
		};
		let prompt = um.prompt_token_count.unwrap_or(0) as usize;
		let cached = um.cached_content_token_count.unwrap_or(0) as usize;
		let completion = um.candidates_token_count.unwrap_or(0) as usize;
		messages::Usage {
			input_tokens: prompt,
			output_tokens: completion,
			cache_creation_input_tokens: None,
			cache_read_input_tokens: (cached > 0).then_some(cached),
			service_tier: None,
		}
	}

	/// Which kind of content block (if any) is currently open in the stream.
	#[derive(Default)]
	enum OpenBlock {
		#[default]
		None,
		Text(usize),
		Thinking(usize),
	}

	pub(super) struct StreamState {
		stream_id: Option<String>,
		model_version: String,
		message_started: bool,
		message_stop_sent: bool,
		block_index: usize,
		open_block: OpenBlock,
		saw_tool_call: bool,
		tool_call_index: u32,
		saw_token: bool,
		// Accumulated for flush_message_end; populated when finish_reason arrives.
		pending_stop_reason: Option<messages::StopReason>,
		pending_input_tokens: usize,
		pending_output_tokens: usize,
		pending_cache_read: Option<usize>,
	}

	impl StreamState {
		pub(super) fn new() -> Self {
			Self {
				stream_id: None,
				model_version: String::new(),
				message_started: false,
				message_stop_sent: false,
				block_index: 0,
				open_block: OpenBlock::None,
				saw_tool_call: false,
				tool_call_index: 0,
				saw_token: false,
				pending_stop_reason: None,
				pending_input_tokens: 0,
				pending_output_tokens: 0,
				pending_cache_read: None,
			}
		}

		fn record_first_token(&mut self, log: &StreamingUsageGuard) {
			if !self.saw_token {
				self.saw_token = true;
				log.update(|r| r.response.first_token = Some(Instant::now()));
			}
		}

		/// Emit `message_delta` + `message_stop` exactly once.
		///
		/// If called from the data handler (finish_reason present), `force` is false and the
		/// stop_reason was already stored in `self.pending_stop_reason`.  If called from the
		/// Done handler (stream closed cleanly but no finishReason arrived), `force` is true
		/// and we fall back to `EndTurn` so the client always receives a well-terminated message.
		fn flush_message_end(
			&mut self,
			force: bool,
			out: &mut Vec<(&'static str, messages::MessagesStreamEvent)>,
		) {
			if self.message_stop_sent {
				return;
			}
			if !self.message_started {
				return;
			}
			let stop_reason = self.pending_stop_reason.take().or(if force {
				Some(messages::StopReason::EndTurn)
			} else {
				None
			});
			let Some(stop_reason) = stop_reason else {
				return;
			};
			out.push(
				messages::MessagesStreamEvent::MessageDelta {
					delta: messages::MessageDelta {
						stop_reason: Some(stop_reason),
						stop_sequence: None,
					},
					usage: messages::MessageDeltaUsage {
						input_tokens: Some(self.pending_input_tokens),
						output_tokens: Some(self.pending_output_tokens),
						cache_creation_input_tokens: None,
						cache_read_input_tokens: self.pending_cache_read,
					},
				}
				.into_sse_tuple(),
			);
			out.push(messages::MessagesStreamEvent::MessageStop.into_sse_tuple());
			self.message_stop_sent = true;
		}

		/// Emit a `content_block_stop` for the currently open block, and return the index.
		fn close_open_block(&mut self, out: &mut Vec<(&'static str, messages::MessagesStreamEvent)>) {
			let idx = match self.open_block {
				OpenBlock::None => return,
				OpenBlock::Text(i) | OpenBlock::Thinking(i) => i,
			};
			out.push(messages::MessagesStreamEvent::ContentBlockStop { index: idx }.into_sse_tuple());
			self.open_block = OpenBlock::None;
			self.block_index += 1;
		}

		pub(super) fn translate(
			&mut self,
			chunk: &vg::GenerateContentResponse,
			log: &StreamingUsageGuard,
		) -> Vec<(&'static str, messages::MessagesStreamEvent)> {
			let mut out: Vec<(&'static str, messages::MessagesStreamEvent)> = Vec::new();

			let id = self
				.stream_id
				.get_or_insert_with(|| {
					chunk
						.response_id
						.clone()
						.unwrap_or_else(|| format!("msg-vtx-{}", chrono::Utc::now().timestamp_millis()))
				})
				.clone();
			if self.model_version.is_empty()
				&& let Some(m) = &chunk.model_version
			{
				self.model_version = m.clone();
			}

			if chunk.usage_metadata.is_some() || chunk.model_version.is_some() {
				log.update(|r| {
					if let Some(um) = &chunk.usage_metadata {
						let (prompt, completion, total) = usage_counts(um);
						r.response.input_tokens = Some(prompt);
						r.response.output_tokens = Some(completion);
						r.response.total_tokens = Some(total);
						r.response.cached_input_tokens = um.cached_content_token_count;
						r.response.reasoning_tokens = um.thoughts_token_count;
					}
					if let Some(m) = &chunk.model_version
						&& r.response.provider_model.is_none()
					{
						r.response.provider_model = Some(strng::new(m));
					}
				});
			}

			// Emit message_start on the first chunk that has any candidate or usage.
			if !self.message_started {
				self.message_started = true;
				// Gemini only sends usageMetadata on the final streaming chunk, so
				// input_tokens is always 0 here. The correct count arrives later via
				// flush_message_end, which emits it in message_delta.usage.input_tokens.
				let mut usage = build_usage_messages(chunk.usage_metadata.as_ref());
				usage.output_tokens = 0;
				let stub = messages::MessagesResponse {
					id: id.clone(),
					r#type: "message".to_string(),
					role: messages::Role::Assistant,
					content: vec![],
					model: self.model_version.clone(),
					stop_reason: None,
					stop_sequence: None,
					usage,
					input_audio_tokens: None,
					output_audio_tokens: None,
				};
				out.push(messages::MessagesStreamEvent::MessageStart { message: stub }.into_sse_tuple());
			}

			let cand = chunk.candidates.first();

			// Emit per-part deltas.
			if let Some(content) = cand.and_then(|c| c.content.as_ref()) {
				for part in &content.parts {
					match part {
						vg::Part::Text(t) if t.thought == Some(true) => {
							self.record_first_token(log);
							// Close any open text block before starting/continuing a thinking block.
							if matches!(self.open_block, OpenBlock::Text(_)) {
								self.close_open_block(&mut out);
							}
							if matches!(self.open_block, OpenBlock::None) {
								let idx = self.block_index;
								out.push(
									messages::MessagesStreamEvent::ContentBlockStart {
										index: idx,
										content_block: messages::ContentBlock::Thinking {
											thinking: String::new(),
											signature: String::new(),
										},
									}
									.into_sse_tuple(),
								);
								self.open_block = OpenBlock::Thinking(idx);
							}
							let idx = match self.open_block {
								OpenBlock::Thinking(i) => i,
								_ => unreachable!(),
							};
							out.push(
								messages::MessagesStreamEvent::ContentBlockDelta {
									index: idx,
									delta: messages::ContentBlockDelta::ThinkingDelta {
										thinking: t.text.clone(),
									},
								}
								.into_sse_tuple(),
							);
							if let Some(sig) = t.thought_signature.as_deref()
								&& !sig.is_empty()
							{
								out.push(
									messages::MessagesStreamEvent::ContentBlockDelta {
										index: idx,
										delta: messages::ContentBlockDelta::SignatureDelta {
											signature: sig.to_string(),
										},
									}
									.into_sse_tuple(),
								);
							}
						},
						vg::Part::Text(t) => {
							self.record_first_token(log);
							// Close any open thinking block before starting/continuing a text block.
							if matches!(self.open_block, OpenBlock::Thinking(_)) {
								self.close_open_block(&mut out);
							}
							if matches!(self.open_block, OpenBlock::None) {
								let idx = self.block_index;
								out.push(
									messages::MessagesStreamEvent::ContentBlockStart {
										index: idx,
										content_block: messages::ContentBlock::Text(messages::ContentTextBlock {
											text: String::new(),
											citations: None,
											cache_control: None,
										}),
									}
									.into_sse_tuple(),
								);
								self.open_block = OpenBlock::Text(idx);
							}
							let idx = match self.open_block {
								OpenBlock::Text(i) => i,
								_ => unreachable!(),
							};
							out.push(
								messages::MessagesStreamEvent::ContentBlockDelta {
									index: idx,
									delta: messages::ContentBlockDelta::TextDelta {
										text: t.text.clone(),
									},
								}
								.into_sse_tuple(),
							);
						},
						vg::Part::FunctionCall(fc) => {
							self.record_first_token(log);
							self.close_open_block(&mut out);
							self.saw_tool_call = true;
							let call_idx = self.tool_call_index;
							self.tool_call_index += 1;
							let base_id = fc
								.function_call
								.id
								.as_deref()
								.map(str::to_string)
								.unwrap_or_else(|| format!("toolu_{id}_{call_idx}"));
							let tool_id = join_tool_call_id(base_id, fc.thought_signature.as_deref());
							let block_idx = self.block_index;
							out.push(
								messages::MessagesStreamEvent::ContentBlockStart {
									index: block_idx,
									content_block: messages::ContentBlock::ToolUse {
										id: tool_id,
										name: fc.function_call.name.clone(),
										input: serde_json::Value::Object(Default::default()),
										cache_control: None,
									},
								}
								.into_sse_tuple(),
							);
							let args_json = to_completions::encode_args(&fc.function_call.args);
							out.push(
								messages::MessagesStreamEvent::ContentBlockDelta {
									index: block_idx,
									delta: messages::ContentBlockDelta::InputJsonDelta {
										partial_json: args_json,
									},
								}
								.into_sse_tuple(),
							);
							out.push(
								messages::MessagesStreamEvent::ContentBlockStop { index: block_idx }
									.into_sse_tuple(),
							);
							self.block_index += 1;
						},
						_ => {},
					}
				}
			}

			// Emit message_delta + message_stop when finish_reason arrives or prompt was blocked.
			let finish_reason_str = cand.and_then(|c| c.finish_reason.as_deref());
			let prompt_blocked = cand.is_none()
				&& chunk
					.prompt_feedback
					.as_ref()
					.and_then(|pf| pf.block_reason.as_ref())
					.is_some();

			if let Some(um) = chunk.usage_metadata.as_ref() {
				let usage = build_usage_messages(Some(um));
				self.pending_input_tokens = usage.input_tokens;
				self.pending_output_tokens = usage.output_tokens;
				self.pending_cache_read = usage.cache_read_input_tokens;
			}

			if finish_reason_str.is_some() || prompt_blocked {
				self.close_open_block(&mut out);

				let finish = if prompt_blocked {
					crate::types::completions::typed::FinishReason::ContentFilter
				} else {
					to_completions::finish_with_tool_override(finish_reason_str, self.saw_tool_call)
				};
				self.pending_stop_reason = Some(crate::conversion::messages::finish_reason_to_stop_reason(
					finish,
				));
				self.flush_message_end(false, &mut out);
			}

			out
		}

		fn on_done(&mut self) -> Vec<(&'static str, messages::MessagesStreamEvent)> {
			let mut out = Vec::new();
			self.close_open_block(&mut out);
			self.flush_message_end(true, &mut out);
			out
		}
	}

	/// Translate a native Gemini `:streamGenerateContent?alt=sse` stream into Anthropic
	/// Messages-format SSE events.
	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		model: Strng,
		log: StreamingUsageGuard,
	) -> Body {
		let mut state = StreamState::new();
		// Gemini ends without [DONE]; append one to the INPUT so json_transform_multi fires
		// SseJsonEvent::Done on clean close, which lets on_done() emit message_stop.
		let b = to_completions::append_done_on_close(b.into_data_stream());
		parse::sse::json_transform_multi::<vg::GenerateContentResponse, messages::MessagesStreamEvent, _>(
			b,
			buffer_limit,
			move |ev| match ev {
				parse::sse::SseJsonEvent::Data(Ok(chunk)) => {
					// Capture before translate() sets message_started so we know whether this
					// chunk will emit a message_start (only the first chunk can).
					let is_first_chunk = !state.message_started;
					let mut events = state.translate(&chunk, &log);
					// Fill in the model if the chunk didn't carry a modelVersion.
					if state.model_version.is_empty() {
						state.model_version = model.to_string();
					}
					// Patch model in a message_start emitted before the first modelVersion arrived.
					// Only possible on the first chunk, so skip scanning on all subsequent chunks.
					if is_first_chunk {
						for (_, ev) in &mut events {
							if let messages::MessagesStreamEvent::MessageStart { message } = ev
								&& message.model.is_empty()
							{
								message.model = state.model_version.clone();
							}
						}
					}
					events
				},
				parse::sse::SseJsonEvent::Data(Err(e)) => {
					tracing::debug!("failed to parse gemini stream chunk: {e}");
					vec![]
				},
				parse::sse::SseJsonEvent::Done => state.on_done(),
			},
		)
	}
}
