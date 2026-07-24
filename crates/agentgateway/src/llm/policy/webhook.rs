use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, HeaderValue, header};
pub use agent_llm::webhook::{Message, ResponseChoice};
use serde::{Deserialize, Serialize};

use crate::cel::RequestSnapshot;
use crate::http::{HeaderOrPseudoValue, RequestOrResponse};
use crate::llm::policy::{Webhook, with_default_timeout};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::*;

const REQUEST_PATH: &str = "request";
const RESPONSE_PATH: &str = "response";

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
	/// - RejectAction: Reject the response.
	pub action: ResponseAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptMessages {
	/// List of prompt messages including role and content.
	pub messages: Vec<Message>,
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
	Reject(RejectAction),
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
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsPromptRequest {
		body: PromptMessages { messages },
	};
	build_request(&body, REQUEST_PATH, webhook, original, http_headers)
}

fn build_request_for_response(
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsResponseRequest {
		body: ResponseChoices { choices },
	};
	build_request(&body, RESPONSE_PATH, webhook, original, http_headers)
}

fn build_request<T: serde::Serialize>(
	body: &T,
	path: &str,
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
	http_headers: &HeaderMap,
) -> anyhow::Result<crate::http::Request> {
	let body_bytes = serde_json::to_vec(body)?;
	let mut rb = ::http::Request::builder()
		.uri(format!("/{path}"))
		.method(http::Method::POST);
	for (k, v) in http_headers {
		// TODO: this is configurable by users
		if k == header::CONTENT_LENGTH {
			// TODO: probably others
			continue;
		}
		rb = rb.header(k.clone(), v.clone());
	}
	let mut req = rb
		.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
		.body(crate::http::Body::from(body_bytes))?;
	apply_header_expressions(&mut req, webhook, original);
	Ok(req)
}

/// Apply the configured CEL header expressions to the outgoing webhook request.
/// Setting the `:path` pseudo-header replaces the default `/request` / `/response` path.
///
/// Expressions are evaluated against the original incoming request (like the
/// `transformation` and `extAuthz` policies), so `request.*`, `jwt.*`, etc. refer to
/// the client's request, not the webhook request being built.
fn apply_header_expressions(
	req: &mut crate::http::Request,
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
) {
	if webhook.headers.is_empty() {
		return;
	}
	let exec = cel::Executor::new_request_snapshot(original);
	for (k, expr) in &webhook.headers {
		let v = match exec.eval(expr) {
			Ok(v) => Some(v),
			Err(e) => {
				debug!("webhook header expression for {k} failed: {e}");
				None
			},
		};
		let val = HeaderOrPseudoValue::from_cel_result(k, v);
		if val.is_none() {
			debug!("webhook header expression for {k} did not produce a value");
		}
		RequestOrResponse::Request(req).apply_header(
			k,
			val,
			http::HeaderMutationAction::OverwriteIfExistsOrAdd,
		);
	}
}

pub async fn send_request(
	client: &PolicyClient,
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<GuardrailsPromptResponse> {
	let whr = with_default_timeout(build_request_for_request(
		webhook,
		original,
		http_headers,
		messages,
	)?);
	let res = Box::pin(
		client
			.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
			.call_reference(whr, &webhook.target),
	)
	.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

pub async fn send_response(
	client: &PolicyClient,
	webhook: &Webhook,
	original: Option<&RequestSnapshot>,
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<GuardrailsResponseResponse> {
	let whr = with_default_timeout(build_request_for_response(
		webhook,
		original,
		http_headers,
		choices,
	)?);
	let res = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_reference(whr, &webhook.target)
		.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use secrecy::SecretString;

	use super::*;
	use crate::http::HeaderOrPseudo;
	use crate::http::jwt::Claims;
	use crate::llm::policy::FailureMode;
	use crate::types::agent::SimpleBackendReference;

	fn webhook(headers: Vec<(HeaderOrPseudo, Arc<cel::Expression>)>) -> Webhook {
		Webhook {
			target: SimpleBackendReference::Invalid,
			headers,
			forward_header_matches: vec![],
			failure_mode: FailureMode::FailClosed,
		}
	}

	fn expr(s: &str) -> Arc<cel::Expression> {
		Arc::new(cel::Expression::new_strict(s).unwrap())
	}

	/// Snapshot of an original client request: POST /v1/chat/completions with an
	/// x-tenant header and (optionally) JWT claims.
	fn original(claims: Option<Claims>) -> RequestSnapshot {
		let mut req = ::http::Request::builder()
			.method(http::Method::POST)
			.uri("http://example.com/v1/chat/completions")
			.header("x-tenant", "acme")
			.body(crate::http::Body::empty())
			.unwrap();
		if let Some(claims) = claims {
			req.extensions_mut().insert(claims);
		}
		cel::snapshot_request(&mut req, false)
	}

	fn claims() -> Claims {
		Claims {
			inner: serde_json::from_value(serde_json::json!({"sub": "user-123"})).unwrap(),
			jwt: SecretString::from("token"),
		}
	}

	#[test]
	fn default_paths_are_preserved() {
		let wh = webhook(vec![]);
		let req = build_request_for_request(&wh, None, &HeaderMap::new(), vec![]).unwrap();
		assert_eq!(req.uri().path(), "/request");
		let resp = build_request_for_response(&wh, None, &HeaderMap::new(), vec![]).unwrap();
		assert_eq!(resp.uri().path(), "/response");
	}

	#[test]
	fn path_pseudo_header_overrides_default_path() {
		let wh = webhook(Vec::from([(
			HeaderOrPseudo::Path,
			expr(r#""/v3/guardrails/agentgateway/request""#),
		)]));
		let snap = original(None);
		let req = build_request_for_request(&wh, Some(&snap), &HeaderMap::new(), vec![]).unwrap();
		assert_eq!(req.uri().path(), "/v3/guardrails/agentgateway/request");
	}

	#[test]
	fn expressions_see_the_original_request() {
		// request.* refers to the original client request, not the webhook request.
		let wh = webhook(Vec::from([
			(
				HeaderOrPseudo::Header(::http::HeaderName::from_static("x-orig-path")),
				expr("request.path"),
			),
			(
				HeaderOrPseudo::Header(::http::HeaderName::from_static("x-tenant")),
				expr(r#"request.headers["x-tenant"]"#),
			),
		]));
		let snap = original(None);
		let req = build_request_for_request(&wh, Some(&snap), &HeaderMap::new(), vec![]).unwrap();
		assert_eq!(
			req.headers().get("x-orig-path").unwrap(),
			"/v1/chat/completions"
		);
		assert_eq!(req.headers().get("x-tenant").unwrap(), "acme");
		// The webhook path itself is untouched by non-:path expressions.
		assert_eq!(req.uri().path(), "/request");
	}

	#[test]
	fn jwt_claims_available_in_expressions() {
		let wh = webhook(Vec::from([(
			HeaderOrPseudo::Header(::http::HeaderName::from_static("x-user")),
			expr("jwt.sub"),
		)]));
		let snap = original(Some(claims()));
		let req = build_request_for_request(&wh, Some(&snap), &HeaderMap::new(), vec![]).unwrap();
		assert_eq!(req.headers().get("x-user").unwrap(), "user-123");
	}

	#[test]
	fn claims_are_not_attached_to_the_outgoing_request() {
		let wh = webhook(Vec::from([(
			HeaderOrPseudo::Header(::http::HeaderName::from_static("x-user")),
			expr("jwt.sub"),
		)]));
		let snap = original(Some(claims()));
		let req = build_request_for_request(&wh, Some(&snap), &HeaderMap::new(), vec![]).unwrap();
		// Claims are only exposed to CEL evaluation; leaking them onto the request
		// would hand the raw JWT to the webhook backend's policy chain (e.g.
		// backendAuth passthrough).
		assert!(req.extensions().get::<Claims>().is_none());
	}

	#[test]
	fn failed_expression_removes_header_and_keeps_path() {
		// An expression referencing missing context must not set the header,
		// and a failed :path expression must leave the default path intact.
		let wh = webhook(Vec::from([
			(
				HeaderOrPseudo::Header(::http::HeaderName::from_static("x-missing")),
				expr(r#"request.headers["does-not-exist"]"#),
			),
			(HeaderOrPseudo::Path, expr("jwt.missing_claim")),
		]));
		let snap = original(None);
		let req = build_request_for_request(&wh, Some(&snap), &HeaderMap::new(), vec![]).unwrap();
		assert!(req.headers().get("x-missing").is_none());
		assert_eq!(req.uri().path(), "/request");
	}

	#[test]
	fn no_snapshot_degrades_gracefully() {
		// With no original-request snapshot (e.g. context unavailable), expressions
		// fail soft: default path kept, headers unset.
		let wh = webhook(Vec::from([
			(HeaderOrPseudo::Path, expr(r#""/prefixed/request""#)),
			(
				HeaderOrPseudo::Header(::http::HeaderName::from_static("x-user")),
				expr("jwt.sub"),
			),
		]));
		let req = build_request_for_request(&wh, None, &HeaderMap::new(), vec![]).unwrap();
		// Static expressions still work without a snapshot...
		assert_eq!(req.uri().path(), "/prefixed/request");
		// ...but context-dependent ones are skipped.
		assert!(req.headers().get("x-user").is_none());
	}
}
