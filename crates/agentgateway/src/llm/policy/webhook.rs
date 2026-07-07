use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, HeaderValue, header};

use crate::llm::policy::with_default_timeout;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackendReference;
use crate::*;
pub use agent_llm::policy::webhook::*;

const REQUEST_PATH: &str = "request";
const RESPONSE_PATH: &str = "response";

fn build_request_for_request(
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsPromptRequest {
		body: PromptMessages { messages },
	};
	build_request(&body, REQUEST_PATH, http_headers)
}

fn build_request_for_response(
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsResponseRequest {
		body: ResponseChoices { choices },
	};
	build_request(&body, RESPONSE_PATH, http_headers)
}

fn build_request<T: serde::Serialize>(
	body: &T,
	path: &str,
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
	let req = rb
		.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
		.body(crate::http::Body::from(body_bytes))?;
	Ok(req)
}

pub async fn send_request(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<GuardrailsPromptResponse> {
	let whr = with_default_timeout(build_request_for_request(http_headers, messages)?);
	let res = Box::pin(
		client
			.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
			.call_reference(whr, target),
	)
	.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

pub async fn send_response(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<GuardrailsResponseResponse> {
	let whr = with_default_timeout(build_request_for_response(http_headers, choices)?);
	let res = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_reference(whr, target)
		.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}
