use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;

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

/// Messages and Responses clients get the signature in their own opaque reasoning field instead,
/// keyed by call id because Gemini signs only the first of parallel calls
const SIGNATURE_CARRIER_PREFIX: &str = "agentgateway.gemini.thought-signature.v1:";

#[derive(serde::Serialize, serde::Deserialize)]
struct ThoughtSignature<S = String> {
	call_id: S,
	signature: S,
}

impl<'a> ThoughtSignature<&'a str> {
	fn encode(call_id: &'a str, signature: &'a str) -> String {
		let json = serde_json::to_string(&Self { call_id, signature })
			.expect("serializing two strings cannot fail");
		format!("{SIGNATURE_CARRIER_PREFIX}{json}")
	}
}

impl ThoughtSignature {
	fn decode(carrier: &str) -> Option<Self> {
		serde_json::from_str(carrier.strip_prefix(SIGNATURE_CARRIER_PREFIX)?).ok()
	}
}

/// Drop our carriers before a request goes to another provider, which can't use them
pub fn strip_signature_carriers(req: &mut types::ChatRequest) {
	use serde_json::Value;
	match req {
		types::ChatRequest::Messages(req) => strip_message_carriers(&mut req.messages),
		types::ChatRequest::Responses(req) => {
			if let types::responses::RequestInput::Items(items) = &mut req.input {
				items.retain_mut(|types::responses::RawInputItem(item)| {
					if item["type"] != "reasoning" || !is_carrier(&item["encrypted_content"]) {
						return true;
					}
					let is_empty = |key: &str| match &item[key] {
						Value::Null => true,
						Value::Array(parts) => parts.is_empty(),
						_ => false,
					};
					if is_empty("summary") && is_empty("content") {
						return false;
					}
					item
						.as_object_mut()
						.expect("a reasoning item is an object")
						.remove("encrypted_content");
					true
				});
			}
		},
		types::ChatRequest::Completions(_) | types::ChatRequest::Gemini(_) => {},
	}
}

/// The Messages half of [`strip_signature_carriers`], shared with count_tokens
pub fn strip_message_carriers(messages: &mut [types::messages::RequestMessage]) {
	for message in messages {
		if let Some(types::messages::ContentBlock::Array(parts)) = &mut message.content {
			parts.retain(|part| {
				!matches!(part, types::messages::ContentPart::Unknown(block)
					if block["type"] == "thinking" && is_carrier(&block["signature"]))
			});
		}
	}
}

fn is_carrier(value: &serde_json::Value) -> bool {
	value.as_str().and_then(ThoughtSignature::decode).is_some()
}

fn client_tool_id(id: String) -> (String, Option<String>) {
	match split_tool_call_id(&id) {
		(base, Some(signature)) => (
			base.to_string(),
			Some(ThoughtSignature::encode(base, signature)),
		),
		_ => (id, None),
	}
}

/// Passthrough for native Gemini inbound: forward the `:streamGenerateContent?alt=sse` SSE
/// bytes untouched while extracting usage for telemetry. Gemini attaches cumulative
/// `usageMetadata` to chunks with the full totals on the final event, so updating on every
/// chunk leaves the last event's counts in the log even on early client disconnect.
pub fn passthrough_stream(
	b: agent_http::Body,
	buffer_limit: usize,
	log: crate::StreamingUsageGuard,
	log_content: crate::LogContentFields,
) -> agent_http::Body {
	use std::time::Instant;
	let mut saw_token = false;
	crate::parse::sse::json_passthrough::<vg::GenerateContentResponse>(b, buffer_limit, move |f| {
		// Gemini never sends a [DONE] sentinel, so f(None) does not fire; all bookkeeping
		// happens per-chunk and the guard flushes on drop.
		let Some(Ok(chunk)) = f else {
			return;
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
			let (prompt, completion, total) = um.counts();
			log.update(|r| {
				r.response.input_tokens = Some(prompt);
				r.response.output_tokens = Some(completion);
				r.response.total_tokens = Some(total);
				r.response.cached_input_tokens = um.cached_content_token_count;
				r.response.reasoning_tokens = um.thoughts_token_count;
				r.response.service_tier = um
					.service_tier
					.as_deref()
					.or(um.traffic_type.as_deref())
					.map(strng::new);
			});
		}
		if log_content.completion {
			let text: String = chunk
				.candidates
				.first()
				.and_then(|c| c.content.as_ref())
				.map(|c| {
					c.parts
						.iter()
						.filter_map(|p| match p {
							vg::Part::Text(t) if t.thought != Some(true) => Some(t.text.as_str()),
							_ => None,
						})
						.collect()
				})
				.unwrap_or_default();
			if !text.is_empty() {
				log.update(|r| {
					let completion = r
						.response
						.completion
						.get_or_insert_with(|| vec![String::new()]);
					if let Some(first) = completion.first_mut() {
						first.push_str(&text);
					}
				});
			}
		}
	})
}

pub mod from_completions {
	use serde::Deserialize;
	use serde_json::{Value, json};

	use super::*;
	use crate::Provider as _;
	use crate::conversion::completions::parse_data_url;
	use crate::model_catalog::{Catalog, nearest_effort};
	use crate::types::completions::typed::ReasoningEffort;

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
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Result<Vec<u8>, AIError> {
		let out = build_request(req, is_vertex, catalog)?;
		serde_json::to_vec(&out).map_err(AIError::RequestMarshal)
	}

	pub(super) fn build_request(
		req: &types::completions::Request,
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Result<vg::GenerateContentRequest, AIError> {
		let model = req
			.model
			.as_deref()
			.ok_or_else(|| AIError::MissingField("model not specified".into()))?;

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
			parts: vec![text_part(system_text.join("\n"))],
			rest: Value::Null,
		});

		let tools = build_tools(req);
		let tool_config = build_tool_config(req);
		let generation_config = build_generation_config(req, model, is_vertex, catalog);

		let cached_content = req
			.rest
			.get("cachedContent")
			.or_else(|| req.rest.get("cached_content"))
			.and_then(Value::as_str)
			.map(str::to_string);

		let safety_settings = match req
			.rest
			.get("safetySettings")
			.or_else(|| req.rest.get("safety_settings"))
		{
			Some(v) => Vec::<vg::SafetySetting>::deserialize(v).unwrap_or_else(|e| {
				tracing::warn!(error = %e, "ignoring malformed safetySettings");
				Vec::new()
			}),
			None => Vec::new(),
		};

		let labels = req.rest.get("labels").and_then(|v| v.as_object().cloned());

		let (system_instruction, tools, tool_config) = if cached_content.is_some() {
			let dropped: Vec<&str> = [
				("systemInstruction", system_instruction.is_some()),
				("tools", !tools.is_empty()),
				("toolConfig", tool_config.is_some()),
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
			(None, Vec::new(), None)
		} else {
			(system_instruction, tools, tool_config)
		};

		Ok(vg::GenerateContentRequest {
			contents,
			system_instruction,
			tools,
			tool_config,
			generation_config,
			safety_settings,
			cached_content,
			labels,
			rest: Default::default(),
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
		// Vertex rejects a reminder between the results of a parallel call
		let mut reminders: Vec<vg::Part> = Vec::new();
		for m in messages {
			let role = m.role.as_str();
			if !matches!(role, "system" | "developer" | "tool" | "function") {
				push_content(&mut contents, "user", std::mem::take(&mut reminders));
			}
			match role {
				// In systemInstruction, a later system message would change the cached prefix every turn
				"system" | "developer" if contents.is_empty() => {
					system_text.extend(content_text(&m.content).filter(|t| !t.is_empty()));
				},
				"system" | "developer" => reminders.extend(
					content_text(&m.content)
						.filter(|t| !t.is_empty())
						.map(|t| text_part(format!("<system-reminder>\n{t}\n</system-reminder>"))),
				),
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
		push_content(&mut contents, "user", reminders);

		// Vertex rejects `id` and correlates functionResponse to functionCall positionally, so each
		// response group must follow the assistant's tool_calls order even when a client returns the
		// `tool` messages out of order. Reorder the functionResponse parts, then drop the now-unused
		// correlation id.
		for content in &mut contents {
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
						"file" => {
							parts.push(file_part(p.rest.get("file"))?);
						},
						_ => {},
					}
				}
			},
			_ => {},
		}
		Ok(parts)
	}

	fn inline_data_part(mime: &str, data: &str) -> vg::Part {
		vg::Part::InlineData(vg::InlineDataPart {
			inline_data: vg::Blob {
				mime_type: canonical_mime(mime).to_string(),
				data: data.to_string(),
				rest: Value::Null,
			},
			rest: Value::Null,
		})
	}

	fn file_data_part(mime: &str, uri: &str) -> vg::Part {
		vg::Part::FileData(vg::FileDataPart {
			file_data: vg::FileData {
				mime_type: Some(canonical_mime(mime).to_string()),
				file_uri: uri.to_string(),
				rest: Value::Null,
			},
			rest: Value::Null,
		})
	}

	fn image_part(image_url: Option<&Value>) -> Result<vg::Part, AIError> {
		let url = image_url
			.and_then(|u| u.get("url"))
			.and_then(Value::as_str)
			.unwrap_or_default();

		if let Some((mime, data)) = parse_data_url(url) {
			return Ok(inline_data_part(mime, data));
		}

		if url.starts_with("gs://") {
			// Vertex's fileData requires a mimeType for gs:// objects and won't infer one
			let Some(mime) =
				explicit_mime_hint(image_url).or_else(|| mime_from_extension(url).map(str::to_string))
			else {
				return Err(AIError::UnsupportedConversion(strng::new(format!(
					"gs:// image_url ({url}) has no recognised extension or MIME hint; pass image_url.format (or mime_type/content_type), or use an object with a known extension"
				))));
			};
			return Ok(file_data_part(&mime, url));
		}

		// http(s) and anything else are not fetchable by Vertex.
		Err(AIError::UnsupportedConversion(strng::new(format!(
			"native Gemini path rejects http(s) image_url ({url}); upload to gs:// or send inline data:"
		))))
	}

	/// Convert an OpenAI `file` content part into a Gemini part.
	///
	/// Inline `data:` payloads and raw base64 become `inlineData`; `gs://` and http(s) URIs in
	/// `file_data` or `file_id` become `fileData`, which Vertex fetches itself.
	fn file_part(file: Option<&Value>) -> Result<vg::Part, AIError> {
		let field = |k: &str| {
			file
				.and_then(|f| f.get(k))
				.and_then(Value::as_str)
				.unwrap_or_default()
		};
		let file_data = field("file_data");
		let file_id = field("file_id");
		let hinted_mime = || {
			explicit_mime_hint(file)
				.or_else(|| mime_from_extension(field("filename")).map(str::to_string))
		};

		if let Some((mime, data)) = parse_data_url(file_data) {
			if !mime.is_empty() {
				return Ok(inline_data_part(mime, data));
			}
			// RFC 2397 allows an absent media type; Vertex rejects an empty mimeType.
			let Some(mime) = hinted_mime() else {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"data: file_data has no media type; pass file.filename with a known extension (or mime_type/content_type)"
				)));
			};
			return Ok(inline_data_part(&mime, data));
		}

		if let Some(uri) = [file_data, file_id]
			.into_iter()
			.find(|u| u.starts_with("gs://") || u.starts_with("https://") || u.starts_with("http://"))
		{
			// Drop the query and fragment from http(s) URIs: signed URLs put the extension before
			// the query, and the query holds the signature, which must not appear in errors.
			// gs:// object names may contain '?' and '#' literally.
			let path = if uri.starts_with("gs://") {
				uri
			} else {
				uri.split(['?', '#']).next().unwrap_or(uri)
			};
			let Some(mime) = hinted_mime().or_else(|| mime_from_extension(path).map(str::to_string))
			else {
				return Err(AIError::UnsupportedConversion(strng::new(format!(
					"file ({path}) has no recognised extension or MIME hint; pass file.filename (or mime_type/content_type), or use a URI with a known extension"
				))));
			};
			return Ok(file_data_part(&mime, uri));
		}

		// Raw base64 without a data URL wrapper, as bedrock.rs also accepts; mime from filename.
		// A malformed `data:` value must not reach here, or its header becomes payload.
		if !file_data.is_empty() && !file_data.contains("://") && !file_data.starts_with("data:") {
			let Some(mime) = hinted_mime() else {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"raw base64 file_data has no MIME source; pass file.filename with a known extension (or mime_type/content_type), or wrap it in a data: URI"
				)));
			};
			return Ok(inline_data_part(&mime, file_data));
		}

		if !file_id.is_empty() {
			return Err(AIError::UnsupportedConversion(strng::new(format!(
				"native Gemini path cannot resolve OpenAI file_id ({file_id}); Vertex has no OpenAI Files store. Send file.file_data as an inline data: URI, or put a gs:// or http(s) URI in file.file_data or file.file_id"
			))));
		}

		Err(AIError::UnsupportedConversion(strng::new(
			"file content part carries no payload; send inline bytes in file.file_data, or a gs:// or http(s) URI in file.file_data or file.file_id",
		)))
	}

	fn text_part(text: impl Into<String>) -> vg::Part {
		vg::Part::Text(vg::TextPart {
			text: text.into(),
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

	/// Append `parts` as a content entry of `role`, merging compatible parts into the
	/// previous entry when the role matches (Gemini requires user/model alternation).
	///
	/// Function responses must remain in their own user entry: Gemini 3 rejects a
	/// functionResponse with sibling parts. Other user entries retain a text filler when
	/// necessary (for example, image-only turns).
	fn push_content(contents: &mut Vec<vg::Content>, role: &str, mut parts: Vec<vg::Part>) {
		if parts.is_empty() {
			return;
		}
		let has_function_response = parts
			.iter()
			.any(|p| matches!(p, vg::Part::FunctionResponse(_)));
		if let Some(last) = contents.last_mut()
			&& last.role.as_deref() == Some(role)
			&& last
				.parts
				.iter()
				.any(|p| matches!(p, vg::Part::FunctionResponse(_)))
				== has_function_response
		{
			if role == "user"
				&& !has_function_response
				&& !last.parts.iter().any(is_text_part)
				&& !parts.iter().any(is_text_part)
			{
				parts.push(text_part(" "));
			}
			last.parts.extend(parts);
			return;
		}
		if role == "user" && !has_function_response && !parts.iter().any(is_text_part) {
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
				parameters: f
					.get("parameters")
					.map(|s| normalize_gemini_schema(s, false)),
				rest: Default::default(),
			})
			.collect();
		if decls.is_empty() {
			Vec::new()
		} else {
			vec![vg::Tool {
				function_declarations: decls,
				rest: Default::default(),
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
					rest: Default::default(),
				}
			},
			_ => return None,
		};
		Some(vg::ToolConfig {
			function_calling_config: Some(cfg),
			rest: Default::default(),
		})
	}

	fn build_generation_config(
		req: &types::completions::Request,
		model: &str,
		is_vertex: bool,
		catalog: Catalog<'_>,
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

		let (response_mime_type, response_schema) = response_format(req, is_vertex);
		let thinking_config = thinking_config(req, model, is_vertex, catalog);

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
			rest: Default::default(),
		};

		if cfg == vg::GenerationConfig::default() {
			None
		} else {
			Some(cfg)
		}
	}

	fn response_format(
		req: &types::completions::Request,
		is_vertex: bool,
	) -> (Option<String>, Option<Value>) {
		let Some(rf) = req.rest.get("response_format") else {
			return (None, None);
		};
		match rf.get("type").and_then(Value::as_str) {
			Some("json_object") => (Some("application/json".into()), None),
			Some("json_schema") => {
				// Unwrap OpenAI's {schema, strict, name, description} and normalize the bare schema.
				// Vertex AI accepts additionalProperties in responseSchema; the Gemini API does not.
				let schema = rf
					.get("json_schema")
					.and_then(|js| js.get("schema"))
					.map(|s| normalize_gemini_schema(s, is_vertex));
				(Some("application/json".into()), schema)
			},
			_ => (None, None),
		}
	}

	// Gemini's responseSchema / functionDeclarations[].parameters accept only a subset of JSON Schema.
	// The normalization below is ported from litellm's `_build_vertex_schema` (BerriAI/litellm, MIT).
	//
	// Authoritative field list: google/ai/generativelanguage/v1beta/content.proto — Schema message.
	// Cross-checked against litellm/types/llms/vertex_ai.py Schema TypedDict (both MIT-licensed).

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
		"minItems",
		"maxItems",
		"minProperties",
		"maxProperties",
		"example",
		"additionalProperties",
		"propertyOrdering",
	];

	/// Normalize an OpenAI JSON Schema into the Gemini/Vertex subset.
	///
	/// Set `preserve_ap` to `true` for Vertex AI `responseSchema` — Vertex accepts
	/// `additionalProperties`. Set it to `false` for the Gemini API `responseSchema` and for
	/// `functionDeclarations[].parameters` (both reject the key).
	pub(super) fn normalize_gemini_schema(schema: &Value, preserve_ap: bool) -> Value {
		let mut out = schema.clone();
		let defs = take_defs(&mut out);
		inline_refs(&mut out, &defs, &mut Vec::new());
		clean_schema_node(&mut out, preserve_ap);
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

	/// Visit each direct child schema (`items`, `properties`, `anyOf`, `allOf`, the schema form of
	/// `additionalProperties`), shared by both passes so they recurse the same keywords.
	/// (`clean_schema_node` flattens `allOf` first, so it is a no-op here.)
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
		for key in ["anyOf", "oneOf", "allOf"] {
			if let Some(Value::Array(arr)) = map.get_mut(key) {
				for v in arr.iter_mut() {
					f(v);
				}
			}
		}
		// Boolean forms carry no subschema. An empty object means "anything goes" and must stay
		// empty: recursing would let the typeless default rewrite it into {"type":"object"}.
		if let Some(ap) = map.get_mut("additionalProperties")
			&& ap.as_object().is_some_and(|o| !o.is_empty())
		{
			f(ap);
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

		// oneOf -> anyOf. Gemini's Schema proto has any_of only and Vertex silently ignores oneOf, which
		// leaves the node typeless and defaults it to an empty object. "Exactly one" and "at least one"
		// only differ for overlapping members, and anyOf is the closest Gemini can express. A node that
		// already carries an anyOf gets the members appended (best-effort, as with allOf).
		if let Some(Value::Array(members)) = map.remove("oneOf") {
			changed = true;
			match map.get_mut("anyOf") {
				Some(Value::Array(existing)) => existing.extend(members),
				_ => {
					map.insert("anyOf".to_string(), Value::Array(members));
				},
			}
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
	fn clean_schema_node(node: &mut Value, preserve_ap: bool) {
		let map = match node {
			Value::Object(map) => map,
			Value::Array(arr) => {
				for v in arr.iter_mut() {
					clean_schema_node(v, preserve_ap);
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

		for_each_child_schema(map, |v| clean_schema_node(v, preserve_ap));
		convert_exclusive_bounds(map);
		map.retain(|k, _| {
			ALLOWED_SCHEMA_FIELDS.contains(&k.as_str()) && (preserve_ap || k != "additionalProperties")
		});
	}

	fn convert_exclusive_bounds(map: &mut serde_json::Map<String, Value>) {
		// Gemini rejects exclusive bounds
		use std::cmp::Ordering;
		for (exclusive, inclusive, tighter) in [
			("exclusiveMinimum", "minimum", Ordering::Greater),
			("exclusiveMaximum", "maximum", Ordering::Less),
		] {
			let Some(bound) = map.remove(exclusive).filter(Value::is_number) else {
				continue;
			};
			let exclusive_value = bound.as_f64().expect("a JSON number converts to f64");
			let is_tighter = match map.get(inclusive) {
				None => true,
				Some(current) => current.as_f64().is_some_and(|inclusive_value| {
					exclusive_value.partial_cmp(&inclusive_value) == Some(tighter)
				}),
			};
			if is_tighter {
				map.insert(inclusive.to_string(), bound);
			}
		}
	}

	fn thinking_config(
		req: &types::completions::Request,
		model: &str,
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Option<vg::ThinkingConfig> {
		if let Some(tc) = req
			.rest
			.get("thinking_config")
			.or_else(|| req.rest.get("thinkingConfig"))
		{
			return vg::ThinkingConfig::deserialize(tc).ok();
		}

		let effort = req.reasoning_effort.as_ref()?;
		let level = select_thinking_level(effort, model, is_vertex, catalog);
		if let Some(level) = level {
			if *effort == ReasoningEffort::None {
				tracing::debug!(
					model,
					level,
					"thinking can't be turned off; using the lowest level"
				);
			}
			return Some(vg::ThinkingConfig {
				thinking_level: Some(level.into()),
				thinking_budget: None,
				include_thoughts: Some(true),
				rest: Default::default(),
			});
		}
		if uses_thinking_levels(model) {
			// Models using levels must not fall back to a thinking budget
			return None;
		}

		if *effort == ReasoningEffort::None {
			return minimum_thinking_config(model);
		}
		let budget = crate::types::thinking_budget_for_reasoning_effort(effort)? as i32;
		Some(vg::ThinkingConfig {
			thinking_level: None,
			thinking_budget: Some(budget),
			include_thoughts: Some(true),
			rest: Default::default(),
		})
	}

	fn select_thinking_level(
		effort: &ReasoningEffort,
		model: &str,
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Option<&'static str> {
		let provider = if is_vertex {
			crate::vertex::Provider::NAME
		} else {
			crate::gemini::Provider::NAME
		};
		let level_in = |accepted: &[ReasoningEffort]| {
			let levels: Vec<_> = accepted
				.iter()
				.filter(|e| gemini_level(e).is_some())
				.cloned()
				.collect();
			nearest_effort(effort, &levels).and_then(|e| gemini_level(&e))
		};
		match catalog.and_then(|c| c.reasoning_efforts(&provider, model)) {
			Some(accepted) => level_in(&accepted),
			None if uses_thinking_levels(model) => level_in(&[
				ReasoningEffort::Low,
				ReasoningEffort::Medium,
				ReasoningEffort::High,
			]),
			None => None,
		}
	}

	fn uses_thinking_levels(model: &str) -> bool {
		model.contains("gemini-3") || model.contains("gemini-4")
	}

	/// Gemini 2.5 Pro requires its minimum budget; Flash and Flash-Lite can use zero
	fn minimum_thinking_config(model: &str) -> Option<vg::ThinkingConfig> {
		let (budget, include_thoughts) = match gemini_2_5_thinking_variant(model)? {
			"pro" => {
				tracing::debug!(
					model,
					"thinking can't be turned off; using the minimum budget"
				);
				(128, Some(true))
			},
			_ => (0, None),
		};
		Some(vg::ThinkingConfig {
			thinking_level: None,
			thinking_budget: Some(budget),
			include_thoughts,
			rest: Default::default(),
		})
	}

	fn gemini_2_5_thinking_variant(model: &str) -> Option<&'static str> {
		let (_, rest) = model.split_once("gemini-2.5-")?;
		let variant = ["flash-lite", "flash", "pro"]
			.into_iter()
			.find(|v| rest.starts_with(v))?;
		rest[variant.len()..]
			.split('-')
			.filter(|part| !part.is_empty())
			.all(|part| part == "preview" || part.bytes().all(|b| b.is_ascii_digit()))
			.then_some(variant)
	}

	fn gemini_level(effort: &ReasoningEffort) -> Option<&'static str> {
		match effort {
			ReasoningEffort::Minimal => Some("minimal"),
			ReasoningEffort::Low => Some("low"),
			ReasoningEffort::Medium => Some("medium"),
			ReasoningEffort::High => Some("high"),
			ReasoningEffort::None | ReasoningEffort::Xhigh | ReasoningEffort::Max => None,
		}
	}
}

pub mod from_messages {
	use std::collections::HashMap;

	use agent_core::prelude::Strng;
	use agent_http::Body;
	use bytes::Bytes;

	use crate::conversion::completions::from_messages as completions;
	use crate::model_catalog::Catalog;
	use crate::types::ResponseType;
	use crate::types::messages::typed as messages;
	use crate::{AIError, InputFormat, LogContentFields, StreamingUsageGuard, json, types};

	pub fn translate(
		req: &types::messages::Request,
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Result<Vec<u8>, AIError> {
		let mut req = json::convert::<_, messages::Request>(req)
			.map_err(|err| AIError::RequestParsing(InputFormat::Messages, err))?;
		req
			.messages
			.iter_mut()
			.filter(|m| m.role == messages::Role::Assistant)
			.for_each(|m| restore_signatures(&mut m.content));
		let req = completions::translate_internal(req);
		super::from_completions::translate(&super::loose(&req)?, is_vertex, catalog)
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		model: Strng,
		log: StreamingUsageGuard,
		log_content: LogContentFields,
	) -> Body {
		let b = super::to_completions::translate_stream(b, buffer_limit, model, log, log_content);
		completions::translate_stream_with_tool_id(
			b,
			buffer_limit,
			StreamingUsageGuard::default(),
			LogContentFields::default(),
			super::client_tool_id,
		)
	}

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let (reply, provider_usage) = super::to_completions::translate(bytes)?;
		let mut response = completions::translate_response_internal(reply)?;
		response.content = move_signatures(response.content);
		Ok(Box::new(crate::conversion::ResponseWithProviderUsage {
			response,
			provider_usage,
		}))
	}

	fn move_signatures(content: Vec<messages::ContentBlock>) -> Vec<messages::ContentBlock> {
		let mut out = Vec::with_capacity(content.len() + 1);
		for mut block in content {
			if let messages::ContentBlock::ToolUse { id, .. } = &mut block {
				let (client_id, carrier) = super::client_tool_id(std::mem::take(id));
				*id = client_id;
				out.extend(carrier.map(|signature| messages::ContentBlock::Thinking {
					thinking: String::new(),
					signature,
				}));
			}
			out.push(block);
		}
		out
	}

	pub fn restore_signatures(content: &mut Vec<messages::ContentBlock>) {
		let mut signatures = HashMap::new();
		content.retain(|block| match block {
			messages::ContentBlock::Thinking { signature, .. } => {
				match super::ThoughtSignature::decode(signature) {
					Some(s) => {
						signatures.insert(s.call_id, s.signature);
						false
					},
					None => true,
				}
			},
			_ => true,
		});
		for block in content {
			if let messages::ContentBlock::ToolUse { id, .. } = block
				&& let Some(signature) = signatures.remove(id.as_str())
			{
				*id = super::join_tool_call_id(std::mem::take(id), Some(&signature));
			}
		}
	}
}

pub mod from_responses {
	use std::collections::HashMap;
	use std::sync::Arc;

	use agent_core::prelude::Strng;
	use agent_http::Body;
	use bytes::Bytes;

	use crate::conversion::namespace_tools::NamespaceToolMap;
	use crate::conversion::openai_compat as compat;
	use crate::model_catalog::Catalog;
	use crate::types::ResponseType;
	use crate::types::responses::typed as responses;
	use crate::{AIError, InputFormat, LogContentFields, StreamingUsageGuard, json, types};

	pub struct TranslatedRequest {
		pub body: Vec<u8>,
		pub namespaces: NamespaceToolMap,
	}

	pub fn translate(
		req: &types::responses::Request,
		is_vertex: bool,
		catalog: Catalog<'_>,
	) -> Result<TranslatedRequest, AIError> {
		let mut req = json::convert::<_, responses::CreateResponse>(req)
			.map_err(|err| AIError::RequestParsing(InputFormat::Responses, err))?;
		if let responses::InputParam::Items(items) = &mut req.input {
			restore_signatures(items);
		}
		let completions = compat::from_responses::translate_typed(req)?;
		let body =
			super::from_completions::translate(&super::loose(&completions.request)?, is_vertex, catalog)?;
		Ok(TranslatedRequest {
			body,
			namespaces: completions.namespaces,
		})
	}

	pub fn translate_stream(
		b: Body,
		buffer_limit: usize,
		model: Strng,
		log: StreamingUsageGuard,
		log_content: LogContentFields,
		namespaces: Option<Arc<NamespaceToolMap>>,
	) -> Body {
		let b = super::to_completions::translate_stream(b, buffer_limit, model, log, log_content);
		compat::to_responses::translate_stream_with_tool_id(
			b,
			buffer_limit,
			StreamingUsageGuard::default(),
			LogContentFields::default(),
			namespaces,
			super::client_tool_id,
		)
	}

	pub fn translate_response(
		bytes: &Bytes,
		model: &str,
		namespaces: Option<&NamespaceToolMap>,
	) -> Result<Box<dyn ResponseType>, AIError> {
		let (reply, provider_usage) = super::to_completions::translate(bytes)?;
		let response =
			compat::to_responses::translate_completion(reply, model, namespaces, super::client_tool_id)?;
		Ok(Box::new(crate::conversion::ResponseWithProviderUsage {
			response,
			provider_usage,
		}))
	}

	/// A turn is a run of reasoning items and calls; a carrier only applies to a call in its turn
	pub fn restore_signatures(items: &mut Vec<responses::InputItem>) {
		use responses::{InputItem, Item};
		let mut signatures = HashMap::new();
		items.retain_mut(|item| match item {
			InputItem::Item(Item::Reasoning(reasoning)) => {
				match reasoning
					.encrypted_content
					.as_deref()
					.and_then(super::ThoughtSignature::decode)
				{
					Some(s) => {
						signatures.insert(s.call_id, s.signature);
						false
					},
					None => true,
				}
			},
			InputItem::Item(Item::FunctionCall(call)) => {
				if let Some(signature) = signatures.remove(&call.call_id) {
					call.call_id =
						super::join_tool_call_id(std::mem::take(&mut call.call_id), Some(&signature));
				}
				true
			},
			_ => {
				signatures.clear();
				true
			},
		});
	}
}

fn loose(req: &types::completions::typed::Request) -> Result<types::completions::Request, AIError> {
	crate::json::convert(req).map_err(AIError::RequestMarshal)
}

pub mod to_completions {
	use std::collections::HashMap;
	use std::sync::Arc;
	use std::time::Instant;

	use agent_http::Body;
	use serde_json::Value;

	use super::*;
	use crate::types::completions::typed as completions;
	use crate::{StreamingUsageGuard, json, parse};

	type LoggedToolCall = (Option<String>, Option<String>, String);
	type LoggedToolCalls = HashMap<u32, LoggedToolCall>;

	pub fn translate_response(bytes: &Bytes) -> Result<Box<dyn ResponseType>, AIError> {
		let (response, provider_usage) = translate(bytes)?;
		let response = json::convert::<_, types::completions::Response>(&response)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(super::super::ResponseWithProviderUsage {
			response,
			provider_usage,
		}))
	}

	/// The Chat Completions reply, and the accounting taken from Gemini's own usage
	pub(super) fn translate(
		bytes: &Bytes,
	) -> Result<(completions::Response, Option<super::super::ProviderUsage>), AIError> {
		let resp: vg::GenerateContentResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		Ok((
			build_response(&resp),
			resp.usage_metadata.as_ref().map(provider_usage),
		))
	}

	fn provider_usage(um: &vg::UsageMetadata) -> super::super::ProviderUsage {
		let (input_tokens, _, total_tokens) = um.counts();
		super::super::ProviderUsage {
			input_tokens,
			total_tokens,
			reasoning_tokens: um.thoughts_token_count,
			cached_input_tokens: um.cached_content_token_count,
			service_tier: um
				.service_tier
				.as_deref()
				.or(um.traffic_type.as_deref())
				.map(strng::new),
			..Default::default()
		}
	}

	#[derive(Default)]
	struct DecodedParts<'a> {
		content: String,
		reasoning: String,
		calls: Vec<DecodedCall<'a>>,
	}

	/// Borrows the function-call fields from the source content; the callers own the single
	/// `String`/`Value` allocation the typed message requires, so the decode itself clones nothing.
	struct DecodedCall<'a> {
		id: Option<&'a str>,
		name: &'a str,
		args: &'a Value,
		thought_signature: Option<&'a str>,
	}

	fn decode_parts<'a>(content: Option<&'a vg::Content>) -> DecodedParts<'a> {
		let mut out = DecodedParts::default();
		let Some(content) = content else {
			return out;
		};
		for part in &content.parts {
			match part {
				vg::Part::Text(t) if t.thought == Some(true) => out.reasoning.push_str(&t.text),
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

	fn encode_args(args: &Value) -> String {
		serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
	}

	fn tool_call_id(native: Option<&str>, seed: &str, index: u32) -> String {
		native
			.map(str::to_string)
			.unwrap_or_else(|| format!("call_{seed}_{index}"))
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
				.filter(|pf| pf.block_reason.is_some());
			let finish = if blocked.is_some() {
				completions::FinishReason::ContentFilter
			} else {
				completions::FinishReason::Stop
			};
			let mut message = completions::ResponseMessage {
				content: Some(String::new()),
				..completions::ResponseMessage::assistant()
			};
			message.refusal = blocked.map(prompt_block_message);
			vec![completions::ChatChoice {
				rest: Default::default(),
				index: 0,
				message,
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
			service_tier: resp.usage_metadata.as_ref().and_then(openai_service_tier),
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
		// The gateway answers a failed reply with `failure`'s error, so its finish reason isn't seen
		let finish = finish_reason(
			cand.finish_reason.as_deref().unwrap_or("STOP"),
			has_tool_calls,
		)
		.unwrap_or(completions::FinishReason::Stop);
		let content = if decoded.content.is_empty() && (has_tool_calls || !decoded.reasoning.is_empty())
		{
			None
		} else {
			Some(decoded.content)
		};
		let reasoning = (!decoded.reasoning.is_empty()).then_some(decoded.reasoning);
		let tool_calls = has_tool_calls.then_some(tool_calls);

		let mut message = completions::ResponseMessage {
			content,
			tool_calls,
			reasoning_content: reasoning,
			..completions::ResponseMessage::assistant()
		};
		if finish == completions::FinishReason::ContentFilter {
			message.refusal = Some(reply_block_message(cand));
		}
		completions::ChatChoice {
			rest: Default::default(),
			index,
			message,
			finish_reason: Some(finish),
			logprobs: None,
		}
	}

	enum Finish {
		Reason(completions::FinishReason),
		/// Gemini produced no usable reply; the client gets an error with this HTTP status and
		/// Google status instead
		Failed(::http::StatusCode, &'static str),
		Unknown,
	}

	fn classify_finish_reason(reason: &str) -> Finish {
		use ::http::StatusCode;
		use completions::FinishReason;
		match reason {
			"STOP" => Finish::Reason(FinishReason::Stop),
			"MAX_TOKENS" | "CONTINUATION" => Finish::Reason(FinishReason::Length),
			"SAFETY"
			| "RECITATION"
			| "LANGUAGE"
			| "BLOCKLIST"
			| "PROHIBITED_CONTENT"
			| "SPII"
			| "IMAGE_SAFETY"
			| "IMAGE_PROHIBITED_CONTENT"
			| "IMAGE_RECITATION"
			| "MODEL_ARMOR"
			| "ESCALATION" => Finish::Reason(FinishReason::ContentFilter),
			"MISSING_THOUGHT_SIGNATURE" => Finish::Failed(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
			"PUP_LIMITED_DISABLED" => Finish::Failed(StatusCode::FORBIDDEN, "PERMISSION_DENIED"),
			"MALFORMED_FUNCTION_CALL"
			| "UNEXPECTED_TOOL_CALL"
			| "TOO_MANY_TOOL_CALLS"
			| "MALFORMED_RESPONSE"
			| "OTHER"
			| "NO_IMAGE"
			| "IMAGE_OTHER" => Finish::Failed(StatusCode::BAD_GATEWAY, "INTERNAL"),
			_ => Finish::Unknown,
		}
	}

	fn finish_reason(reason: &str, saw_tool_call: bool) -> Option<completions::FinishReason> {
		match classify_finish_reason(reason) {
			Finish::Reason(completions::FinishReason::Stop) if saw_tool_call => {
				Some(completions::FinishReason::ToolCalls)
			},
			Finish::Reason(reason) => Some(reason),
			Finish::Failed(..) | Finish::Unknown => None,
		}
	}

	/// The error to answer with when Gemini ended its reply with a failure: its HTTP status and a
	/// Google error body
	pub fn failure(bytes: &[u8]) -> Option<(::http::StatusCode, Bytes)> {
		let resp: vg::GenerateContentResponse = serde_json::from_slice(bytes).ok()?;
		let (status, error) = reply_failure(&resp)?;
		let body = serde_json::to_vec(&serde_json::json!({ "error": error })).ok()?;
		Some((status, Bytes::from(body)))
	}

	fn reply_failure(resp: &vg::GenerateContentResponse) -> Option<(::http::StatusCode, Value)> {
		let cand = resp.candidates.first()?;
		let reason = cand.finish_reason.as_deref()?;
		let (status, google_status) = match classify_finish_reason(reason) {
			Finish::Reason(_) => return None,
			Finish::Failed(status, google_status) => (status, google_status),
			Finish::Unknown => {
				tracing::warn!(
					reason,
					"unknown Gemini finish reason; reporting it as a failure"
				);
				(::http::StatusCode::BAD_GATEWAY, "INTERNAL")
			},
		};
		let message = match &cand.finish_message {
			Some(detail) => format!("Gemini stopped with {reason}: {detail}"),
			None => format!("Gemini stopped with {reason}"),
		};
		let error = serde_json::json!({
			"code": status.as_u16(),
			"message": message,
			"status": google_status,
		});
		Some((status, error))
	}

	fn reply_block_message(cand: &vg::Candidate) -> String {
		cand.finish_message.clone().unwrap_or_else(|| {
			let reason = cand.finish_reason.as_deref().unwrap_or_default();
			format!("Gemini blocked the reply: {reason}")
		})
	}

	/// The Gemini API gives only a reason; Vertex adds a message
	fn prompt_block_message(feedback: &vg::PromptFeedback) -> String {
		feedback.block_reason_message.clone().unwrap_or_else(|| {
			let reason = feedback.block_reason.as_deref().unwrap_or_default();
			format!("Gemini blocked the prompt: {reason}")
		})
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
			let blocked_prompt = chunk
				.prompt_feedback
				.as_ref()
				.filter(|pf| chunk.candidates.is_empty() && pf.block_reason.is_some());
			// A failed reply has no finish reason here; `translate_stream` sends its error
			let finish = match (cand, cand.and_then(|c| c.finish_reason.as_deref())) {
				(Some(cand), Some(reason)) => {
					let finish = finish_reason(reason, self.saw_function_call);
					if finish == Some(completions::FinishReason::ContentFilter) {
						delta.refusal = Some(reply_block_message(cand));
					}
					finish
				},
				_ => blocked_prompt.map(|pf| {
					delta.refusal = Some(prompt_block_message(pf));
					completions::FinishReason::ContentFilter
				}),
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
					delta,
					finish_reason: finish,
					..Default::default()
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
				service_tier: chunk.usage_metadata.as_ref().and_then(openai_service_tier),
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
		log_content: crate::LogContentFields,
	) -> Body {
		let mut state = StreamState::new();
		let mut saw_token = false;
		let mut last_token_at: Option<Instant> = None;
		let mut completion = log_content.completion.then(String::new);
		let mut tool_calls: Option<LoggedToolCalls> = log_content.tool_calls.then(HashMap::new);
		let end = Arc::new(parse::sse::StreamEnd::default());
		let seen_end = end.clone();
		let mut failed = false;
		let body = parse::sse::json_transform_multi::<
			vg::GenerateContentResponse,
			completions::StreamEvent,
			_,
		>(b, buffer_limit, move |ev| {
			let chunk = match ev {
				_ if failed => return vec![],
				parse::sse::SseJsonEvent::Data(Ok(c)) => c,
				parse::sse::SseJsonEvent::Data(Err(e)) => {
					tracing::debug!("failed to parse gemini stream chunk: {e}");
					return vec![];
				},
				parse::sse::SseJsonEvent::Done
				| parse::sse::SseJsonEvent::Eof
				| parse::sse::SseJsonEvent::Error => return vec![],
			};
			if let Some(error) = chunk.rest.get("error") {
				tracing::warn!(%error, "gemini stream failed");
				failed = true;
				seen_end.fail();
				return vec![("", completions::StreamEvent::Error(stream_error(error)))];
			}

			let now = Instant::now();
			if !saw_token {
				saw_token = true;
				last_token_at = Some(now);
				log.update(|r| r.response.first_token = Some(now));
			} else if let Some(prev) = last_token_at.replace(now) {
				let gap = now.duration_since(prev);
				log.update(|r| r.response.inter_chunk_latencies.record(gap));
			}
			if let Some(m) = &chunk.model_version {
				log.update(|r| {
					if r.response.provider_model.is_none() {
						r.response.provider_model = Some(strng::new(m));
					}
				});
			}
			if let Some(um) = &chunk.usage_metadata {
				let (prompt, completion, total) = um.counts();
				log.update(|r| {
					r.response.input_tokens = Some(prompt);
					r.response.output_tokens = Some(completion);
					r.response.total_tokens = Some(total);
					r.response.cached_input_tokens = um.cached_content_token_count;
					r.response.reasoning_tokens = um.thoughts_token_count;
					r.response.service_tier = um
						.service_tier
						.as_deref()
						.or(um.traffic_type.as_deref())
						.map(strng::new);
				});
			}

			if let Some((_, error)) = reply_failure(&chunk) {
				failed = true;
				seen_end.fail();
				return vec![("", completions::StreamEvent::Error(stream_error(&error)))];
			}

			match state.translate(&chunk) {
				// Gemini may omit modelVersion on a chunk
				Some(mut sr) => {
					if sr.model.is_empty() {
						sr.model = model.to_string();
					}
					if let Some(choice) = sr.choices.first() {
						if choice.finish_reason.is_some() {
							seen_end.finish();
						}
						if let Some(content) = &choice.delta.content
							&& let Some(completion) = completion.as_mut()
						{
							completion.push_str(content);
						}
						if let Some(calls) = &choice.delta.tool_calls {
							for call in calls {
								if let Some(tool_calls) = tool_calls.as_mut() {
									let entry = tool_calls.entry(call.index).or_default();
									if let Some(id) = &call.id {
										entry.0 = Some(id.clone());
									}
									if let Some(function) = &call.function {
										if let Some(name) = &function.name {
											entry.1 = Some(name.clone());
										}
										if let Some(arguments) = &function.arguments {
											entry.2.push_str(arguments);
										}
									}
								}
							}
						}
						if let Some(finish_reason) = choice
							.finish_reason
							.as_ref()
							.and_then(crate::types::serialize_str)
						{
							let tool_parts = tool_calls.as_mut().and_then(|tool_calls| {
								crate::conversion::completions::finalize_streaming_tool_calls(
									tool_calls
										.drain()
										.map(|(idx, (id, name, arguments))| (idx, id, name, arguments)),
								)
							});
							let mut tool_parts = tool_parts;
							let mut finish_reason = Some(finish_reason);
							log.update(|r| {
								if let Some(completion) = completion.take() {
									r.response.completion = Some(vec![completion]);
								}
								crate::conversion::completions::build_output_messages(
									&mut r.response,
									tool_parts.take(),
									finish_reason.take(),
								);
							});
						}
					}
					vec![("", completions::StreamEvent::Chunk(sr))]
				},
				None => vec![],
			}
		});
		parse::sse::append_done_if_finished(body, end)
	}

	fn stream_error(error: &Value) -> completions::ChatCompletionErrorResponse {
		match serde_json::from_value::<completions::GoogleError>(error.clone()) {
			Ok(error) => crate::conversion::completions::chat_error_from_google(&error),
			Err(_) => completions::ChatCompletionErrorResponse {
				event_id: None,
				error: completions::ChatCompletionError {
					r#type: Some("api_error".to_string()),
					message: error.to_string(),
					param: None,
					code: None,
					event_id: None,
				},
			},
		}
	}

	/// Maps Gemini service tiers and Vertex traffic types to OpenAI service tiers.
	fn openai_service_tier(um: &vg::UsageMetadata) -> Option<String> {
		let tier = match um.service_tier.as_deref().or(um.traffic_type.as_deref())? {
			"standard" | "ON_DEMAND" => "default",
			"flex" | "ON_DEMAND_FLEX" => "flex",
			"priority" | "ON_DEMAND_PRIORITY" => "priority",
			"PROVISIONED_THROUGHPUT" => "scale",
			_ => return None,
		};
		Some(tier.to_string())
	}

	fn build_usage(um: &vg::UsageMetadata) -> completions::Usage {
		let (prompt, completion, total) = um.counts();
		completions::Usage {
			prompt_tokens: prompt as u32,
			completion_tokens: completion as u32,
			total_tokens: total as u32,
			prompt_tokens_details: um.cached_content_token_count.map(|c| {
				completions::UsagePromptDetails {
					cached_tokens: Some(c),
					..Default::default()
				}
			}),
			completion_tokens_details: um.thoughts_token_count.map(|t| {
				completions::UsageCompletionDetails {
					reasoning_tokens: Some(t),
					..Default::default()
				}
			}),
			..Default::default()
		}
	}
}
