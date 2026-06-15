use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::http::jwt::Claims;
use crate::llm::RequestType;
use crate::llm::policy::BedrockGuardrails;
use crate::llm::policy::guardrail::{GuardrailBackend, GuardrailBedrock};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{SimpleBackend, SimpleBackendWithPolicies};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuardrailSource {
	/// Content from user input (requests)
	Input,
	/// Content from model output (responses)
	Output,
}

/// Text content block for guardrail evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailTextBlock {
	pub text: String,
}

/// Content block for guardrail evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailContentBlock {
	pub text: GuardrailTextBlock,
}

/// Output content from guardrail with masked/anonymized text
#[derive(Debug, Clone, Deserialize)]
pub struct GuardrailOutputContent {
	pub text: String,
}

/// Request body for ApplyGuardrail API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ApplyGuardrailRequest {
	/// The source of the content (INPUT for requests, OUTPUT for responses)
	pub source: GuardrailSource,
	/// The content blocks to evaluate
	pub content: Vec<GuardrailContentBlock>,
}

/// Action taken by the guardrail
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuardrailAction {
	/// No intervention needed
	None,
	/// Guardrail intervened and blocked/modified content
	GuardrailIntervened,
}

/// Response from ApplyGuardrail API
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApplyGuardrailResponse {
	/// The action taken by the guardrail
	pub action: GuardrailAction,
	/// Outputs with masked text (if configured with mask)
	#[serde(default)]
	pub outputs: Vec<GuardrailOutputContent>,
	/// Assessment details containing action type (BLOCKED, ANONYMIZED, etc.)
	#[serde(default)]
	pub assessments: Vec<serde_json::Value>,
}

impl ApplyGuardrailResponse {
	/// Returns true if the guardrail blocked content
	pub fn is_blocked(&self) -> bool {
		self.action == GuardrailAction::GuardrailIntervened && self.has_blocked_assessment()
	}

	/// Returns true if the guardrail anonymized/masked content
	pub fn is_anonymized(&self) -> bool {
		self.action == GuardrailAction::GuardrailIntervened && !self.has_blocked_assessment()
	}

	/// Returns the masked output texts
	pub fn output_texts(&self) -> Vec<String> {
		self.outputs.iter().map(|o| o.text.clone()).collect()
	}

	/// Check if any assessment contains a BLOCKED action
	fn has_blocked_assessment(&self) -> bool {
		self.assessments.iter().any(Self::value_contains_blocked)
	}

	/// Search for "action": "BLOCKED" in JSON value
	fn value_contains_blocked(value: &serde_json::Value) -> bool {
		match value {
			serde_json::Value::Object(map) => {
				if let Some(serde_json::Value::String(action)) = map.get("action")
					&& action == "BLOCKED"
				{
					return true;
				}
				map.values().any(Self::value_contains_blocked)
			},
			serde_json::Value::Array(arr) => arr.iter().any(Self::value_contains_blocked),
			_ => false,
		}
	}
}

/// Resolve the guardrail backend (referenced or built from the deprecated inline fields)
/// along with its typed Bedrock provider configuration.
fn resolve(
	client: &PolicyClient,
	guardrails: &BedrockGuardrails,
) -> anyhow::Result<(SimpleBackendWithPolicies, GuardrailBedrock)> {
	let backend = super::resolve_guardrail_backend(
		client,
		guardrails.backend_ref.as_ref(),
		|| {
			let (Some(identifier), Some(version), Some(region)) = (
				guardrails.guardrail_identifier.clone(),
				guardrails.guardrail_version.clone(),
				guardrails.region.clone(),
			) else {
				anyhow::bail!(
					"bedrockGuardrails requires either backendRef or guardrailIdentifier, guardrailVersion, and region"
				);
			};
			Ok(GuardrailBackend::Bedrock(GuardrailBedrock {
				identifier,
				version,
				region,
			}))
		},
		strng::literal!("_bedrock-guardrails"),
	)?;
	let cfg = if let SimpleBackend::Guardrail(_, GuardrailBackend::Bedrock(b)) = &backend.backend {
		b.clone()
	} else {
		anyhow::bail!(
			"guardrail backend {} is not a bedrock guardrail backend",
			backend.backend
		)
	};
	Ok((backend, cfg))
}

/// Send a request to the Bedrock Guardrails ApplyGuardrail API for request content
pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	guardrails: &BedrockGuardrails,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let content = req
		.get_messages()
		.into_iter()
		.map(|m| GuardrailContentBlock {
			text: GuardrailTextBlock {
				text: m.content.to_string(),
			},
		})
		.collect_vec();

	send_guardrail_request(
		client,
		claims.clone(),
		guardrails,
		GuardrailSource::Input,
		content,
	)
	.await
}

/// Send a request to the Bedrock Guardrails ApplyGuardrail API for response content
pub async fn send_response(
	content: Vec<String>,
	claims: Option<Claims>,
	client: &PolicyClient,
	guardrails: &BedrockGuardrails,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let content = content
		.into_iter()
		.map(|text| GuardrailContentBlock {
			text: GuardrailTextBlock { text },
		})
		.collect_vec();

	send_guardrail_request(
		client,
		claims.clone(),
		guardrails,
		GuardrailSource::Output,
		content,
	)
	.await
}

async fn send_guardrail_request(
	client: &PolicyClient,
	claims: Option<Claims>,
	guardrails: &BedrockGuardrails,
	source: GuardrailSource,
	content: Vec<GuardrailContentBlock>,
) -> anyhow::Result<ApplyGuardrailResponse> {
	let (backend, cfg) = resolve(client, guardrails)?;
	let request_body = ApplyGuardrailRequest { source, content };
	// The transport (endpoint host, TLS, AWS auth) is owned by the resolved backend;
	// only the API path and body are built here.
	let path = format!(
		"/guardrail/{}/version/{}/apply",
		cfg.identifier, cfg.version
	);

	tracing::debug!(
		request_body = %serde_json::to_string_pretty(&request_body).unwrap_or_default(),
		path = %path,
		"Sending Bedrock guardrail request"
	);

	// AWS requires both Content-Type and Accept headers
	let mut rb = ::http::Request::builder()
		.uri(&path)
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.header(::http::header::ACCEPT, "application/json");

	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}

	let req = rb.body(crate::http::Body::from(serde_json::to_vec(&request_body)?))?;

	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_resolved(req, backend, &guardrails.policies)
		.await?;

	let status = resp.status();
	let lim = crate::http::response_buffer_limit(&resp);
	let (_, body) = resp.into_parts();
	let bytes = crate::http::read_body_with_limit(body, lim).await?;

	if !status.is_success() {
		let error_body = String::from_utf8_lossy(&bytes);
		tracing::warn!(
			status = %status,
			error_body = %error_body,
			guardrail_id = %cfg.identifier,
			"Bedrock guardrail API returned error"
		);
		anyhow::bail!(
			"Bedrock guardrail API error: status={}, body={}",
			status,
			error_body
		);
	}

	let resp: ApplyGuardrailResponse = serde_json::from_slice(&bytes)
		.map_err(|e| anyhow::anyhow!("Failed to parse Bedrock guardrail response: {e}"))?;

	if resp.is_blocked() {
		tracing::debug!(
			guardrail_id = %cfg.identifier,
			guardrail_version = %cfg.version,
			source = ?source,
			"Bedrock guardrail blocked content"
		);
	} else if resp.is_anonymized() {
		tracing::debug!(
			guardrail_id = %cfg.identifier,
			guardrail_version = %cfg.version,
			source = ?source,
			"Bedrock guardrail anonymized content"
		);
	}

	Ok(resp)
}
