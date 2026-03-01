use agent_core::strng;
use itertools::Itertools;

use crate::http::jwt::Claims;
use crate::json;
use crate::llm::RequestType;
use crate::llm::policy::Moderation;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::SpanWriter;
use crate::types::agent::{BackendPolicy, ResourceName, SimpleBackend, Target};

pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	moderation: &Moderation,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<async_openai::types::moderations::CreateModerationResponse> {
	let model = moderation
		.model
		.clone()
		.unwrap_or(strng::literal!("omni-moderation-latest"));
	let mut pols = vec![BackendPolicy::BackendTLS(
		crate::http::backendtls::SYSTEM_TRUST.clone(),
	)];
	pols.extend(moderation.policies.iter().cloned());
	// let auth = BackendAuth::from(moderation.auth.clone());
	let content = req
		.get_messages()
		.into_iter()
		.map(|t| t.content)
		.collect_vec();
	let mut rb = ::http::Request::builder()
		.uri("https://api.openai.com/v1/moderations")
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
	let mock_be = SimpleBackend::Opaque(
		ResourceName::new(strng::literal!("_openai-moderation"), strng::literal!("")),
		Target::Hostname(strng::literal!("api.openai.com"), 443),
	);
	let resp = client
		.call_with_explicit_policies_and_span(req, mock_be, pols, span_writer)
		.await?;
	let resp: async_openai::types::moderations::CreateModerationResponse =
		json::from_response_body(resp).await?;
	Ok(resp)
}
