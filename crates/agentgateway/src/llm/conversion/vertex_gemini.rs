use agent_core::strng;
use bytes::Bytes;

use crate::llm::types::{ResponseType, vertex_gemini as vg};
use crate::llm::{AIError, logged_response_parsing, types};

#[cfg(test)]
#[path = "vertex_gemini_tests.rs"]
mod tests;

pub mod from_completions {
	use serde::Deserialize;
	use serde_json::{Value, json};

	use super::*;
	use crate::llm::conversion::completions::parse_data_url;

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

	fn explicit_mime_hint(image_url: Option<&Value>) -> Option<String> {
		let obj = image_url?;
		let hint = ["format", "mime_type", "content_type"]
			.into_iter()
			.find_map(|k| obj.get(k).and_then(Value::as_str).filter(|h| !h.is_empty()))?;
		if hint.contains('/') {
			Some(hint.to_string())
		} else {
			mime_from_ext_token(hint).map(str::to_string)
		}
	}
	pub fn translate(
		req: &types::completions::Request,
		configured_model: Option<&str>,
	) -> Result<Vec<u8>, AIError> {
		let out = build_request(req, configured_model)?;
		serde_json::to_vec(&out).map_err(AIError::RequestMarshal)
	}

	pub(super) fn build_request(
		req: &types::completions::Request,
		configured_model: Option<&str>,
	) -> Result<vg::GenerateContentRequest, AIError> {
		let model = req
			.model
			.as_deref()
			.or(configured_model)
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

		let cached_content = req
			.rest
			.get("cachedContent")
			.or_else(|| req.rest.get("cached_content"))
			.and_then(Value::as_str)
			.map(str::to_string);

		let safety_settings = req
			.rest
			.get("safetySettings")
			.or_else(|| req.rest.get("safety_settings"))
			.and_then(|v| Vec::<vg::SafetySetting>::deserialize(v).ok())
			.unwrap_or_default();

		let labels = req.rest.get("labels").and_then(|v| v.as_object().cloned());

		let mut out = vg::GenerateContentRequest {
			contents,
			system_instruction,
			tools,
			tool_config,
			generation_config,
			safety_settings,
			cached_content,
			labels,
			rest: Value::Null,
		};

		// Invariant: cachedContent is mutually exclusive with systemInstruction, tools, and toolConfig
		if out.cached_content.is_some() {
			let dropped: Vec<&str> = [
				("systemInstruction", out.system_instruction.take().is_some()),
				("tools", !std::mem::take(&mut out.tools).is_empty()),
				("toolConfig", out.tool_config.take().is_some()),
			]
			.into_iter()
			.filter_map(|(name, present)| present.then_some(name))
			.collect();
			if !dropped.is_empty() {
				tracing::warn!(
					dropped = ?dropped,
					"cachedContent is set; dropped cache-incompatible fields"
				);
			}
		}

		Ok(out)
	}

	fn messages_to_contents(
		messages: &[types::completions::RequestMessage],
	) -> Result<(Vec<String>, Vec<vg::Content>), AIError> {
		use types::completions::Content;

		let mut id_to_name: std::collections::HashMap<String, String> = Default::default();
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
						for c in calls {
							if let (Some(id), Some(name)) = (
								c.get("id").and_then(Value::as_str),
								c.get("function")
									.and_then(|f| f.get("name"))
									.and_then(Value::as_str),
							) {
								id_to_name.insert(id.to_string(), name.to_string());
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
					let name = m
						.tool_call_id
						.as_ref()
						.and_then(|id| id_to_name.get(id))
						.cloned()
						.or_else(|| m.name.clone())
						.unwrap_or_default();
					let response = content_text(&m.content)
						.map(|t| json!({ "content": t }))
						.unwrap_or(Value::Null);
					let part = vg::Part::FunctionResponse(vg::FunctionResponsePart {
						function_response: vg::FunctionResponse {
							name,
							id: m.tool_call_id.clone(),
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

	fn image_part(image_url: Option<&Value>) -> Result<vg::Part, AIError> {
		let url = image_url
			.and_then(|u| u.get("url"))
			.and_then(Value::as_str)
			.unwrap_or_default();

		if let Some((mime, data)) = parse_data_url(url) {
			return Ok(vg::Part::InlineData(vg::InlineDataPart {
				inline_data: vg::Blob {
					mime_type: canonical_mime(mime).to_string(),
					data: data.to_string(),
					rest: Value::Null,
				},
				rest: Value::Null,
			}));
		}

		if url.starts_with("gs://") {
			// Vertex's fileData requires a mimeType for gs:// objects and won't infer one;
			// take an explicit client hint or the path extension, else reject before egress
			// rather than letting Vertex 400.
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
					rest: Value::Null,
				},
				rest: Value::Null,
			}));
		}

		// http(s) and anything else are not fetchable by Vertex.
		Err(AIError::InvalidResponse(strng::new(format!(
			"native Gemini path rejects http(s) image_url ({url}); upload to gs:// or send inline data:"
		))))
	}

	fn text_part(text: &str) -> vg::Part {
		vg::Part::Text(vg::TextPart {
			text: text.to_string(),
			thought: None,
			thought_signature: None,
			rest: Value::Null,
		})
	}

	fn is_text_part(p: &vg::Part) -> bool {
		matches!(p, vg::Part::Text(_))
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
			.unwrap_or(Value::Null);
		let id = call.get("id").and_then(Value::as_str).map(str::to_string);
		vg::Part::FunctionCall(vg::FunctionCallPart {
			function_call: vg::FunctionCall {
				name,
				id,
				args,
				rest: Value::Null,
			},
			thought: None,
			thought_signature: None,
			rest: Value::Null,
		})
	}

	/// Append `parts` as a content entry of `role`, merging into the previous entry
	/// when the role matches (Gemini requires user/model alternation).
	///
	/// For user entries, also enforces the Vertex invariant that every user turn must
	/// contain at least one text part (image-only turns are rejected otherwise).
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
			rest: Value::Null,
		});
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
				parameters: f.get("parameters").cloned(),
				rest: Value::Null,
			})
			.collect();
		if decls.is_empty() {
			Vec::new()
		} else {
			vec![vg::Tool {
				function_declarations: decls,
				rest: Value::Null,
			}]
		}
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
					rest: Value::Null,
				}
			},
			_ => return None,
		};
		Some(vg::ToolConfig {
			function_calling_config: Some(cfg),
			rest: Value::Null,
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
			rest: Value::Null,
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
				// Unwrap OpenAI's {schema, strict, name, description}; Gemini wants the bare schema.
				let schema = rf
					.get("json_schema")
					.and_then(|js| js.get("schema"))
					.cloned();
				(Some("application/json".into()), schema)
			},
			_ => (None, None),
		}
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
				rest: Value::Null,
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
				rest: Value::Null,
			})
		}
	}
}

pub mod to_completions {
	use serde_json::{Value, json};

	use super::*;

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let resp: vg::GenerateContentResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		let value = build_response_value(&resp);
		let out: types::completions::Response =
			serde_json::from_value(value).map_err(AIError::ResponseParsing)?;
		Ok(Box::new(out))
	}

	fn build_response_value(resp: &vg::GenerateContentResponse) -> Value {
		let model = resp.model_version.clone().unwrap_or_default();
		let id = resp
			.response_id
			.clone()
			.unwrap_or_else(|| format!("vertex-gemini-{}", chrono::Utc::now().timestamp_millis()));
		let created = chrono::Utc::now().timestamp();

		let choices = if resp.candidates.is_empty() {
			// Prompt-level block (promptFeedback.blockReason) with no candidate.
			let block = resp
				.prompt_feedback
				.as_ref()
				.and_then(|pf| pf.block_reason.clone());
			vec![json!({
				"index": 0,
				"message": { "role": "assistant", "content": "" },
				"finish_reason": if block.is_some() { "content_filter" } else { "stop" },
			})]
		} else {
			resp
				.candidates
				.iter()
				.enumerate()
				.map(|(i, cand)| build_choice(i as u32, cand, &id))
				.collect()
		};

		let usage = resp.usage_metadata.as_ref().map(build_usage);

		json!({
			"id": id,
			"object": "chat.completion",
			"created": created,
			"model": model,
			"choices": choices,
			"usage": usage,
		})
	}

	fn build_choice(index: u32, cand: &vg::Candidate, request_id: &str) -> Value {
		let mut content = String::new();
		let mut reasoning = String::new();
		let mut tool_calls: Vec<Value> = Vec::new();

		if let Some(c) = &cand.content {
			for part in &c.parts {
				match part {
					vg::Part::Text(t) => {
						if is_thought(t) {
							reasoning.push_str(strip_thought_prefix(&t.text));
						} else {
							content.push_str(&t.text);
						}
					},
					vg::Part::FunctionCall(fc) => {
						let idx = tool_calls.len();
						let id = fc
							.function_call
							.id
							.clone()
							.unwrap_or_else(|| format!("call_{request_id}_{idx}"));
						let args =
							serde_json::to_string(&fc.function_call.args).unwrap_or_else(|_| "{}".to_string());
						tool_calls.push(json!({
							"index": idx,
							"id": id,
							"type": "function",
							"function": { "name": fc.function_call.name, "arguments": args },
						}));
					},
					_ => {},
				}
			}
		}

		let mut finish = map_finish_reason(cand.finish_reason.as_deref());

		if finish == "stop" && !tool_calls.is_empty() {
			finish = "tool_calls";
		}

		let mut message = serde_json::Map::new();
		message.insert("role".into(), json!("assistant"));
		message.insert(
			"content".into(),
			if content.is_empty() && !tool_calls.is_empty() {
				Value::Null
			} else {
				json!(content)
			},
		);
		if !reasoning.is_empty() {
			message.insert("reasoning_content".into(), json!(reasoning));
		}
		if !tool_calls.is_empty() {
			message.insert("tool_calls".into(), json!(tool_calls));
		}

		json!({
			"index": index,
			"message": Value::Object(message),
			"finish_reason": finish,
		})
	}

	fn is_thought(t: &vg::TextPart) -> bool {
		t.thought == Some(true) || starts_with_thought_prefix(&t.text)
	}

	fn starts_with_thought_prefix(text: &str) -> bool {
		text
			.trim_start()
			.to_ascii_uppercase()
			.starts_with("THOUGHT:")
	}

	fn strip_thought_prefix(text: &str) -> &str {
		if starts_with_thought_prefix(text) {
			let trimmed = text.trim_start();
			trimmed[8..].trim_start()
		} else {
			text
		}
	}

	// Unknown Gemini finishReason values map to "stop".
	pub(crate) fn map_finish_reason(reason: Option<&str>) -> &'static str {
		match reason {
			Some("MAX_TOKENS") => "length",
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
			) => "content_filter",
			// STOP, MALFORMED_FUNCTION_CALL, IMAGE_OTHER, NO_IMAGE, OTHER,
			// FINISH_REASON_UNSPECIFIED, None, and any future value.
			_ => "stop",
		}
	}

	fn build_usage(um: &vg::UsageMetadata) -> Value {
		let prompt = um.prompt_token_count.unwrap_or(0);
		let completion = um.candidates_token_count.unwrap_or(0);
		let total = um.total_token_count.unwrap_or(prompt + completion);

		let mut usage = serde_json::Map::new();
		usage.insert("prompt_tokens".into(), json!(prompt));
		usage.insert("completion_tokens".into(), json!(completion));
		usage.insert("total_tokens".into(), json!(total));
		if let Some(cached) = um.cached_content_token_count {
			usage.insert(
				"prompt_tokens_details".into(),
				json!({ "cached_tokens": cached }),
			);
		}
		if let Some(reasoning) = um.thoughts_token_count {
			usage.insert(
				"completion_tokens_details".into(),
				json!({ "reasoning_tokens": reasoning }),
			);
		}
		Value::Object(usage)
	}
}
