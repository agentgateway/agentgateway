use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::cel::GuardDetail;
use crate::http::auth::{AwsAuth, BackendAuthKind};
use crate::http::jwt::Claims;
use crate::llm::bedrock::AwsRegion;
use crate::llm::policy::{BedrockGuardrails, with_default_timeout};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{Backend, BackendTrafficPolicy, ResourceName};

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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct ApplyGuardrailResponse {
	/// The action taken by the guardrail
	pub action: GuardrailAction,
	/// The reason the guardrail reported for its action
	#[serde(default)]
	pub action_reason: Option<String>,
	/// Outputs with masked text (if configured with mask)
	#[serde(default)]
	pub outputs: Vec<GuardrailOutputContent>,
	/// Assessment details containing action type (BLOCKED, ANONYMIZED, etc.)
	#[serde(default)]
	pub assessments: Vec<serde_json::Value>,
}

impl ApplyGuardrailResponse {
	/// Returns true if the guardrail intervened at all
	pub fn is_intervened(&self) -> bool {
		self.action == GuardrailAction::GuardrailIntervened
	}

	/// Returns true if the guardrail blocked content
	pub fn is_blocked(&self) -> bool {
		self.is_intervened() && self.has_assessment_action("BLOCKED")
	}

	/// Returns true if the guardrail anonymized/masked content without blocking.
	pub fn is_anonymized(&self) -> bool {
		self.is_intervened()
			&& !self.has_assessment_action("BLOCKED")
			&& self.has_assessment_action("ANONYMIZED")
	}

	pub fn into_output_texts(self) -> Vec<String> {
		self.outputs.into_iter().map(|o| o.text).collect()
	}

	fn has_assessment_action(&self, action: &str) -> bool {
		self
			.assessments
			.iter()
			.any(|a| Self::value_contains_action(a, action))
	}

	/// Search for `"action": "<action>"` in JSON value
	fn value_contains_action(value: &serde_json::Value, action: &str) -> bool {
		match value {
			serde_json::Value::Object(map) => {
				if let Some(serde_json::Value::String(a)) = map.get("action")
					&& a == action
				{
					return true;
				}
				map.values().any(|v| Self::value_contains_action(v, action))
			},
			serde_json::Value::Array(arr) => arr.iter().any(|v| Self::value_contains_action(v, action)),
			_ => false,
		}
	}
}

/// Assessment keys that are safe to log. Top-level keys are policy names (fixed
/// API schema), so an unknown policy is kept as `"[redacted]"` to show what
/// fired; below that, unknown keys are dropped wholesale so content — and key
/// names that could carry it in future API shapes — never reaches logs or CEL.
const ASSESSMENT_METADATA_KEYS: &[&str] = &[
	// policy containers
	"topicPolicy",
	"topics",
	"contentPolicy",
	"filters",
	"sensitiveInformationPolicy",
	"piiEntities",
	"regexes",
	"wordPolicy",
	"customWords",
	"managedWordLists",
	"contextualGroundingPolicy",
	"invocationMetrics",
	"appliedGuardrailDetails",
	// metadata leaves
	"name",
	"type",
	"action",
	"detected",
	"confidence",
	"filterStrength",
	"threshold",
	"score",
	"guardrailProcessingLatency",
	"guardrailId",
	"guardrailVersion",
	"guardrailOrigin",
];

fn redact_assessment(value: &mut serde_json::Value) {
	let serde_json::Value::Object(map) = value else {
		*value = serde_json::Value::String("[redacted]".to_string());
		return;
	};
	for (k, v) in map.iter_mut() {
		if ASSESSMENT_METADATA_KEYS.contains(&k.as_str()) {
			filter_assessment_fields(v);
		} else {
			*v = serde_json::Value::String("[redacted]".to_string());
		}
	}
}

fn filter_assessment_fields(value: &mut serde_json::Value) {
	match value {
		serde_json::Value::Object(map) => {
			map.retain(|k, _| ASSESSMENT_METADATA_KEYS.contains(&k.as_str()));
			map.values_mut().for_each(filter_assessment_fields);
		},
		serde_json::Value::Array(arr) => arr.iter_mut().for_each(filter_assessment_fields),
		_ => {},
	}
}

impl ApplyGuardrailResponse {
	/// Guardrail-log detail for this intervention, with assessments redacted.
	pub(super) fn build_detail(&mut self, config: &BedrockGuardrails) -> GuardDetail {
		let assessments = std::mem::take(&mut self.assessments)
			.into_iter()
			.map(|mut a| {
				redact_assessment(&mut a);
				a
			})
			.collect();
		GuardDetail {
			guardrail_id: Some(config.guardrail_identifier.clone()),
			guardrail_version: Some(config.guardrail_version.clone()),
			action_reason: self.action_reason.take(),
			assessments,
		}
	}
}

impl BedrockGuardrails {
	/// User-provided policies come first so they take precedence during resolution
	/// then system TLS and implicit AWS auth are appended as fallbacks.
	pub(crate) fn build_request_policies(&self) -> Vec<BackendTrafficPolicy> {
		let mut pols: Vec<BackendTrafficPolicy> = self.policies.to_vec();
		pols.push(BackendTrafficPolicy::BackendTLS(
			crate::http::backendtls::SYSTEM_TRUST.clone(),
		));
		pols.push(BackendTrafficPolicy::backend_auth(BackendAuthKind::Aws(
			AwsAuth::Implicit {
				service_name: None,
				region: None,
				assume_role: None,
				source_credentials_cache: Default::default(),
				assume_role_cache: Default::default(),
			},
		)));
		pols
	}
}

/// Send content to the Bedrock Guardrails ApplyGuardrail API
pub async fn send(
	source: GuardrailSource,
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

	let request_body = ApplyGuardrailRequest { source, content };
	let host = strng::format!("bedrock-runtime.{}.amazonaws.com", guardrails.region);
	let path = format!(
		"/guardrail/{}/version/{}/apply",
		guardrails.guardrail_identifier, guardrails.guardrail_version
	);
	let uri = format!("https://{}{}", host, path);

	tracing::debug!(
		source = ?request_body.source,
		content_blocks = request_body.content.len(),
		uri = %uri,
		"Sending Bedrock guardrail request"
	);

	let pols = guardrails.build_request_policies();

	// AWS requires both Content-Type and Accept headers
	let mut rb = ::http::Request::builder()
		.uri(&uri)
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.header(::http::header::ACCEPT, "application/json")
		.extension(AwsRegion {
			region: guardrails.region.to_string(),
		});

	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}

	let req = rb.body(crate::http::Body::from(serde_json::to_vec(&request_body)?))?;

	let mock_be = Backend::Dynamic(
		ResourceName::new(strng::literal!("_bedrock-guardrails"), strng::literal!("")),
		None,
	);

	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_with_explicit_policies_list(with_default_timeout(req), mock_be, pols)
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
			guardrail_id = %guardrails.guardrail_identifier,
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

	if resp.is_intervened() {
		tracing::debug!(
			guardrail_id = %guardrails.guardrail_identifier,
			guardrail_version = %guardrails.guardrail_version,
			source = ?source,
			blocked = resp.is_blocked(),
			anonymized = resp.is_anonymized(),
			"Bedrock guardrail intervened"
		);
	}

	Ok(resp)
}
