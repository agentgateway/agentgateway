use agent_core::prelude::Strng;
use agent_core::strng;

use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("aws.bedrock");
}

impl Provider {
	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
	) -> Strng {
		let model = self.model.as_deref().unwrap_or(model);
		match route_type {
			super::RouteType::AnthropicTokenCount => strng::format!("/model/{model}/count-tokens"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self) -> Strng {
		strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
	}
}

#[cfg(test)]
mod tests {
	use ::http::HeaderMap;
	use serde_json::json;

	use super::*;

	#[test]
	fn test_metadata_from_header() {
		let provider = Provider {
			model: None,
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		// Simulate transformation CEL setting x-bedrock-metadata header
		let mut headers = HeaderMap::new();
		headers.insert(
			"x-bedrock-metadata",
			r#"{"user_id": "user123", "department": "engineering"}"#
				.parse()
				.unwrap(),
		);

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3-sonnet".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "Hello".to_string(),
					citations: None,
					cache_control: None,
				})],
			}],
			max_tokens: 100,
			metadata: None,
			system: None,
			stop_sequences: vec![],
			stream: false,
			temperature: None,
			top_k: None,
			top_p: None,
			tools: None,
			tool_choice: None,
			thinking: None,
		};

		let out = translate_request_messages(req, &provider, Some(&headers)).unwrap();
		let metadata = out.request_metadata.unwrap();

		assert_eq!(metadata.get("user_id"), Some(&"user123".to_string()));
		assert_eq!(metadata.get("department"), Some(&"engineering".to_string()));
	}

	#[test]
	fn test_translate_request_messages_maps_top_k_from_typed() {
		let provider = Provider {
			model: Some(strng::new("anthropic.claude-3")),
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
		};

		let req = anthropic::MessagesRequest {
			model: "anthropic.claude-3".to_string(),
			messages: vec![anthropic::Message {
				role: anthropic::Role::User,
				content: vec![anthropic::ContentBlock::Text(anthropic::ContentTextBlock {
					text: "hello".to_string(),
					citations: None,
					cache_control: None,
				})],
			}],
			system: None,
			max_tokens: 256,
			stop_sequences: vec![],
			stream: false,
			temperature: Some(0.7),
			top_p: Some(0.9),
			top_k: Some(7),
			tools: None,
			tool_choice: None,
			metadata: None,
			thinking: None,
		};

		let out = translate_request_messages(req, &provider, None).unwrap();
		let inf = out.inference_config.unwrap();
		assert_eq!(inf.top_k, Some(7));
	}

	#[test]
	fn test_extract_beta_headers_variants() {
		let headers = HeaderMap::new();
		assert!(extract_beta_headers(&headers).unwrap().is_none());

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			"prompt-caching-2024-07-31".parse().unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![json!("prompt-caching-2024-07-31")]
		);

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			"cache-control-2024-08-15,computer-use-2024-10-22"
				.parse()
				.unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![
				json!("cache-control-2024-08-15"),
				json!("computer-use-2024-10-22"),
			]
		);

		let mut headers = HeaderMap::new();
		headers.insert(
			"anthropic-beta",
			" cache-control-2024-08-15 , computer-use-2024-10-22 "
				.parse()
				.unwrap(),
		);
		assert_eq!(
			extract_beta_headers(&headers).unwrap().unwrap(),
			vec![
				json!("cache-control-2024-08-15"),
				json!("computer-use-2024-10-22"),
			]
		);

		let mut headers = HeaderMap::new();
		headers.append(
			"anthropic-beta",
			"cache-control-2024-08-15".parse().unwrap(),
		);
		headers.append("anthropic-beta", "computer-use-2024-10-22".parse().unwrap());
		let mut beta_features = extract_beta_headers(&headers)
			.unwrap()
			.unwrap()
			.into_iter()
			.map(|v| v.as_str().unwrap().to_string())
			.collect::<Vec<_>>();
		beta_features.sort();
		assert_eq!(
			beta_features,
			vec![
				"cache-control-2024-08-15".to_string(),
				"computer-use-2024-10-22".to_string(),
			]
		);
	}
}
