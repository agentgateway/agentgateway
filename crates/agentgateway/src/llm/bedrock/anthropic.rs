//! Anthropic Messages API to Bedrock Converse translation provider

use agent_core::prelude::Strng;
use agent_core::strng;
use bytes::Bytes;
use chrono::Utc;
use http::{HeaderMap, HeaderValue, StatusCode};
#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

mod streaming;

use super::anthropic_translation::{self as translator, AnthropicHeaders, TranslationConfig};
use super::models::resolve_model_global;
use super::thinking::{process_thinking_request, validate_thinking_request};
use super::tools::{extract_tool_result_ids, extract_tool_use_ids, global_tool_cycle_store};
use super::types::{ConverseErrorResponse, ConverseRequest, ConverseResponse};
use crate::http::Response;
use crate::llm::anthropic_types::{self as anthropic, MessagesRequest, MessagesResponse};
use crate::llm::{AIError, LLMResponse};
use crate::telemetry::log::AsyncLog;

/// Provider configuration for direct Anthropic-to-Bedrock translation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	/// AWS region for Bedrock API calls
	pub region: Strng,

	/// Optional model override
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,

	/// Guardrail configuration (enables guardrails if specified)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,

	/// Additional model request fields
	#[serde(skip_serializing_if = "Option::is_none")]
	pub additional_model_fields: Option<serde_json::Value>,

	/// Redact thinking blocks for security/privacy (default: false)
	#[serde(default)]
	pub redact_thinking: bool,

	/// Optional list of anthropic_beta feature flags
	#[serde(default)]
	pub anthropic_beta: Option<Vec<String>>,

	/// Optional override for tool cycle TTL in seconds (default: 300)
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_cycle_ttl_secs: Option<u64>,

	/// Custom model mapping configuration
	#[serde(default)]
	pub model_map: Option<serde_json::Value>,

	/// Observability configuration
	#[serde(default)]
	pub observability: ObservabilityConfig,
}

/// Observability configuration for logging and telemetry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ObservabilityConfig {
	/// Capture full reconstructed prompt text in traces/logs
	#[serde(default)]
	pub record_prompt_text: bool,

	/// Capture assistant completion text in traces/logs
	#[serde(default)]
	pub record_response_text: bool,

	/// Maximum characters to record for prompt/response (0 = unlimited)
	#[serde(default = "default_max_log_chars")]
	pub max_log_chars: usize,

	/// Include thinking blocks content in recorded completion
	#[serde(default)]
	pub include_thinking_text: bool,

	/// Include tool input/result JSON in prompt capture
	#[serde(default)]
	pub include_tool_io_text: bool,
}

impl Default for ObservabilityConfig {
	fn default() -> Self {
		Self {
			record_prompt_text: false,
			record_response_text: false,
			max_log_chars: 10000,
			include_thinking_text: false,
			include_tool_io_text: false,
		}
	}
}

/// Default configuration functions for advanced options
fn default_max_log_chars() -> usize {
	10000
}

impl crate::llm::Provider for Provider {
	const NAME: Strng = strng::literal!("bedrock_direct");
}

impl Provider {
	/// Extract Anthropic headers from HTTP request headers
	pub fn extract_headers(headers: &HeaderMap) -> Result<AnthropicHeaders, AIError> {
		translator::extract_anthropic_headers(headers)
	}

	/// Process Anthropic Messages API request directly (no universal format)
	#[instrument(
		skip(self, anthropic_request),
		fields(
			conversation_id = conversation_id.as_deref(),
			// GenAI semantic conventions will be recorded via trait
		)
	)]
	pub async fn process_request(
		&self,
		anthropic_request: MessagesRequest,
		conversation_id: Option<String>,
	) -> Result<ConverseRequest, AIError> {
		let conversation_id =
			conversation_id.unwrap_or_else(|| format!("conv_{}", chrono::Utc::now().timestamp_millis()));

		// Validate the Anthropic request
		translator::validate_anthropic_request(&anthropic_request)?;

		// Let Bedrock handle attachment validation

		// Advanced thinking validation (always enabled)
		let thinking_result = validate_thinking_request(&anthropic_request)?;

		if !thinking_result.valid {
			if thinking_result.should_disable_thinking {
				warn!(
						conversation_id = %conversation_id,
						errors = ?thinking_result.errors,
						"Thinking validation failed, disabling thinking mode"
				);
				// In a full implementation, we would disable thinking here
			} else {
				return Err(AIError::MissingField(strng::new(&format!(
					"Thinking validation failed: {}",
					thinking_result.errors.join("; ")
				))));
			}
		}

		if !thinking_result.warnings.is_empty() {
			warn!(
					conversation_id = %conversation_id,
					warnings = ?thinking_result.warnings,
					"Thinking validation warnings"
			);
		}

		// Tool cycle validation (always enabled) - check for tool results in user messages
		// Configure tool cycle store TTL if specified
		if let Some(ttl_secs) = self.tool_cycle_ttl_secs {
			let config = super::tools::ToolCycleConfig {
				ttl_seconds: ttl_secs,
				max_active_cycles: 5000,
				enabled: true,
			};
			global_tool_cycle_store().update_config(config);
		}

		for message in &anthropic_request.messages {
			if matches!(message.role, anthropic::MessageRole::User) {
				let tool_result_ids = extract_tool_result_ids(message.content.as_blocks());
				if !tool_result_ids.is_empty() {
					// Validate tool results against pending cycles (strict enforcement)
					match global_tool_cycle_store().fulfill_partial(&conversation_id, &tool_result_ids) {
						Err(err) => {
							// Lenient enforcement - log error but allow request to proceed
							warn!(
									conversation_id = %conversation_id,
									error = %err,
									"Tool cycle validation failed, proceeding anyway"
							);
						},
						Ok(result) => {
							debug!(
									conversation_id = %conversation_id,
									complete = result.complete,
									remaining = result.remaining.len(),
									"Tool cycle validation succeeded"
							);
						},
					}
				}
			}
		}

		// Apply model override if configured
		let mut anthropic_request = anthropic_request;
		if let Some(model_override) = &self.model {
			anthropic_request.model = model_override.to_string();
		}

		// Model resolution (no capability validation - pass-through approach)
		let bedrock_model_id = resolve_model_global(&anthropic_request.model)
			.map_err(|e| AIError::MissingField(strng::new(&format!("Model resolution failed: {}", e))))?;

		debug!(
				original_model = %anthropic_request.model,
				bedrock_model_id = %bedrock_model_id,
				"Model resolution completed"
		);

		// Update the request to use the resolved Bedrock model ID for translation
		anthropic_request.model = bedrock_model_id.clone();

		// Process thinking blocks (apply redaction if configured)
		if self.redact_thinking {
			process_thinking_request(&mut anthropic_request)?;
		}

		// Create translation configuration with proper additional fields handling
		// Following the working AWS SDK implementation pattern: only send additional_model_request_fields when there are actual fields
		let mut additional_fields_map = serde_json::Map::new();

		// Add beta features to additional model fields if configured
		if let Some(beta_features) = &self.anthropic_beta {
			additional_fields_map.insert(
				"anthropic_beta".to_string(),
				serde_json::Value::Array(
					beta_features
						.iter()
						.map(|s| serde_json::Value::String(s.clone()))
						.collect(),
				),
			);
		}

		// Add any configured additional model fields
		if let Some(configured_fields) = &self.additional_model_fields {
			if let Some(obj) = configured_fields.as_object() {
				for (key, value) in obj {
					additional_fields_map.insert(key.clone(), value.clone());
				}
			}
		}

		// Performance analysis is tracked internally but not sent to Bedrock
		// Bedrock Converse API doesn't accept performance fields
		// if performance_analysis.as_ref().map(|a| a.use_latency_optimization).unwrap_or(false) {
		//     additional_fields_map.insert("performance".to_string(), serde_json::json!({
		//         "latency": "optimized"
		//     }));
		// }

		// Only include additional_model_fields if there are actual fields to send
		// This is critical: older Bedrock models reject requests with empty additional_model_request_fields
		let final_additional_fields = if additional_fields_map.is_empty() {
			None
		} else {
			Some(serde_json::Value::Object(additional_fields_map))
		};

		// Enable guardrails if configured - let Bedrock decide if the model supports them
		// If a model doesn't support guardrails, Bedrock will return an appropriate error
		let enable_guardrails = self.guardrail_identifier.is_some() && self.guardrail_version.is_some();

		let translation_config = TranslationConfig {
			aws_region: self.region.to_string(),
			enable_guardrails,
			guardrail_identifier: if enable_guardrails {
				self.guardrail_identifier.as_ref().map(|s| s.to_string())
			} else {
				None
			},
			guardrail_version: if enable_guardrails {
				self.guardrail_version.as_ref().map(|s| s.to_string())
			} else {
				None
			},
			additional_model_fields: final_additional_fields,
			// Enable prompt caching by default for performance
			enable_prompt_caching: true,
			prompt_cache_min_tokens: None,    // Use default (1024)
			prompt_cache_safety_margin: None, // Use default (76)
			prompt_cache_force: None,         // Use default (false)
			prompt_cache_include_tool_weight: true,
		};

		// Perform direct translation
		let bedrock_request =
			translator::translate_request(anthropic_request.clone(), &translation_config).await?;

		Ok(bedrock_request)
	}

	/// Process Bedrock response back to Anthropic format
	#[instrument(skip(self, bytes))]
	pub async fn process_response_direct(
		&self,
		model_id: &str,
		bytes: &Bytes,
	) -> Result<MessagesResponse, AIError> {
		// Log the actual response bytes for debugging
		let response_str = std::str::from_utf8(bytes).unwrap_or("invalid utf8");
		tracing::debug!("Raw Bedrock response: {}", response_str);

		let bedrock_response: ConverseResponse =
			serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;

		let anthropic_response = translator::translate_response(bedrock_response, model_id)?;

		// Validate the translated response
		translator::validate_anthropic_response(&anthropic_response)?;

		// Tool cycle tracking (always enabled) - extract tool use IDs from assistant response
		let tool_use_ids = extract_tool_use_ids(&anthropic_response.content);
		if !tool_use_ids.is_empty() {
			let conversation_id = format!("conv_{}", chrono::Utc::now().timestamp_millis());
			global_tool_cycle_store().insert_cycle(&conversation_id, tool_use_ids);
		}

		Ok(anthropic_response)
	}

	/// Process Bedrock error response back to Anthropic format
	pub async fn process_error_direct(
		&self,
		status_code: StatusCode,
		bytes: &Bytes,
	) -> Result<anthropic::MessagesErrorResponse, AIError> {
		// CRITICAL: Log raw error response for debugging ValidationException
		let error_str = std::str::from_utf8(bytes).unwrap_or("invalid utf8");
		tracing::error!(
			"Raw Bedrock error response ({}): {}",
			status_code,
			error_str
		);

		// Try to parse as Bedrock error response, fall back to generic error for non-Bedrock responses
		let bedrock_error = match serde_json::from_slice::<ConverseErrorResponse>(bytes) {
			Ok(error) => error,
			Err(parse_err) => {
				// CRITICAL: Log the parsing failure with details
				tracing::error!(
					"Failed to parse Bedrock error as ConverseErrorResponse: {}",
					parse_err
				);

				// If it's not a valid Bedrock error format, create a generic one
				let error_message = String::from_utf8_lossy(bytes).trim().to_string();
				let error_message = if error_message.is_empty() {
					format!("HTTP {} error", status_code.as_u16())
				} else {
					error_message
				};

				ConverseErrorResponse {
					message: error_message,
					error_type: None,
				}
			},
		};

		// Extract error type from status or response
		let error_type =
			translator::extract_bedrock_error_type(status_code.as_u16(), Some(&bedrock_error));

		let anthropic_error =
			translator::translate_error_response(bedrock_error, error_type.as_deref())?;

		Ok(anthropic_error)
	}

	/// Process streaming responses with direct Anthropic SSE output
	#[instrument(skip(self, resp, log))]
	pub async fn process_streaming(
		&self,
		log: AsyncLog<LLMResponse>,
		resp: Response,
		model_id: &str,
	) -> Response {
		// Generate message ID for the stream
		let message_id = format!("msg_{:016x}", Utc::now().timestamp_millis());
		let model = model_id.to_string();

		// Set up logging for first token timing
		let log_clone = log.clone();
		tokio::spawn(async move {
			// Simple first-token tracking - this could be enhanced with proper event correlation
			tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
			log_clone.non_atomic_mutate(|r| {
				r.first_token = Some(std::time::Instant::now());
				r.provider_model = Some(strng::new(&model));
			});
		});

		resp.map(move |body_stream| {
			// Use proper architectural separation: streaming module handles Anthropic business logic
			streaming::transform_bedrock_to_anthropic_sse(body_stream, message_id, model_id.to_string())
		})
	}

	/// Get the Bedrock host for this region
	pub fn get_host(&self) -> String {
		format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}

	/// Get the Bedrock path for the given model and streaming mode
	pub fn get_path_for_model(&self, model_id: &str, is_streaming: bool) -> String {
		translator::extract_bedrock_model_path(model_id, is_streaming)
	}

	/// Get the resolved Bedrock model ID from an Anthropic model name
	pub fn resolve_model_id(&self, anthropic_model: &str) -> Result<String, AIError> {
		// Apply model override if configured
		let model_name = if let Some(model_override) = &self.model {
			model_override.as_str()
		} else {
			anthropic_model
		};

		// Model resolution (no capability validation - pass-through approach)
		let bedrock_model_id = resolve_model_global(model_name)
			.map_err(|e| AIError::MissingField(strng::new(&format!("Model resolution failed: {}", e))))?;

		Ok(bedrock_model_id)
	}

	/// Build request headers for Bedrock API
	pub fn build_bedrock_headers(
		&self,
		anthropic_headers: &AnthropicHeaders,
	) -> Result<HeaderMap, AIError> {
		let mut headers = HeaderMap::new();

		// Required headers for Bedrock Converse API
		headers.insert("Content-Type", HeaderValue::from_static("application/json"));

		// Preserve Anthropic version as custom header for potential use
		if let Some(version) = &anthropic_headers.anthropic_version {
			headers.insert(
				"X-Anthropic-Version",
				HeaderValue::from_str(version)
					.map_err(|_| AIError::MissingField("invalid header value".into()))?,
			);
		}

		// Preserve beta features as custom header
		if let Some(beta_features) = &anthropic_headers.anthropic_beta {
			let beta_header_value = beta_features.join(",");
			headers.insert(
				"X-Anthropic-Beta",
				HeaderValue::from_str(&beta_header_value)
					.map_err(|_| AIError::MissingField("invalid header value".into()))?,
			);
		}

		Ok(headers)
	}
}

/// AWS region information for request extensions
#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

/// Extract usage information from SSE data for metrics
#[allow(dead_code)]
fn extract_usage_from_sse(sse_data: &str) -> Result<Option<anthropic::Usage>, AIError> {
	// Look for message_delta events containing usage
	if let Some(data_start) = sse_data.find("data: ") {
		let json_str = &sse_data[data_start + 6..];
		if let Some(json_end) = json_str.find('\n') {
			let json_str = &json_str[..json_end];

			if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
				if parsed.get("type").and_then(|t| t.as_str()) == Some("message_delta") {
					if let Some(delta) = parsed.get("delta") {
						if let Some(usage) = delta.get("usage") {
							return Ok(serde_json::from_value(usage.clone()).ok());
						}
					}
				}
			}
		}
	}

	Ok(None)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_header_building() {
		let provider = Provider {
			region: strng::new("us-east-1"),
			model: None,
			guardrail_identifier: None,
			guardrail_version: None,
			additional_model_fields: None,
			redact_thinking: false,
			anthropic_beta: None,
			tool_cycle_ttl_secs: None,
			model_map: None,
			observability: ObservabilityConfig::default(),
		};

		let anthropic_headers = AnthropicHeaders {
			anthropic_version: Some("2023-06-01".to_string()),
			anthropic_beta: Some(vec!["files-api-2025-04-14".to_string()]),
			conversation_id: None,
		};

		let headers = provider.build_bedrock_headers(&anthropic_headers).unwrap();

		assert!(headers.contains_key("Content-Type"));
		assert!(headers.contains_key("X-Anthropic-Version"));
		assert!(headers.contains_key("X-Anthropic-Beta"));
	}

	#[test]
	fn test_backward_compatibility_deserialization() {
		// Test that old bedrock config format can deserialize into new Provider
		let old_config_json = r#"{
            "region": "us-east-1",
            "model": "claude-3-sonnet-20240229",
            "guardrailIdentifier": "test-guardrail",
            "guardrailVersion": "1.0"
        }"#;

		let provider: Provider = serde_json::from_str(old_config_json).unwrap();

		// Verify core fields deserialize correctly
		assert_eq!(provider.region.as_str(), "us-east-1");
		assert_eq!(
			provider.model.as_deref().unwrap(),
			"claude-3-sonnet-20240229"
		);
		assert_eq!(
			provider.guardrail_identifier.as_ref().unwrap().as_str(),
			"test-guardrail"
		);
		assert_eq!(provider.guardrail_version.as_ref().unwrap().as_str(), "1.0");
	}
}
