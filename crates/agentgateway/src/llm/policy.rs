use ::http::HeaderMap;
use async_openai::types::{ChatCompletionRequestMessage, CreateChatCompletionResponse};
use bytes::Bytes;

use crate::http::auth::{BackendAuth, SimpleBackendAuth};
use crate::http::jwt::Claims;
use crate::http::{Response, StatusCode, auth};
use crate::llm::policy::webhook::{MaskActionBody, Message, RequestAction};
use crate::llm::{AIError, pii, universal};
use crate::types::agent::{HeaderMatch, HeaderValueMatch, Target};
use crate::{client, *};

#[apply(schema!)]
pub struct Policy {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt_guard: Option<PromptGuard>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub defaults: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub overrides: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompts: Option<PromptEnrichment>,
}

#[apply(schema!)]
pub struct PromptEnrichment {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "crate::llm::SimpleChatCompletionMessage")
	)]
	pub append: Vec<ChatCompletionRequestMessage>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "crate::llm::SimpleChatCompletionMessage")
	)]
	pub prepend: Vec<ChatCompletionRequestMessage>,
}

#[apply(schema!)]
pub struct PromptGuard {
	// Guards applied to client requests before they reach the LLM
	pub request: Option<RequestGuard>,
	// Guards applied to LLM responses before they reach the client
	pub response: Option<ResponseGuard>,
}
impl Policy {
	pub fn apply_prompt_enrichment(&self, chat: &mut universal::Request) -> universal::Request {
		if let Some(prompts) = &self.prompts {
			let old_messages = std::mem::take(&mut chat.messages);
			chat.messages = prompts
				.prepend
				.clone()
				.into_iter()
				.chain(old_messages)
				.chain(prompts.append.clone())
				.collect();
		}
		chat.clone()
	}
	pub fn unmarshal_request(&self, bytes: &Bytes) -> Result<universal::Request, AIError> {
		if self.defaults.is_none() && self.overrides.is_none() && self.prompts.is_none() {
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
		client: &client::Client,
		req: &mut universal::Request,
		http_headers: &HeaderMap,
		claims: Option<Claims>,
	) -> anyhow::Result<Option<Response>> {
		let Some(g) = self.prompt_guard.as_ref().and_then(|g| g.request.as_ref()) else {
			return Ok(None);
		};
		if let Some(moderation) = &g.openai_moderation {
			let model = moderation
				.model
				.clone()
				.unwrap_or(strng::literal!("omni-moderation-latest"));
			let auth = BackendAuth::from(moderation.auth.clone());
			let content = req
				.messages
				.iter()
				.filter_map(universal::message_text)
				.collect::<Vec<_>>();
			let mut rb = ::http::Request::builder()
				.uri("https://api.openai.com/v1/moderations")
				.method(::http::Method::POST)
				.header(::http::header::CONTENT_TYPE, "application/json");
			if let Some(claims) = claims {
				rb = rb.extension(claims);
			}
			let mut req = rb.body(http::Body::from(serde_json::to_vec(&serde_json::json!({
				"input": content,
				"model": model,
			}))?))?;
			auth::apply_backend_auth(Some(&auth), &mut req).await?;
			let resp = client.simple_call(req).await;
			let resp: async_openai::types::CreateModerationResponse =
				json::from_body(resp?.into_body()).await?;
			if resp.results.iter().any(|r| r.flagged) {
				return Ok(Some(g.rejection.as_response()));
			}
		}
		if let Some(webhook) = &g.webhook {
			let headers =
				Self::get_webhook_forward_headers(http_headers, &webhook.forward_header_matches);
			let whr = webhook::send_request(client, &webhook.target, &headers, req).await?;
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
					req.messages = msgs.into_iter().map(Self::convert_message).collect();
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
		}
		for msg in &mut req.messages {
			let Some(original_content) = universal::message_text(msg) else {
				continue;
			};

			let (res, modified_content) = Self::apply_prompt_guard_regex(original_content, &g.regex);
			if let Some(content) = modified_content {
				*msg = Self::convert_message(Message {
					role: universal::message_role(msg).to_string(),
					content,
				});
			}
			if res.is_some() {
				return Ok(res);
			}
		}
		Ok(None)
	}

	fn get_webhook_forward_headers(
		http_headers: &HeaderMap,
		header_matches: &[HeaderMatch],
	) -> HeaderMap {
		let mut headers = HeaderMap::new();
		for HeaderMatch { name, value } in header_matches {
			let Some(have) = http_headers.get(name.as_str()) else {
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
					let Some(have) = have.to_str().ok() else {
						continue;
					};
					let Some(m) = want.find(have) else {
						continue;
					};
					// Make sure we matched the entire thing
					if !(m.start() == 0 && m.end() == have.len()) {
						continue;
					}
				},
			}
			headers.insert(name, have.clone());
		}
		headers
	}

	fn convert_message(r: Message) -> ChatCompletionRequestMessage {
		match r.role.as_str() {
			"system" => universal::RequestMessage::from(universal::RequestSystemMessage::from(r.content)),
			"assistant" => {
				universal::RequestMessage::from(universal::RequestAssistantMessage::from(r.content))
			},
			// TODO: the webhook API cannot express functions or tools...
			"function" => universal::RequestMessage::from(universal::RequestFunctionMessage {
				content: Some(r.content),
				name: "".to_string(),
			}),
			"tool" => universal::RequestMessage::from(universal::RequestToolMessage {
				content: universal::RequestToolMessageContent::from(r.content),
				tool_call_id: "".to_string(),
			}),
			_ => universal::RequestMessage::from(universal::RequestUserMessage::from(r.content)),
		}
	}

	fn apply_prompt_guard_regex(
		original_content: &str,
		regex: &Option<RegexRules>,
	) -> (Option<Response>, Option<String>) {
		if let Some(rgx) = regex {
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
						};
						let results = pii::recognizer(rec, &current_content);

						if !results.is_empty() {
							match &rgx.action {
								Action::Reject { response } => {
									return (Some(response.as_response()), None);
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
					RegexRule::Regex { pattern, name } => {
						let ranges: Vec<std::ops::Range<usize>> = pattern
							.find_iter(&current_content)
							.map(|m| m.range())
							.collect();

						if !ranges.is_empty() {
							match &rgx.action {
								Action::Reject { response } => {
									return (Some(response.as_response()), None);
								},
								Action::Mask => {
									// Process matches in reverse order to avoid index shifting
									for range in ranges.into_iter().rev() {
										current_content.replace_range(range, &format!("<{name}>"));
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
				return (None, Some(current_content));
			}
		}
		(None, None)
	}

	pub async fn apply_response_prompt_guard(
		client: &client::Client,
		resp: &mut CreateChatCompletionResponse,
		http_headers: &HeaderMap,
		g: &Option<ResponseGuard>,
	) -> anyhow::Result<Option<Response>> {
		let Some(guard) = g else {
			return Ok(None);
		};

		if let Some(webhook) = &guard.webhook {
			let headers =
				Self::get_webhook_forward_headers(http_headers, &webhook.forward_header_matches);
			let whr = webhook::send_response(client, &webhook.target, &headers, resp).await?;
			match whr.action {
				RequestAction::Mask(mask) => {
					debug!(
						"webhook masked response: {}",
						mask
							.reason
							.unwrap_or_else(|| "no reason specified".to_string())
					);
					let MaskActionBody::PromptMessages(body) = mask.body else {
						anyhow::bail!("invalid webhook response");
					};
					let msgs = body.messages;
					if resp.choices.len() != msgs.len() {
						anyhow::bail!("webhook response message count mismatch");
					}
					for (i, (resp_msg, wh_msg)) in resp.choices.iter_mut().zip(msgs).enumerate() {
						if resp_msg.message.role.to_string() != wh_msg.role {
							anyhow::bail!(
								"webhook response message {} role mismatch; expected {}, got {}",
								i,
								resp_msg.message.role,
								wh_msg.role
							);
						}
						resp_msg.message.content = Some(wh_msg.content);
					}
				},
				RequestAction::Reject(rej) => {
					debug!(
						"webhook rejected response: {}",
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
						"webhook passed response: {}",
						pass
							.reason
							.unwrap_or_else(|| "no reason specified".to_string())
					);
					// No action needed
				},
			}
		}

		for msg in resp.choices.iter_mut() {
			let Some(original_content) = msg.message.content.as_deref() else {
				continue;
			};

			let (res, modified_content) = Self::apply_prompt_guard_regex(original_content, &guard.regex);
			if let Some(content) = modified_content {
				msg.message.content = Some(content);
			}
			if res.is_some() {
				return Ok(res);
			}
		}
		Ok(None)
	}
}

#[apply(schema!)]
pub struct RequestGuard {
	#[serde(default)]
	pub rejection: RequestRejection,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub regex: Option<RegexRules>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub webhook: Option<Webhook>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub openai_moderation: Option<Moderation>,
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
		name: String,
	},
}

impl RequestRejection {
	pub fn as_response(&self) -> Response {
		::http::response::Builder::new()
			.status(self.status)
			.body(http::Body::from(self.body.clone()))
			.expect("static request should succeed")
	}
}

#[apply(schema!)]
pub enum Builtin {
	#[serde(rename = "ssn")]
	Ssn,
	CreditCard,
	PhoneNumber,
	Email,
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
	pub target: Target,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub forward_header_matches: Vec<HeaderMatch>,
}

#[apply(schema!)]
pub struct Moderation {
	/// Model to use. Defaults to `omni-moderation-latest`
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	#[serde(serialize_with = "ser_redact")]
	pub auth: SimpleBackendAuth,
}

#[apply(schema!)]
#[derive(Default)]
pub enum Action {
	#[default]
	Mask,
	Reject {
		#[serde(default)]
		response: RequestRejection,
	},
}

#[apply(schema!)]
pub struct RequestRejection {
	#[serde(default = "default_body", serialize_with = "ser_string_or_bytes")]
	pub body: Bytes,
	#[serde(default = "default_code", with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	pub status: StatusCode,
}

impl Default for RequestRejection {
	fn default() -> Self {
		Self {
			body: default_body(),
			status: default_code(),
		}
	}
}

#[apply(schema!)]
pub struct ResponseGuard {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub regex: Option<RegexRules>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub webhook: Option<Webhook>,
}

#[apply(schema!)]
pub struct PromptGuardRegex {}
fn default_code() -> StatusCode {
	StatusCode::FORBIDDEN
}

fn default_body() -> Bytes {
	Bytes::from_static(b"The request was rejected due to inappropriate content")
}

mod webhook {
	use ::http::header::CONTENT_TYPE;
	use ::http::{HeaderMap, HeaderValue, header};
	use async_openai::types::CreateChatCompletionResponse;
	use serde::{Deserialize, Serialize};

	use crate::client::Client;
	use crate::llm::universal::Request;
	use crate::types::agent::Target;
	use crate::*;

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct GuardrailsPromptRequest {
		/// body contains the object which is a list of the Message JSON objects from the prompts in the request
		pub body: PromptMessages,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct GuardrailsPromptResponse {
		/// action is the action to be taken based on the request.
		/// The following actions are available on the response:
		/// - PassAction: No action is required.
		/// - MaskAction: Mask the response body.
		/// - RejectAction: Reject the request.
		pub action: RequestAction,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct GuardrailsResponseRequest {
		/// body contains the object with a list of Choice that contains the response content from the LLM.
		pub body: ResponseChoices,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct GuardrailsResponseResponse {
		/// action is the action to be taken based on the request.
		/// The following actions are available on the response:
		/// - PassAction: No action is required.
		/// - MaskAction: Mask the response body.
		pub action: ResponseAction,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct Message {
		/// The role associated to the content in this message.
		pub role: String,
		/// The content text for this message.
		pub content: String,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct PromptMessages {
		/// List of prompt messages including role and content.
		pub messages: Vec<Message>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct ResponseChoice {
		/// message contains the role and text content of the response from the LLM model.
		pub message: Message,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct ResponseChoices {
		/// list of possible independent responses from the LLM
		pub choices: Vec<ResponseChoice>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct PassAction {
		/// reason is a human readable string that explains the reason for the action.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reason: Option<String>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct MaskAction {
		/// body contains the modified messages that masked out some of the original contents.
		/// When used in a GuardrailPromptResponse, this should be PromptMessages.
		/// When used in GuardrailResponseResponse, this should be ResponseChoices
		pub body: MaskActionBody,
		/// reason is a human readable string that explains the reason for the action.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reason: Option<String>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(rename_all = "snake_case")]
	pub struct RejectAction {
		/// body is the rejection message that will be used for HTTP error response body.
		pub body: String,
		/// status_code is the HTTP status code to be returned in the HTTP error response.
		pub status_code: u16,
		/// reason is a human readable string that explains the reason for the action.
		#[serde(skip_serializing_if = "Option::is_none")]
		pub reason: Option<String>,
	}

	/// Enum for actions available in prompt responses
	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(untagged, rename_all = "snake_case")]
	pub enum RequestAction {
		Mask(MaskAction),
		Reject(RejectAction),
		Pass(PassAction),
	}

	/// Enum for actions available in response responses
	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(untagged, rename_all = "snake_case")]
	pub enum ResponseAction {
		Mask(MaskAction),
		Pass(PassAction),
	}

	/// Enum for MaskAction body that can be either PromptMessages or ResponseChoices
	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(untagged)]
	pub enum MaskActionBody {
		PromptMessages(PromptMessages),
		ResponseChoices(ResponseChoices),
	}

	fn build_request_for_request(
		target: &Target,
		http_headers: &HeaderMap,
		i: &Request,
	) -> anyhow::Result<crate::http::Request> {
		let body = GuardrailsPromptRequest {
			body: PromptMessages {
				messages: i
					.messages
					.iter()
					.filter_map(|m| {
						let role = llm::universal::message_role(m).to_string();
						let content = llm::universal::message_text(m).map(|s| s.to_string());
						content.map(|content| Message { role, content })
					})
					.collect(),
			},
		};
		build_request(&body, target, http_headers)
	}

	fn build_request_for_response(
		target: &Target,
		http_headers: &HeaderMap,
		resp: &CreateChatCompletionResponse,
	) -> anyhow::Result<crate::http::Request> {
		let body = GuardrailsPromptRequest {
			body: PromptMessages {
				messages: resp
					.choices
					.iter()
					.filter_map(|m| {
						let role = m.message.role.to_string();
						let content = m.message.content.clone();
						content.map(|content| Message { role, content })
					})
					.collect(),
			},
		};
		build_request(&body, target, http_headers)
	}

	fn build_request(
		body: &GuardrailsPromptRequest,
		target: &Target,
		http_headers: &HeaderMap,
	) -> anyhow::Result<crate::http::Request> {
		let body_bytes = serde_json::to_vec(&body)?;
		let mut rb = ::http::Request::builder()
			.uri(format!("http://{target}/request"))
			.method(http::Method::POST);
		for (k, v) in http_headers {
			// TODO: this is configurable by users
			if k == header::CONTENT_LENGTH {
				// TODO: probably others
				continue;
			}
			rb = rb.header(k.clone(), v.clone());
		}
		let req = rb
			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
			.body(crate::http::Body::from(body_bytes))?;
		Ok(req)
	}

	pub async fn send_request(
		client: &Client,
		target: &Target,
		http_headers: &HeaderMap,
		req: &Request,
	) -> anyhow::Result<GuardrailsPromptResponse> {
		let whr = build_request_for_request(target, http_headers, req)?;
		let res = client
			.call(client::Call {
				req: whr,
				target: target.clone(),
				transport: Default::default(), // TODO: use policies
			})
			.await?;
		let parsed = json::from_body(res.into_body()).await?;
		Ok(parsed)
	}

	pub async fn send_response(
		client: &Client,
		target: &Target,
		http_headers: &HeaderMap,
		resp: &CreateChatCompletionResponse,
	) -> anyhow::Result<GuardrailsPromptResponse> {
		let whr = build_request_for_response(target, http_headers, resp)?;
		let res = client
			.call(client::Call {
				req: whr,
				target: target.clone(),
				transport: Default::default(), // TODO: use policies
			})
			.await?;
		let parsed = json::from_body(res.into_body()).await?;
		Ok(parsed)
	}
}

#[cfg(test)]
mod tests {
	use ::http::{HeaderName, HeaderValue};

	use super::*;

	#[test]
	fn test_get_webhook_forward_headers() {
		let mut headers = HeaderMap::new();
		headers.insert("x-test-header", HeaderValue::from_static("test-value"));
		headers.insert(
			"x-another-header",
			HeaderValue::from_static("another-value"),
		);
		headers.insert(
			"x-regex-header",
			HeaderValue::from_static("regex-match-123"),
		);

		let header_matches = vec![
			HeaderMatch {
				name: HeaderName::from_static("x-test-header"),
				value: HeaderValueMatch::Exact(HeaderValue::from_static("test-value")),
			},
			HeaderMatch {
				name: HeaderName::from_static("x-another-header"),
				value: HeaderValueMatch::Exact(HeaderValue::from_static("wrong-value")),
			},
			HeaderMatch {
				name: HeaderName::from_static("x-regex-header"),
				value: HeaderValueMatch::Regex(regex::Regex::new(r"regex-match-\d+").unwrap()),
			},
			HeaderMatch {
				name: HeaderName::from_static("x-missing-header"),
				value: HeaderValueMatch::Exact(HeaderValue::from_static("some-value")),
			},
		];

		let result = Policy::get_webhook_forward_headers(&headers, &header_matches);

		assert_eq!(result.len(), 2);
		assert_eq!(
			result.get("x-test-header").unwrap(),
			&HeaderValue::from_static("test-value")
		);
		assert_eq!(
			result.get("x-regex-header").unwrap(),
			&HeaderValue::from_static("regex-match-123")
		);
	}
}
