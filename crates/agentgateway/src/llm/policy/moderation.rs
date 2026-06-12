use agent_core::strng;
use itertools::Itertools;

use crate::http::jwt::Claims;
use crate::json;
use crate::llm::RequestType;
use crate::llm::policy::Moderation;
use crate::llm::policy::guardrail::{GuardrailBackend, GuardrailOpenAIModeration};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{SimpleBackend, SimpleBackendWithPolicies};

/// Resolve the guardrail backend (referenced, or a synthetic OpenAI moderation backend).
fn resolve(
	client: &PolicyClient,
	moderation: &Moderation,
) -> anyhow::Result<SimpleBackendWithPolicies> {
	let backend = super::resolve_guardrail_backend(
		client,
		moderation.backend_ref.as_ref(),
		|| {
			Ok(GuardrailBackend::OpenAIModeration(
				GuardrailOpenAIModeration {},
			))
		},
		strng::literal!("_openai-moderation"),
	)?;
	if !matches!(
		&backend.backend,
		SimpleBackend::Guardrail(_, GuardrailBackend::OpenAIModeration(_))
	) {
		anyhow::bail!(
			"guardrail backend {} is not an openAI moderation guardrail backend",
			backend.backend
		);
	}
	Ok(backend)
}

pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	moderation: &Moderation,
) -> anyhow::Result<async_openai::types::moderations::CreateModerationResponse> {
	let backend = resolve(client, moderation)?;
	let model = moderation
		.model
		.clone()
		.unwrap_or(strng::literal!("omni-moderation-latest"));
	let content = req
		.get_messages()
		.into_iter()
		.map(|t| t.content)
		.collect_vec();
	let mut rb = ::http::Request::builder()
		.uri("/v1/moderations")
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json");
	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}
	let req = rb.body(crate::http::Body::from(serde_json::to_vec(
		&serde_json::json!({
			"input": content,
			"model": model,
		}),
	)?))?;
	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_resolved(req, backend, &moderation.policies)
		.await?;
	let resp: async_openai::types::moderations::CreateModerationResponse =
		json::from_response_body(resp).await?;
	Ok(resp)
}
