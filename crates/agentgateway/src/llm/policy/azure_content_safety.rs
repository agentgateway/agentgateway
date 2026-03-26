use agent_core::strng;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::http::auth::{AzureAuth, BackendAuth};
use crate::http::jwt::Claims;
use crate::llm::RequestType;
use crate::llm::policy::AzureContentSafety;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendPolicy, ResourceName, SimpleBackend, Target};

/// Request body for the Azure Content Safety Analyze Text API
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeTextRequest {
	/// The text to be analyzed (max 10,000 Unicode characters)
	pub text: String,
	/// The categories to analyze. If empty, all categories are analyzed.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub categories: Vec<TextCategory>,
	/// The names of blocklists to check
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub blocklist_names: Vec<String>,
	/// When true, further analysis stops if a blocklist is hit
	#[serde(skip_serializing_if = "Option::is_none")]
	pub halt_on_blocklist_hit: Option<bool>,
	/// Output type: FourSeverityLevels or EightSeverityLevels
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_type: Option<AnalyzeTextOutputType>,
}

/// Output severity level granularity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalyzeTextOutputType {
	/// Severity values: 0, 2, 4, 6
	FourSeverityLevels,
	/// Severity values: 0, 1, 2, 3, 4, 5, 6, 7
	EightSeverityLevels,
}

/// Harm categories supported by Azure Content Safety
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TextCategory {
	Hate,
	SelfHarm,
	Sexual,
	Violence,
}

/// Response from the Analyze Text API
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AnalyzeTextResponse {
	/// Blocklist match results
	pub blocklists_match: Vec<TextBlocklistMatch>,
	/// Category analysis results
	pub categories_analysis: Vec<TextCategoriesAnalysis>,
}

/// A blocklist match result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TextBlocklistMatch {
	pub blocklist_name: String,
	pub blocklist_item_id: String,
	pub blocklist_item_text: String,
}

/// Category analysis result with severity score
#[derive(Debug, Clone, Deserialize)]
pub struct TextCategoriesAnalysis {
	pub category: String,
	pub severity: i32,
}

impl AnalyzeTextResponse {
	/// Returns true if any category exceeds the severity threshold,
	/// or if any blocklist was matched.
	pub fn is_blocked(&self, severity_threshold: i32) -> bool {
		if !self.blocklists_match.is_empty() {
			return true;
		}
		self.categories_analysis
			.iter()
			.any(|c| c.severity >= severity_threshold)
	}
}

/// Send a request to Azure Content Safety for request content analysis
pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	config: &AzureContentSafety,
) -> anyhow::Result<AnalyzeTextResponse> {
	let content = req
		.get_messages()
		.into_iter()
		.map(|m| m.content.to_string())
		.collect_vec()
		.join("\n");

	send_analyze_text_request(client, claims, config, &content).await
}

/// Send a request to Azure Content Safety for response content analysis
pub async fn send_response(
	content: Vec<String>,
	claims: Option<Claims>,
	client: &PolicyClient,
	config: &AzureContentSafety,
) -> anyhow::Result<AnalyzeTextResponse> {
	let combined_content = content.join("\n");
	send_analyze_text_request(client, claims, config, &combined_content).await
}

async fn send_analyze_text_request(
	client: &PolicyClient,
	claims: Option<Claims>,
	config: &AzureContentSafety,
	text: &str,
) -> anyhow::Result<AnalyzeTextResponse> {
	let request_body = AnalyzeTextRequest {
		text: text.to_string(),
		categories: Vec::new(),
		blocklist_names: config
			.blocklist_names
			.as_ref()
			.cloned()
			.unwrap_or_default(),
		halt_on_blocklist_hit: config.halt_on_blocklist_hit,
		output_type: None,
	};

	let api_version = config
		.api_version
		.as_ref()
		.map(|s| s.as_str())
		.unwrap_or("2024-09-01");

	// Azure Content Safety endpoint: {endpoint}/contentsafety/text:analyze?api-version={version}
	let endpoint = config.endpoint.as_str().trim_end_matches('/');
	let host_str = endpoint
		.strip_prefix("https://")
		.or_else(|| endpoint.strip_prefix("http://"))
		.unwrap_or(endpoint);
	let host = strng::new(host_str);

	let uri = format!(
		"https://{}/contentsafety/text:analyze?api-version={}",
		host, api_version
	);

	debug!(
		uri = %uri,
		"[Azure Content Safety] >>> Sending text analysis request"
	);

	let mut pols = vec![
		BackendPolicy::BackendTLS(crate::http::backendtls::SYSTEM_TRUST.clone()),
		// Default to implicit Azure auth
		BackendPolicy::BackendAuth(BackendAuth::Azure(AzureAuth::default())),
	];
	pols.extend(config.policies.iter().cloned());

	let mut rb = ::http::Request::builder()
		.uri(&uri)
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json");

	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}

	let req = rb.body(crate::http::Body::from(serde_json::to_vec(&request_body)?))?;

	let mock_be = SimpleBackend::Opaque(
		ResourceName::new(
			strng::literal!("_azure-content-safety"),
			strng::literal!(""),
		),
		Target::Hostname(host, 443),
	);

	let resp = client
		.call_with_explicit_policies(req, mock_be, pols)
		.await?;

	let status = resp.status();
	let lim = crate::http::response_buffer_limit(&resp);
	let (_, body) = resp.into_parts();
	let bytes = crate::http::read_body_with_limit(body, lim).await?;

	if !status.is_success() {
		let error_body = String::from_utf8_lossy(&bytes);
		warn!(
			status = %status,
			error_body = %error_body,
			endpoint = %config.endpoint,
			"Azure Content Safety API returned error"
		);
		anyhow::bail!(
			"Azure Content Safety API error: status={}, body={}",
			status,
			error_body
		);
	}

	let resp: AnalyzeTextResponse = serde_json::from_slice(&bytes)
		.map_err(|e| anyhow::anyhow!("Failed to parse Azure Content Safety response: {e}"))?;

	let threshold = config.severity_threshold.unwrap_or(2);
	if resp.is_blocked(threshold) {
		warn!(
			endpoint = %config.endpoint,
			severity_threshold = threshold,
			categories = ?resp.categories_analysis.iter().map(|c| (&c.category, c.severity)).collect::<Vec<_>>(),
			blocklist_matches = resp.blocklists_match.len(),
			"[Azure Content Safety] Content BLOCKED"
		);
	} else {
		debug!(
			endpoint = %config.endpoint,
			categories = ?resp.categories_analysis.iter().map(|c| (&c.category, c.severity)).collect::<Vec<_>>(),
			"[Azure Content Safety] Content passed safety checks"
		);
	}

	Ok(resp)
}
