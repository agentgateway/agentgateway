use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::http::auth::{AwsAuth, BackendAuth};
use crate::http::jwt::Claims;
use crate::llm::RequestType;
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

	/// The decision the guardrail would have enforced, as a stable string:
	/// `BLOCKED`, `ANONYMIZED`, or `NONE`. Used for audit-mode logging/metrics.
	pub fn would_action(&self) -> &'static str {
		if self.is_blocked() {
			"BLOCKED"
		} else if self.is_anonymized() {
			"ANONYMIZED"
		} else {
			"NONE"
		}
	}

	/// Emit a structured, ingestible record of an audit-mode evaluation. The
	/// `assessments` array carries the per-filter findings (category, confidence,
	/// `detected`) as AWS returns them, with content-bearing fields redacted (see
	/// `redacted_assessments`). Downstream log pipelines (e.g. a CloudWatch
	/// subscription into a data lake) can parse the metadata. No prompt or
	/// completion text and no matched sensitive strings are logged.
	pub fn log_audit(&self, guardrail_id: &str, guardrail_version: &str, source: GuardrailSource) {
		let assessments = serde_json::to_string(&self.redacted_assessments()).unwrap_or_default();
		tracing::info!(
			target: "agentgateway::guardrail::audit",
			guardrail_id = %guardrail_id,
			guardrail_version = %guardrail_version,
			source = ?source,
			would_action = %self.would_action(),
			assessments = %assessments,
			"bedrock guardrail audit evaluation"
		);
	}

	/// Assessments with content-bearing fields stripped, safe to log.
	///
	/// Bedrock guardrail assessments echo the matched content in several places:
	/// `sensitiveInformationPolicy.piiEntities[].match` and `...regexes[].{match,regex}`
	/// carry raw matched PII, `wordPolicy.customWords[].match` echoes the matched
	/// word, and AWS can add new content-bearing fields in future API versions.
	/// Logging these verbatim would leak the very sensitive data the guardrail
	/// exists to catch, even though the prompt/completion itself is not logged.
	///
	/// A denylist (drop keys named `match`) is not safe here: it silently passes
	/// through any content-bearing field AWS adds later. This uses the inverse — an
	/// allowlist of the structural metadata keys that audit mode exists to surface
	/// (`type`, `action`, `confidence`, `detected`, `name`, `filterStrength`,
	/// `threshold`, `score`, and the policy/filter container keys). Every other key,
	/// known or not, is dropped. New AWS fields are excluded by default (fail
	/// closed), and only scalar values survive under a leaf key, so no free-form
	/// text can ride through under an allowed name.
	pub(crate) fn redacted_assessments(&self) -> Vec<serde_json::Value> {
		self.assessments.iter().map(Self::redact_value).collect()
	}

	/// Structural metadata keys that carry no user content and are safe to log.
	/// Container keys (whose values are objects/arrays we recurse into) and leaf
	/// metadata keys (whose scalar values describe the finding, not the matched
	/// content) both live here; anything absent is dropped.
	const SAFE_ASSESSMENT_KEYS: &'static [&'static str] = &[
		// Policy containers
		"topicPolicy",
		"contentPolicy",
		"wordPolicy",
		"sensitiveInformationPolicy",
		"contextualGroundingPolicy",
		// Per-policy collections
		"topics",
		"filters",
		"customWords",
		"managedWordLists",
		"piiEntities",
		"regexes",
		// Leaf metadata (no matched content). `name` is the operator-configured
		// topic/regex label (e.g. "Finance"), not user content.
		"type",
		"action",
		"confidence",
		"detected",
		"name",
		"filterStrength",
		"threshold",
		"score",
	];

	/// Recursively keep only allowlisted, content-free keys from a guardrail
	/// assessment value. Unknown keys are dropped rather than passed through, so a
	/// content-bearing field AWS adds later cannot leak. A leaf metadata key
	/// (e.g. `type`, `name`) keeps only a scalar value; if AWS ever nests text
	/// under such a key, the object/array is dropped instead of recursed into.
	fn redact_value(value: &serde_json::Value) -> serde_json::Value {
		match value {
			serde_json::Value::Object(map) => serde_json::Value::Object(
				map
					.iter()
					.filter(|(k, _)| {
						Self::SAFE_ASSESSMENT_KEYS
							.iter()
							.any(|safe| k.eq_ignore_ascii_case(safe))
					})
					.map(|(k, v)| (k.clone(), Self::redact_value(v)))
					.collect(),
			),
			serde_json::Value::Array(arr) => {
				serde_json::Value::Array(arr.iter().map(Self::redact_value).collect())
			},
			other => other.clone(),
		}
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

impl BedrockGuardrails {
	/// User-provided policies come first so they take precedence during resolution
	/// then system TLS and implicit AWS auth are appended as fallbacks.
	pub(crate) fn build_request_policies(&self) -> Vec<BackendTrafficPolicy> {
		let mut pols: Vec<BackendTrafficPolicy> = self.policies.to_vec();
		pols.push(BackendTrafficPolicy::BackendTLS(
			crate::http::backendtls::SYSTEM_TRUST.clone(),
		));
		pols.push(BackendTrafficPolicy::BackendAuth(BackendAuth::Aws(
			AwsAuth::Implicit {
				service_name: None,
				assume_role: None,
				source_credentials_cache: Default::default(),
				assume_role_cache: Default::default(),
			},
		)));
		pols
	}
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
	let request_body = ApplyGuardrailRequest { source, content };
	let host = strng::format!("bedrock-runtime.{}.amazonaws.com", guardrails.region);
	let path = format!(
		"/guardrail/{}/version/{}/apply",
		guardrails.guardrail_identifier, guardrails.guardrail_version
	);
	let uri = format!("https://{}{}", host, path);

	tracing::debug!(
		request_body = %serde_json::to_string_pretty(&request_body).unwrap_or_default(),
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
		(),
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

	if resp.is_blocked() {
		tracing::debug!(
			guardrail_id = %guardrails.guardrail_identifier,
			guardrail_version = %guardrails.guardrail_version,
			source = ?source,
			"Bedrock guardrail blocked content"
		);
	} else if resp.is_anonymized() {
		tracing::debug!(
			guardrail_id = %guardrails.guardrail_identifier,
			guardrail_version = %guardrails.guardrail_version,
			source = ?source,
			"Bedrock guardrail anonymized content"
		);
	}

	Ok(resp)
}
