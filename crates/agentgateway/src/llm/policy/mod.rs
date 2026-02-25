use crate::http::filters::HeaderModifier;
use crate::http::jwt::Claims;
use crate::http::{Response, StatusCode, auth};
use crate::llm::policy::webhook::{MaskActionBody, RequestAction, ResponseAction};
use crate::llm::{AIError, RequestType, ResponseType};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;
use crate::types::agent::{BackendPolicy, HeaderMatch, HeaderValueMatch, SimpleBackendReference};
use crate::types::local::LocalBackendPolicies;
use crate::*;
use ::http::HeaderMap;
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod webhook;

mod bedrock_guardrails;
mod google_model_armor;
mod moderation;
mod pii;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Routes stored in a deterministic order: **longest key to shortest key**, with `"*"` always last.
///
/// This lets us iterate and match more-specific suffixes first.
#[derive(Debug, Clone, Default)]
pub struct SortedRoutes {
	inner: IndexMap<Strng, crate::llm::RouteType>,
}

impl SortedRoutes {
	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}

	pub fn insert(&mut self, k: Strng, v: crate::llm::RouteType) -> Option<crate::llm::RouteType> {
		let prev = self.inner.insert(k, v);
		self.sort();
		prev
	}

	fn sort(&mut self) {
		// Sort by:
		// - wildcard last
		// - longer keys first
		// - stable tie-breaker (lexicographic) for deterministic output
		let mut entries: Vec<(Strng, crate::llm::RouteType)> =
			std::mem::take(&mut self.inner).into_iter().collect();
		entries.sort_by(|(a, _), (b, _)| {
			let a = a.as_str();
			let b = b.as_str();
			(a == "*", std::cmp::Reverse(a.len()), a).cmp(&(b == "*", std::cmp::Reverse(b.len()), b))
		});
		self.inner = entries.into_iter().collect();
	}
}

impl std::ops::Deref for SortedRoutes {
	type Target = IndexMap<Strng, crate::llm::RouteType>;

	fn deref(&self) -> &Self::Target {
		&self.inner
	}
}

impl<'a> IntoIterator for &'a SortedRoutes {
	type Item = (&'a Strng, &'a crate::llm::RouteType);
	type IntoIter = indexmap::map::Iter<'a, Strng, crate::llm::RouteType>;

	fn into_iter(self) -> Self::IntoIter {
		self.inner.iter()
	}
}

impl FromIterator<(Strng, crate::llm::RouteType)> for SortedRoutes {
	fn from_iter<T: IntoIterator<Item = (Strng, crate::llm::RouteType)>>(iter: T) -> Self {
		let mut routes = Self {
			inner: iter.into_iter().collect(),
		};
		routes.sort();
		routes
	}
}

impl Serialize for SortedRoutes {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.inner.serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for SortedRoutes {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let mut routes = Self {
			inner: IndexMap::<Strng, crate::llm::RouteType>::deserialize(deserializer)?,
		};
		routes.sort();
		Ok(routes)
	}
}

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
	pub transformations: Option<HashMap<String, Arc<cel::Expression>>>,
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
	#[serde(default, skip_serializing_if = "SortedRoutes::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, crate::llm::RouteType>")
	)]
	pub routes: SortedRoutes,
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
#[cfg_attr(feature = "schema", schemars(extend("minProperties" = 1)))]
pub struct PromptEnrichment {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub append: Vec<crate::llm::SimpleChatCompletionMessage>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
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

enum GuardrailOutcome {
	None,
	Masked,
	Rejected(Response),
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
			if !prompts.prepend.is_empty() {
				chat.prepend_prompts(prompts.prepend.clone());
			}
			if !prompts.append.is_empty() {
				chat.append_prompts(prompts.append.clone());
			}
		}
	}

	pub fn resolve_route(&self, path: &str) -> crate::llm::RouteType {
		let mut wildcard: Option<crate::llm::RouteType> = None;

		// `self.routes` is stored longest->shortest, with "*" last, so the first match wins.
		for (path_suffix, rt) in self.routes.iter() {
			if path_suffix.as_str() == "*" {
				wildcard = Some(*rt);
				continue;
			}
			if path.ends_with(path_suffix.as_str()) {
				return *rt;
			}
		}

		wildcard.unwrap_or(crate::llm::RouteType::Completions)
	}

	pub fn unmarshal_request<T: DeserializeOwned>(
		&self,
		bytes: &Bytes,
		log: &mut Option<&mut RequestLog>,
	) -> Result<T, AIError> {
		if self.defaults.is_none() && self.overrides.is_none() && self.transformations.is_none() {
			// Fast path: directly bytes to typed
			return serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing);
		}
		// Slow path: bytes --> json (transform) --> typed
		let v: serde_json::Value =
			serde_json::from_slice(bytes.as_ref()).map_err(AIError::RequestParsing)?;
		let exec = cel::Executor::new_llm(log.as_ref().and_then(|x| x.request_snapshot.as_ref()), &v);
		let to_set: Vec<_> = self
			.transformations
			.iter()
			.flatten()
			.map(|(k, expr)| (k, Self::eval_transformation_expression(expr, &exec)))
			.collect();

		let serde_json::Value::Object(mut map) = v else {
			return Err(AIError::MissingField("request must be an object".into()));
		};
		for (k, v) in self.overrides.iter().flatten() {
			map.insert(k.clone(), v.clone());
		}
		for (k, v) in to_set.into_iter() {
			match v {
				Some(v) => {
					map.insert(k.clone(), v);
				},
				None => {
					map.remove(k);
				},
			}
		}
		for (k, v) in self.defaults.iter().flatten() {
			map.entry(k.clone()).or_insert_with(|| v.clone());
		}
		serde_json::from_value(serde_json::Value::Object(map)).map_err(AIError::RequestParsing)
	}

	fn eval_transformation_expression(
		expression: &cel::Expression,
		exec: &cel::Executor<'_>,
	) -> Option<serde_json::Value> {
		exec.eval(expression).ok()?.json().ok()
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
				RequestGuardKind::Regex(rg) => match Self::apply_regex(req, rg, &g.rejection)? {
					GuardrailOutcome::Rejected(res) => {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					},
					GuardrailOutcome::Masked => {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Mask,
						);
					},
					GuardrailOutcome::None => {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
					},
				},
				RequestGuardKind::Webhook(wh) => {
					if let Some(res) = Self::apply_webhook(req, http_headers, &client, wh).await? {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					}
				},
				RequestGuardKind::OpenAIModeration(m) => {
					if let Some(res) =
						Self::apply_moderation(req, claims.clone(), &client, &g.rejection, m).await?
					{
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					} else {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
					}
				},
				RequestGuardKind::BedrockGuardrails(bg) => {
					if let Some(res) =
						Self::apply_bedrock_guardrails_request(req, claims.clone(), &client, &g.rejection, bg)
							.await?
					{
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					} else {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
					}
				},
				RequestGuardKind::GoogleModelArmor(gma) => {
					if let Some(res) =
						Self::apply_google_model_armor_request(req, claims.clone(), &client, &g.rejection, gma)
							.await?
					{
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					} else {
						Self::record_guardrail_trip(
							&client,
							crate::telemetry::metrics::GuardrailPhase::Request,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
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

	async fn apply_bedrock_guardrails_request(
		req: &mut dyn RequestType,
		claims: Option<Claims>,
		client: &PolicyClient,
		rej: &RequestRejection,
		guardrails: &BedrockGuardrails,
	) -> anyhow::Result<Option<Response>> {
		let resp = bedrock_guardrails::send_request(req, claims.clone(), client, guardrails).await?;
		if resp.is_blocked() {
			Ok(Some(rej.as_response()))
		} else {
			Ok(None)
		}
	}

	async fn apply_bedrock_guardrails_response(
		resp: &mut dyn ResponseType,
		claims: Option<Claims>,
		client: &PolicyClient,
		rej: &RequestRejection,
		guardrails: &BedrockGuardrails,
	) -> anyhow::Result<Option<Response>> {
		// Extract text content from response choices
		let content: Vec<String> = resp
			.to_webhook_choices()
			.into_iter()
			.map(|c| c.message.content.to_string())
			.collect();

		if content.is_empty() {
			return Ok(None);
		}

		let guardrail_resp =
			bedrock_guardrails::send_response(content, claims, client, guardrails).await?;
		if guardrail_resp.is_blocked() {
			Ok(Some(rej.as_response()))
		} else {
			Ok(None)
		}
	}

	async fn apply_google_model_armor_request(
		req: &mut dyn RequestType,
		claims: Option<Claims>,
		client: &PolicyClient,
		rej: &RequestRejection,
		model_armor: &GoogleModelArmor,
	) -> anyhow::Result<Option<Response>> {
		let resp = google_model_armor::send_request(req, claims.clone(), client, model_armor).await?;
		if resp.is_blocked() {
			Ok(Some(rej.as_response()))
		} else {
			Ok(None)
		}
	}

	async fn apply_google_model_armor_response(
		resp: &mut dyn ResponseType,
		claims: Option<Claims>,
		client: &PolicyClient,
		rej: &RequestRejection,
		model_armor: &GoogleModelArmor,
	) -> anyhow::Result<Option<Response>> {
		// Extract text content from response choices
		let content: Vec<String> = resp
			.to_webhook_choices()
			.into_iter()
			.map(|c| c.message.content.to_string())
			.collect();

		if content.is_empty() {
			return Ok(None);
		}

		let guardrail_resp =
			google_model_armor::send_response(content, claims.clone(), client, model_armor).await?;
		if guardrail_resp.is_blocked() {
			Ok(Some(rej.as_response()))
		} else {
			Ok(None)
		}
	}

	fn apply_regex(
		req: &mut dyn RequestType,
		rgx: &RegexRules,
		rej: &RequestRejection,
	) -> anyhow::Result<GuardrailOutcome> {
		let mut msgs = req.get_messages();
		let mut any_changed = false;
		for msg in &mut msgs {
			match Self::apply_prompt_guard_regex(&msg.content, rgx) {
				Some(RegexResult::Reject) => {
					return Ok(GuardrailOutcome::Rejected(rej.as_response()));
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
			return Ok(GuardrailOutcome::Masked);
		}
		Ok(GuardrailOutcome::None)
	}

	fn apply_regex_response(
		resp: &mut dyn ResponseType,
		rgx: &RegexRules,
		rej: &RequestRejection,
	) -> anyhow::Result<GuardrailOutcome> {
		let mut msgs = resp.to_webhook_choices();
		let mut any_changed = false;
		for msg in &mut msgs {
			match Self::apply_prompt_guard_regex(&msg.message.content, rgx) {
				Some(RegexResult::Reject) => {
					return Ok(GuardrailOutcome::Rejected(rej.as_response()));
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
			return Ok(GuardrailOutcome::Masked);
		}
		Ok(GuardrailOutcome::None)
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
				Self::record_guardrail_trip(
					client,
					crate::telemetry::metrics::GuardrailPhase::Request,
					crate::telemetry::metrics::GuardrailAction::Mask,
				);
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
				Self::record_guardrail_trip(
					client,
					crate::telemetry::metrics::GuardrailPhase::Request,
					crate::telemetry::metrics::GuardrailAction::Allow,
				);
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
				Self::record_guardrail_trip(
					client,
					crate::telemetry::metrics::GuardrailPhase::Response,
					crate::telemetry::metrics::GuardrailAction::Mask,
				);
			},
			ResponseAction::Reject(rej) => {
				debug!(
					"webhook rejected response: {}",
					rej
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				Self::record_guardrail_trip(
					client,
					crate::telemetry::metrics::GuardrailPhase::Response,
					crate::telemetry::metrics::GuardrailAction::Reject,
				);
				return Ok(Some(
					::http::response::Builder::new()
						.status(rej.status_code)
						.body(http::Body::from(rej.body))?,
				));
			},
			ResponseAction::Pass(pass) => {
				debug!(
					"webhook passed response: {}",
					pass
						.reason
						.unwrap_or_else(|| "no reason specified".to_string())
				);
				Self::record_guardrail_trip(
					client,
					crate::telemetry::metrics::GuardrailPhase::Response,
					crate::telemetry::metrics::GuardrailAction::Allow,
				);
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

	fn record_guardrail_trip(
		client: &PolicyClient,
		phase: crate::telemetry::metrics::GuardrailPhase,
		action: crate::telemetry::metrics::GuardrailAction,
	) {
		client
			.inputs
			.metrics
			.guardrail_checks
			.get_or_create(&crate::telemetry::metrics::GuardrailLabels { phase, action })
			.inc();
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
				ResponseGuardKind::Regex(rg) => match Self::apply_regex_response(resp, rg, &g.rejection)? {
					GuardrailOutcome::Rejected(res) => {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					},
					GuardrailOutcome::Masked => {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Mask,
						);
					},
					GuardrailOutcome::None => {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
					},
				},
				ResponseGuardKind::Webhook(wh) => {
					if let Some(res) = Self::apply_webhook_response(resp, http_headers, client, wh).await? {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					}
				},
				ResponseGuardKind::BedrockGuardrails(bg) => {
					if let Some(res) =
						Self::apply_bedrock_guardrails_response(resp, None, client, &g.rejection, bg).await?
					{
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					} else {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
					}
				},
				ResponseGuardKind::GoogleModelArmor(gma) => {
					if let Some(res) =
						Self::apply_google_model_armor_response(resp, None, client, &g.rejection, gma).await?
					{
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Reject,
						);
						return Ok(Some(res));
					} else {
						Self::record_guardrail_trip(
							client,
							crate::telemetry::metrics::GuardrailPhase::Response,
							crate::telemetry::metrics::GuardrailAction::Allow,
						);
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
	BedrockGuardrails(BedrockGuardrails),
	GoogleModelArmor(GoogleModelArmor),
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
	#[serde(deserialize_with = "de_from_local_backend_policy")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalBackendPolicies"))]
	pub policies: Vec<BackendPolicy>,
}

/// Configuration for AWS Bedrock Guardrails integration.
#[apply(schema!)]
pub struct BedrockGuardrails {
	/// The unique identifier of the guardrail
	pub guardrail_identifier: Strng,
	/// The version of the guardrail
	pub guardrail_version: Strng,
	/// AWS region where the guardrail is deployed
	pub region: Strng,
	/// Backend policies for AWS authentication (optional, defaults to implicit AWS auth)
	#[serde(
		default,
		deserialize_with = "de_from_local_backend_policy_opt",
		skip_serializing_if = "Vec::is_empty"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<LocalBackendPolicies>"))]
	pub policies: Vec<BackendPolicy>,
}

/// Configuration for Google Cloud Model Armor integration.
#[apply(schema!)]
pub struct GoogleModelArmor {
	/// The template ID for the Model Armor configuration
	pub template_id: Strng,
	/// The GCP project ID
	pub project_id: Strng,
	/// The GCP region (default: us-central1)
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub location: Option<Strng>,
	/// Backend policies for GCP authentication (optional, defaults to implicit GCP auth)
	#[serde(
		default,
		deserialize_with = "de_from_local_backend_policy_opt",
		skip_serializing_if = "Vec::is_empty"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<LocalBackendPolicies>"))]
	pub policies: Vec<BackendPolicy>,
}

pub fn de_from_local_backend_policy<'de: 'a, 'a, D>(
	deserializer: D,
) -> Result<Vec<BackendPolicy>, D::Error>
where
	D: Deserializer<'de>,
{
	let s = LocalBackendPolicies::deserialize(deserializer)?;
	s.translate().map_err(serde::de::Error::custom)
}

pub fn de_from_local_backend_policy_opt<'de: 'a, 'a, D>(
	deserializer: D,
) -> Result<Vec<BackendPolicy>, D::Error>
where
	D: Deserializer<'de>,
{
	let s = Option::<LocalBackendPolicies>::deserialize(deserializer)?;
	match s {
		Some(policies) => policies.translate().map_err(serde::de::Error::custom),
		None => Ok(Vec::new()),
	}
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
	BedrockGuardrails(BedrockGuardrails),
	GoogleModelArmor(GoogleModelArmor),
}

#[apply(schema!)]
pub struct PromptGuardRegex {}
fn default_code() -> StatusCode {
	StatusCode::FORBIDDEN
}

fn default_body() -> Bytes {
	Bytes::from_static(b"The request was rejected due to inappropriate content")
}

#[test]
fn test_prompt_caching_policy_deserialization() {
	use serde_json::json;

	let json = json!({
		"promptCaching": {
			"cacheSystem": true,
			"cacheMessages": true,
			"cacheTools": false,
			"minTokens": 1024
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system);
	assert!(caching.cache_messages);
	assert!(!caching.cache_tools);
	assert_eq!(caching.min_tokens, Some(1024));
}

#[test]
fn test_prompt_caching_policy_defaults() {
	use serde_json::json;

	// Empty config should have system and messages enabled by default
	let json = json!({
		"promptCaching": {}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system); // Default: true
	assert!(caching.cache_messages); // Default: true
	assert!(!caching.cache_tools); // Default: false
	assert_eq!(caching.min_tokens, Some(1024)); // Default: 1024
}

#[test]
fn test_policy_without_prompt_caching_field() {
	use serde_json::json;

	let json = json!({
		"modelAliases": {
			"gpt-4": "anthropic.claude-3-sonnet-20240229-v1:0"
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// prompt_caching should be None when not specified
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_prompt_caching_explicit_disable() {
	use serde_json::json;

	// Explicitly disable caching
	let json = json!({
		"promptCaching": null
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// Should be None when explicitly set to null
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_resolve_route() {
	let mut routes = SortedRoutes::default();
	routes.insert(
		strng::literal!("/completions"),
		crate::llm::RouteType::Completions,
	);
	routes.insert(
		strng::literal!("/v1/messages"),
		crate::llm::RouteType::Messages,
	);
	routes.insert(strng::literal!("*"), crate::llm::RouteType::Passthrough);

	let policy = Policy {
		routes,
		..Default::default()
	};

	// Suffix matching
	assert_eq!(
		policy.resolve_route("/v1/chat/completions"),
		crate::llm::RouteType::Completions
	);
	assert_eq!(
		policy.resolve_route("/api/completions"),
		crate::llm::RouteType::Completions
	);
	// Exact suffix match
	assert_eq!(
		policy.resolve_route("/v1/messages"),
		crate::llm::RouteType::Messages
	);
	// Wildcard fallback
	assert_eq!(
		policy.resolve_route("/v1/models"),
		crate::llm::RouteType::Passthrough
	);
	// Empty routes defaults to Completions
	assert_eq!(
		Policy::default().resolve_route("/any/path"),
		crate::llm::RouteType::Completions
	);
}

#[test]
fn test_model_alias_wildcard_resolution() {
	let mut policy = Policy {
		model_aliases: HashMap::from([
			(strng::new("gpt-4"), strng::new("exact-target")),
			(
				strng::new("claude-haiku-3.5-*"),
				strng::new("haiku-3.5-target"),
			),
			(strng::new("claude-haiku-*"), strng::new("haiku-target")),
			(strng::new("*-sonnet-*"), strng::new("sonnet-target")),
		]),
		..Default::default()
	};

	policy.compile_model_alias_patterns();

	// Exact match takes precedence over wildcards
	assert_eq!(
		policy.resolve_model_alias("gpt-4"),
		Some(&strng::new("exact-target"))
	);

	// Longer patterns are more specific (checked first)
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-3.5-v1"),
		Some(&strng::new("haiku-3.5-target")) // Matches "claude-haiku-3.5-*" not "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-v1"),
		Some(&strng::new("haiku-target")) // Only matches "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("other-sonnet-model"),
		Some(&strng::new("sonnet-target")) // Matches "*-sonnet-*"
	);

	// No match returns None
	assert_eq!(policy.resolve_model_alias("unmatched-model"), None);
}

#[test]
fn test_model_alias_pattern_validation() {
	// Pattern must contain wildcard
	assert!(ModelAliasPattern::from_wildcard("no-wildcards").is_err());

	// Special characters are escaped (dot is literal, not regex wildcard)
	let pattern = ModelAliasPattern::from_wildcard("test.*").unwrap();
	assert!(pattern.matches("test.v1"));
	assert!(!pattern.matches("testXv1")); // X doesn't match literal dot
}

#[test]
fn test_unmarshal_request_with_transformation_policy() {
	use serde_json::json;

	let policy = Policy {
		transformations: Some(
			[
				(
					"max_tokens".to_string(),
					Arc::new(cel::Expression::new_strict("min(llmRequest.max_tokens, 50)").unwrap()),
				),
				(
					"model".to_string(),
					Arc::new(
						cel::Expression::new_strict(
							r#"
				llmRequest.model.split("/").with(m,
					m.size() == 2 ? m[1] : m[0]
				)"#,
						)
						.unwrap(),
					),
				),
			]
			.into_iter()
			.collect(),
		),
		..Default::default()
	};

	let input = Bytes::from_static(br#"{"model":"provider/model","max_tokens":999}"#);
	let out: serde_json::Value = policy
		.unmarshal_request(&input, &mut None)
		.expect("request should unmarshal");

	assert_eq!(out.get("model"), Some(&json!("model")));
	assert_eq!(out.get("max_tokens"), Some(&json!(50)));
}
