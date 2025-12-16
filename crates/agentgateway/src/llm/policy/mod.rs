use ::http::HeaderMap;
use bytes::Bytes;
use serde::de::DeserializeOwned;

use crate::http::filters::HeaderModifier;
use crate::http::jwt::Claims;
use crate::http::{Response, StatusCode, auth};
use crate::llm::policy::webhook::{MaskActionBody, RequestAction, ResponseAction};
use crate::llm::{AIError, RequestType, ResponseType};
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendPolicy, HeaderMatch, HeaderValueMatch, SimpleBackendReference};
use crate::*;

pub mod webhook;

mod moderation;
mod pii;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[apply(schema!)]
#[derive(Default)]
pub struct Policy {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt_guard: Option<PromptGuard>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub defaults: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub overrides: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompts: Option<PromptEnrichment>,
	#[serde(
		rename = "modelAliases",
		default,
		skip_serializing_if = "HashMap::is_empty"
	)]
	pub model_aliases: HashMap<Strng, Strng>,
	/// Compiled wildcard patterns, sorted by specificity (longer patterns first).
	/// Not serialized - computed from model_aliases during policy creation.
	/// Wrapped in Arc to avoid cloning compiled regex during policy merging.
	#[serde(skip)]
	pub wildcard_patterns: Arc<Vec<(ModelAliasPattern, Strng)>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt_caching: Option<PromptCachingConfig>,
	#[serde(default, skip_serializing_if = "IndexMap::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub routes: IndexMap<Strng, crate::llm::RouteType>,
}

/// Wildcard pattern converted to regex for model name matching.
/// Stores the compiled regex and original pattern length for specificity sorting.
#[apply(schema!)]
pub struct ModelAliasPattern {
	#[serde(with = "serde_regex")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	regex: regex::Regex,
	pattern_len: usize,
}

impl ModelAliasPattern {
	pub fn from_wildcard(pattern: &str) -> Result<Self, String> {
		if !pattern.contains('*') {
			return Err(format!("Pattern '{}' contains no wildcards", pattern));
		}

		// Convert wildcard to regex: escape all chars, then replace \* with (.*)
		let escaped = regex::escape(pattern);
		let regex_pattern = escaped.replace(r"\*", "(.*)");

		let regex = regex::Regex::new(&format!("^{}$", regex_pattern))
			.map_err(|e| format!("Invalid wildcard pattern '{}': {}", pattern, e))?;

		Ok(ModelAliasPattern {
			regex,
			pattern_len: pattern.len(),
		})
	}

	pub fn matches(&self, model: &str) -> bool {
		self.regex.is_match(model)
	}

	pub fn specificity(&self) -> usize {
		self.pattern_len
	}
}

#[apply(schema!)]
#[serde(default)]
pub struct PromptCachingConfig {
	#[serde(rename = "cacheSystem")]
	pub cache_system: bool,

	#[serde(rename = "cacheMessages")]
	pub cache_messages: bool,

	#[serde(rename = "cacheTools")]
	pub cache_tools: bool,

	#[serde(rename = "minTokens")]
	pub min_tokens: Option<usize>,
}

impl Default for PromptCachingConfig {
	fn default() -> Self {
		Self {
			cache_system: true,
			cache_messages: true,
			cache_tools: false,
			min_tokens: Some(1024),
		}
	}
}

#[apply(schema!)]
pub struct PromptEnrichment {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub append: Vec<crate::llm::SimpleChatCompletionMessage>,
	pub prepend: Vec<crate::llm::SimpleChatCompletionMessage>,
}

#[apply(schema!)]
pub struct PromptGuard {
	// Guards applied to client requests before they reach the LLM
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub request: Vec<RequestGuard>,
	// Guards applied to LLM responses before they reach the client
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub response: Vec<ResponseGuard>,
}

impl Policy {
	pub fn compile_model_alias_patterns(&mut self) {
		let mut patterns = Vec::new();

		for (key, value) in &self.model_aliases {
			if key.contains('*') {
				match ModelAliasPattern::from_wildcard(key.as_str()) {
					Ok(pattern) => {
						patterns.push((pattern, value.clone()));
					},
					Err(e) => {
						// Log warning but continue - don't fail entire policy
						tracing::warn!(
							pattern = %key,
							error = %e,
							"Invalid model alias wildcard pattern, skipping"
						);
					},
				}
			}
		}

		// Sort by specificity: longer patterns first (more specific matches)
		patterns.sort_by_key(|(pattern, _)| std::cmp::Reverse(pattern.specificity()));

		self.wildcard_patterns = Arc::new(patterns);

		tracing::debug!(
			exact_aliases = self.model_aliases.len(),
			wildcard_patterns = self.wildcard_patterns.len(),
			"Compiled model alias patterns"
		);
	}

	pub fn resolve_model_alias(&self, model: &str) -> Option<&Strng> {
		// Fast path: exact match in HashMap (O(1))
		if let Some(target) = self.model_aliases.get(model) {
			return Some(target);
		}

		// Slow path: pattern matching (sorted by specificity, checks longer patterns first)
		for (pattern, target) in self.wildcard_patterns.iter() {
			if pattern.matches(model) {
				tracing::debug!(
					model = %model,
					target = %target,
					pattern_specificity = pattern.specificity(),
					"Model alias pattern match"
				);
				return Some(target);
			}
		}

		None
	}

	pub fn apply_prompt_enrichment(&self, chat: &mut dyn RequestType) {
		if let Some(prompts) = &self.prompts {
			chat.prepend_prompts(prompts.prepend.clone());
		}
	}

	pub fn resolve_route(&self, path: &str) -> crate::llm::RouteType {
		for (path_suffix, rt) in &self.routes {
			if path_suffix == "*" || path.ends_with(path_suffix.as_str()) {
				return *rt;
			}
		}
		crate::llm::RouteType::Completions
	}

	pub fn unmarshal_request<T: DeserializeOwned>(&self, bytes: &Bytes) -> Result<T, AIError> {
		if self.defaults.is_none() && self.overrides.is_none() {
			// Fast path: directly bytes to typed
			return serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing);
		}
		// Slow path: bytes --> json (transform) --> typed
		let v: serde_json::Value =
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?;
		let serde_json::Value::Object(mut map) = v else {
			return Err(AIError::MissingField("request must be an object".into()));
		};
		for (k, v) in self.overrides.iter().flatten() {
			map.insert(k.clone(), v.clone());
		}
		for (k, v) in self.defaults.iter().flatten() {
			map.entry(k.clone()).or_insert_with(|| v.clone());
		}
		serde_json::from_value(serde_json::Value::Object(map)).map_err(AIError::RequestParsing)
	}

	pub async fn apply_prompt_guard(
		&self,
		backend_info: &auth::BackendInfo,
		req: &mut dyn RequestType,
		http_headers: &HeaderMap,
		claims: Option<Claims>,
	) -> anyhow::Result<Option<Response>> {
		let client = PolicyClient {
			inputs: backend_info.inputs.clone(),
		};
		for g in self
			.prompt_guard
			.as_ref()
			.iter()
			.flat_map(|g| g.request.iter())
		{
			match &g.kind {
				RequestGuardKind::Regex(rg) => {
					if let Some(res) = Self::apply_regex(req, rg, &g.rejection)? {
						return Ok(Some(res));
					}
				},
				RequestGuardKind::Webhook(wh) => {
					if let Some(res) = Self::apply_webhook(req, http_headers, &client, wh).await? {
						return Ok(Some(res));
					}
				},
				RequestGuardKind::OpenAIModeration(m) => {
					if let Some(res) =
						Self::apply_moderation(req, claims.clone(), &client, &g.rejection, m).await?
					{
						return Ok(Some(res));
					}
				},
			}
		}
		Ok(None)
	}

	async fn apply_moderation(
		req: &mut dyn RequestType,
		claims: Option<Claims>,
		client: &PolicyClient,
		rej: &RequestRejection,
		moderation: &Moderation,
	) -> anyhow::Result<Option<Response>> {
		let resp = moderation::send_request(req, claims, client, moderation).await?;
		if resp.results.iter().any(|r| r.flagged) {
			Ok(Some(rej.as_response()))
		} else {
			Ok(None)
		}
	}

	fn apply_regex(
		req: &mut dyn RequestType,
		rgx: &RegexRules,
		rej: &RequestRejection,
	) -> anyhow::Result<Option<Response>> {
		let mut msgs = req.get_messages();
		let mut any_changed = false;
		for msg in &mut msgs {
			match Self::apply_prompt_guard_regex(&msg.content, rgx) {
				Some(RegexResult::Reject) => {
					return Ok(Some(rej.as_response()));
				},
				Some(RegexResult::Mask(content)) => {
					any_changed = true;
					msg.content = content.into();
				},
				None => {},
			}
		}
		if any_changed {
			req.set_messages(msgs);
		}
		Ok(None)
	}

	fn apply_regex_response(
		resp: &mut dyn ResponseType,
		rgx: &RegexRules,
		rej: &RequestRejection,
	) -> anyhow::Result<Option<Response>> {
		let mut msgs = resp.to_webhook_choices();
		let mut any_changed = false;
		for msg in &mut msgs {
			match Self::apply_prompt_guard_regex(&msg.message.content, rgx) {
				Some(RegexResult::Reject) => {
					return Ok(Some(rej.as_response()));
				},
				Some(RegexResult::Mask(content)) => {
					any_changed = true;
					msg.message.content = content.into();
				},
				None => {},
			}
		}
		if any_changed {
			resp.set_webhook_choices(msgs)?;
		}
		Ok(None)
	}

	async fn apply_webhook(
		req: &mut dyn RequestType,
		http_headers: &HeaderMap,
		client: &PolicyClient,
		webhook: &Webhook,
	) -> anyhow::Result<Option<Response>> {
		let messsages = req.get_messages();
		let headers = Self::get_webhook_forward_headers(http_headers, &webhook.forward_header_matches);
		let whr = webhook::send_request(client, &webhook.target, &headers, messsages).await?;
		match whr.action {
			RequestAction::Mask(mask) => {
				debug!(
					"webhook masked request: {}",
					mask
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				let MaskActionBody::PromptMessages(body) = mask.body else {
					anyhow::bail!("invalid webhook response");
				};
				let msgs = body.messages;
				req.set_messages(msgs);
			},
			RequestAction::Reject(rej) => {
				debug!(
					"webhook rejected request: {}",
					rej
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				return Ok(Some(
					::http::response::Builder::new()
						.status(rej.status_code)
						.body(http::Body::from(rej.body))?,
				));
			},
			RequestAction::Pass(pass) => {
				debug!(
					"webhook passed request: {}",
					pass
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				// No action needed
			},
		}
		Ok(None)
	}

	async fn apply_webhook_response(
		resp: &mut dyn ResponseType,
		http_headers: &HeaderMap,
		client: &PolicyClient,
		webhook: &Webhook,
	) -> anyhow::Result<Option<Response>> {
		let messsages = resp.to_webhook_choices();
		let headers = Self::get_webhook_forward_headers(http_headers, &webhook.forward_header_matches);
		let whr = webhook::send_response(client, &webhook.target, &headers, messsages).await?;
		match whr.action {
			ResponseAction::Mask(mask) => {
				debug!(
					"webhook masked response: {}",
					mask
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				let MaskActionBody::ResponseChoices(body) = mask.body else {
					anyhow::bail!("invalid webhook response");
				};
				let msgs = body.choices;
				resp.set_webhook_choices(msgs)?;
			},
			ResponseAction::Pass(pass) => {
				debug!(
					"webhook passed response: {}",
					pass
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				// No action needed
			},
		}
		Ok(None)
	}

	fn get_webhook_forward_headers(
		http_headers: &HeaderMap,
		header_matches: &[HeaderMatch],
	) -> HeaderMap {
		let mut headers = HeaderMap::new();
		for HeaderMatch { name, value } in header_matches {
			// Only handle regular headers (HeaderMap doesn't contain pseudo headers)
			let header_name = match name {
				crate::http::HeaderOrPseudo::Header(h) => h,
				_ => continue, // Skip pseudo headers
			};
			let Some(have) = http_headers.get(header_name.as_str()) else {
				continue;
			};
			match value {
				HeaderValueMatch::Exact(want) => {
					if have != want {
						continue;
					}
				},
				HeaderValueMatch::Regex(want) => {
					// Must be a valid string to do regex match
					let Some(have_str) = have.to_str().ok() else {
						continue;
					};
					let Some(m) = want.find(have_str) else {
						continue;
					};
					// Make sure we matched the entire thing
					if !(m.start() == 0 && m.end() == have_str.len()) {
						continue;
					}
				},
			}
			headers.insert(header_name, have.clone());
		}
		headers
	}

	// fn convert_message(r: Message) -> ChatCompletionRequestMessage {
	// 	match r.role.as_str() {
	// 		"system" => universal::RequestMessage::from(universal::RequestSystemMessage::from(r.content)),
	// 		"assistant" => {
	// 			universal::RequestMessage::from(universal::RequestAssistantMessage::from(r.content))
	// 		},
	// 		// TODO: the webhook API cannot express functions or tools...
	// 		"function" => universal::RequestMessage::from(universal::RequestFunctionMessage {
	// 			content: Some(r.content),
	// 			name: "".to_string(),
	// 		}),
	// 		"tool" => universal::RequestMessage::from(universal::RequestToolMessage {
	// 			content: universal::RequestToolMessageContent::from(r.content),
	// 			tool_call_id: "".to_string(),
	// 		}),
	// 		_ => universal::RequestMessage::from(universal::RequestUserMessage::from(r.content)),
	// 	}
	// }

	fn apply_prompt_guard_regex(original_content: &str, rgx: &RegexRules) -> Option<RegexResult> {
		let mut current_content = original_content.to_string();
		let mut content_modified = false;

		// Process each rule sequentially, updating the content as we go
		for r in &rgx.rules {
			match r {
				RegexRule::Builtin { builtin } => {
					let rec = match builtin {
						Builtin::Ssn => &*pii::SSN,
						Builtin::CreditCard => &*pii::CC,
						Builtin::PhoneNumber => &*pii::PHONE,
						Builtin::Email => &*pii::EMAIL,
						Builtin::CaSin => &*pii::CA_SIN,
					};
					let results = pii::recognizer(rec, &current_content);

					if !results.is_empty() {
						match &rgx.action {
							Action::Reject => {
								return Some(RegexResult::Reject);
							},
							Action::Mask => {
								// Sort in reverse to avoid index shifting during replacement
								let mut sorted_results = results;
								sorted_results.sort_by(|a, b| b.start.cmp(&a.start));

								for result in sorted_results {
									current_content.replace_range(
										result.start..result.end,
										&format!("<{}>", result.entity_type.to_uppercase()),
									);
								}
								content_modified = true;
							},
						}
					}
				},
				RegexRule::Regex { pattern } => {
					let ranges: Vec<std::ops::Range<usize>> = pattern
						.find_iter(&current_content)
						.map(|m| m.range())
						.collect();

					if !ranges.is_empty() {
						match &rgx.action {
							Action::Reject => {
								return Some(RegexResult::Reject);
							},
							Action::Mask => {
								// Process matches in reverse order to avoid index shifting
								for range in ranges.into_iter().rev() {
									current_content.replace_range(range, "<masked>");
								}
								content_modified = true;
							},
						}
					}
				},
			}
		}
		// Only update the message if content was actually modified
		if content_modified {
			return Some(RegexResult::Mask(current_content));
		}
		None
	}

	pub async fn apply_response_prompt_guard(
		client: &PolicyClient,
		resp: &mut dyn ResponseType,
		http_headers: &HeaderMap,
		guards: &Vec<ResponseGuard>,
	) -> anyhow::Result<Option<Response>> {
		for g in guards {
			match &g.kind {
				ResponseGuardKind::Regex(rg) => {
					if let Some(res) = Self::apply_regex_response(resp, rg, &g.rejection)? {
						return Ok(Some(res));
					}
				},
				ResponseGuardKind::Webhook(wh) => {
					if let Some(res) = Self::apply_webhook_response(resp, http_headers, client, wh).await? {
						return Ok(Some(res));
					}
				},
			}
		}
		Ok(None)
	}
}

enum RegexResult {
	Mask(String),
	Reject,
}

#[apply(schema!)]
pub struct RequestGuard {
	#[serde(default)]
	pub rejection: RequestRejection,
	#[serde(flatten)]
	pub kind: RequestGuardKind,
}

#[apply(schema!)]
pub enum RequestGuardKind {
	Regex(RegexRules),
	Webhook(Webhook),
	OpenAIModeration(Moderation),
}

#[apply(schema!)]
pub struct RegexRules {
	#[serde(default)]
	pub action: Action,
	pub rules: Vec<RegexRule>,
}

#[apply(schema!)]
#[serde(untagged)]
pub enum RegexRule {
	Builtin {
		builtin: Builtin,
	},
	Regex {
		#[serde(with = "serde_regex")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		pattern: regex::Regex,
	},
}

impl RequestRejection {
	pub fn as_response(&self) -> Response {
		let mut response = ::http::response::Builder::new()
			.status(self.status)
			.body(http::Body::from(self.body.clone()))
			.expect("static request should succeed");

		// Apply header modifications if present
		if let Some(ref headers) = self.headers
			&& let Err(e) = headers.apply(response.headers_mut())
		{
			warn!("Failed to apply rejection response headers: {}", e);
		}

		response
	}
}

#[apply(schema!)]
pub enum Builtin {
	#[serde(rename = "ssn")]
	Ssn,
	CreditCard,
	PhoneNumber,
	Email,
	CaSin,
}

#[apply(schema!)]
pub struct Rule<T> {
	action: Action,
	rule: T,
}

#[apply(schema!)]
pub struct NamedRegex {
	#[serde(with = "serde_regex")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pattern: regex::Regex,
	name: String,
}

#[apply(schema!)]
pub struct Webhook {
	pub target: SimpleBackendReference,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub forward_header_matches: Vec<HeaderMatch>,
}

#[apply(schema!)]
pub struct Moderation {
	/// Model to use. Defaults to `omni-moderation-latest`
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(skip_deserializing)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub policies: Vec<BackendPolicy>,
}

#[apply(schema!)]
#[derive(Default)]
pub enum Action {
	#[default]
	Mask,
	Reject,
}

#[apply(schema!)]
pub struct RequestRejection {
	#[serde(default = "default_body", serialize_with = "ser_string_or_bytes")]
	pub body: Bytes,
	#[serde(default = "default_code", with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	pub status: StatusCode,
	/// Optional headers to add, set, or remove from the rejection response
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub headers: Option<HeaderModifier>,
}

impl Default for RequestRejection {
	fn default() -> Self {
		Self {
			body: default_body(),
			status: default_code(),
			headers: None,
		}
	}
}

#[apply(schema!)]
pub struct ResponseGuard {
	#[serde(default)]
	pub rejection: RequestRejection,
	#[serde(flatten)]
	pub kind: ResponseGuardKind,
}

#[apply(schema!)]
pub enum ResponseGuardKind {
	Regex(RegexRules),
	Webhook(Webhook),
}

#[apply(schema!)]
pub struct PromptGuardRegex {}
fn default_code() -> StatusCode {
	StatusCode::FORBIDDEN
}

fn default_body() -> Bytes {
	Bytes::from_static(b"The request was rejected due to inappropriate content")
}
