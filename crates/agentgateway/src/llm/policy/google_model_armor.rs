//! Google Cloud Model Armor integration for request/response content filtering.
//!
//! This module provides integration with Google Cloud Model Armor, allowing per-request
//! content filtering similar to OpenAI's moderation endpoint. It uses GCP authentication
//! via the backend policies.

use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::http::auth::{BackendAuth, GcpAuth};
use crate::http::jwt::Claims;
use crate::json;
use crate::llm::RequestType;
use crate::llm::policy::GoogleModelArmor;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::SpanWriter;
use crate::types::agent::{BackendPolicy, ResourceName, SimpleBackend, Target};

/// User prompt data for sanitization
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UserPromptData {
	pub text: String,
}

/// Model response data for sanitization
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ModelResponseData {
	pub text: String,
}

/// Request body for sanitizeUserPrompt API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SanitizeUserPromptRequest {
	pub user_prompt_data: UserPromptData,
}

/// Request body for sanitizeModelResponse API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SanitizeModelResponseRequest {
	pub model_response_data: ModelResponseData,
}

/// Match state indicating whether a filter found a match
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchState {
	MatchFound,
	NoMatchFound,
	#[serde(other)]
	Unknown,
}

// note: model armor responses are all camelCase

/// RAI (Responsible AI) filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RaiFilterResult {
	pub match_state: Option<MatchState>,
}

/// Prompt Injection and Jailbreak filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PiAndJailbreakFilterResult {
	pub match_state: Option<MatchState>,
}

/// Malicious URI filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MaliciousUriFilterResult {
	pub match_state: Option<MatchState>,
}

/// CSAM (Child Safety) filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CsamFilterResult {
	pub match_state: Option<MatchState>,
}

/// Virus scan filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct VirusScanFilterResult {
	pub match_state: Option<MatchState>,
}

/// Inspect result within SDP filter
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct InspectResult {
	pub match_state: Option<MatchState>,
}

/// Deidentify result within SDP filter
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeidentifyResult {
	pub match_state: Option<MatchState>,
}

/// Sensitive Data Protection filter result
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SdpFilterResult {
	pub inspect_result: Option<InspectResult>,
	pub deidentify_result: Option<DeidentifyResult>,
}

/// Individual Google Model Armor filter results
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilterResultEntry {
	pub rai_filter_result: Option<RaiFilterResult>,
	pub pi_and_jailbreak_filter_result: Option<PiAndJailbreakFilterResult>,
	pub malicious_uri_filter_result: Option<MaliciousUriFilterResult>,
	pub csam_filter_result: Option<CsamFilterResult>,
	pub virus_scan_filter_result: Option<VirusScanFilterResult>,
	pub sdp_filter_result: Option<SdpFilterResult>,
}

// TODO: check if this is true for all versions?
/// Google Model Armor filter results can be either a map or a list, need to handle both
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum FilterResults {
	Map(std::collections::HashMap<String, FilterResultEntry>),
	List(Vec<FilterResultEntry>),
}

impl Default for FilterResults {
	fn default() -> Self {
		FilterResults::List(Vec::new())
	}
}

impl FilterResults {
	pub fn entries(&self) -> Vec<&FilterResultEntry> {
		match self {
			FilterResults::Map(map) => map.values().collect(),
			FilterResults::List(list) => list.iter().collect(),
		}
	}
}

/// Sanitization result from Model Armor
#[derive(Debug, Clone, Deserialize, Default)]
// model armor response is camelCase
#[serde(rename_all = "camelCase", default)]
pub struct SanitizationResult {
	pub filter_results: FilterResults,
}

/// Response from Model Armor sanitize APIs
#[derive(Debug, Clone, Deserialize, Default)]
// model armor response is camelCase
#[serde(rename_all = "camelCase", default)]
pub struct SanitizeResponse {
	pub sanitization_result: Option<SanitizationResult>,
}

impl SanitizeResponse {
	/// Returns true if any filter found a match indicating content should be blocked
	pub fn is_blocked(&self) -> bool {
		let Some(result) = &self.sanitization_result else {
			return false;
		};

		for entry in result.filter_results.entries() {
			if let Some(rai) = &entry.rai_filter_result
				&& rai.match_state == Some(MatchState::MatchFound)
			{
				return true;
			}

			if let Some(pi) = &entry.pi_and_jailbreak_filter_result
				&& pi.match_state == Some(MatchState::MatchFound)
			{
				return true;
			}

			if let Some(uri) = &entry.malicious_uri_filter_result
				&& uri.match_state == Some(MatchState::MatchFound)
			{
				return true;
			}

			if let Some(csam) = &entry.csam_filter_result
				&& csam.match_state == Some(MatchState::MatchFound)
			{
				return true;
			}

			if let Some(virus) = &entry.virus_scan_filter_result
				&& virus.match_state == Some(MatchState::MatchFound)
			{
				return true;
			}

			// Check SDP filter (both inspect and deidentify results)
			if let Some(sdp) = &entry.sdp_filter_result {
				if let Some(inspect) = &sdp.inspect_result
					&& inspect.match_state == Some(MatchState::MatchFound)
				{
					return true;
				}
				if let Some(deidentify) = &sdp.deidentify_result
					&& deidentify.match_state == Some(MatchState::MatchFound)
				{
					return true;
				}
			}
		}

		false
	}
}

/// Use Model Armor sanitizeUserPrompt API for request content
pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	model_armor: &GoogleModelArmor,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<SanitizeResponse> {
	let content = req
		.get_messages()
		.into_iter()
		.map(|m| m.content.to_string())
		.collect_vec()
		.join("\n");

	let request_body = SanitizeUserPromptRequest {
		user_prompt_data: UserPromptData { text: content },
	};

	let response = send_model_armor_request(
		client,
		claims.clone(),
		model_armor,
		"sanitizeUserPrompt",
		&request_body,
		span_writer,
	)
	.await?;

	debug!(
		template_id = %model_armor.template_id,
		is_blocked = response.is_blocked(),
		response = ?response,
		"[Model Armor] <<< Received REQUEST response from Model Armor"
	);

	if response.is_blocked() {
		warn!(
			template_id = %model_armor.template_id,
			"[Model Armor] REQUEST BLOCKED by Google Model Armor"
		);
	} else {
		debug!(
			template_id = %model_armor.template_id,
			"[Model Armor] Request passed Google Model Armor checks"
		);
	}

	Ok(response)
}

/// Use Model Armor sanitizeModelResponse API for response content
pub async fn send_response(
	content: Vec<String>,
	claims: Option<Claims>,
	client: &PolicyClient,
	model_armor: &GoogleModelArmor,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<SanitizeResponse> {
	let combined_content = content.join("\n");

	let request_body = SanitizeModelResponseRequest {
		model_response_data: ModelResponseData {
			text: combined_content,
		},
	};

	let response = send_model_armor_request(
		client,
		claims.clone(),
		model_armor,
		"sanitizeModelResponse",
		&request_body,
		span_writer,
	)
	.await?;

	debug!(
		template_id = %model_armor.template_id,
		is_blocked = response.is_blocked(),
		response = ?response,
		"[Model Armor] <<< Received RESPONSE response from Model Armor"
	);

	if response.is_blocked() {
		warn!(
			template_id = %model_armor.template_id,
			"[Model Armor] RESPONSE BLOCKED by Google Model Armor"
		);
	} else {
		debug!(
			template_id = %model_armor.template_id,
			"[Model Armor] Response passed Google Model Armor checks"
		);
	}

	Ok(response)
}

async fn send_model_armor_request<T: Serialize>(
	client: &PolicyClient,
	claims: Option<Claims>,
	model_armor: &GoogleModelArmor,
	action: &str,
	request_body: &T,
	span_writer: Option<SpanWriter>,
) -> anyhow::Result<SanitizeResponse> {
	// Use default location if not specified
	let location = model_armor
		.location
		.as_ref()
		.map(|s| s.as_str())
		.unwrap_or("us-central1");

	// Build the Model Armor API URL
	let host = strng::format!("modelarmor.{}.rep.googleapis.com", location);
	let path = format!(
		"/v1/projects/{}/locations/{}/templates/{}:{}",
		model_armor.project_id, location, model_armor.template_id, action
	);
	let uri = format!("https://{}{}", host, path);

	let mut pols = vec![
		BackendPolicy::BackendTLS(crate::http::backendtls::SYSTEM_TRUST.clone()),
		// Default to implicit GCP auth
		BackendPolicy::BackendAuth(BackendAuth::Gcp(GcpAuth::default())),
	];
	pols.extend(model_armor.policies.iter().cloned());

	let mut rb = ::http::Request::builder()
		.uri(&uri)
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json");

	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}

	let req = rb.body(crate::http::Body::from(serde_json::to_vec(request_body)?))?;

	let mock_be = SimpleBackend::Opaque(
		ResourceName::new(strng::literal!("_google-model-armor"), strng::literal!("")),
		Target::Hostname(host, 443),
	);

	let resp = client
		.call_with_explicit_policies_and_span(req, mock_be, pols, span_writer)
		.await?;

	let resp: SanitizeResponse = json::from_response_body(resp).await?;
	Ok(resp)
}
