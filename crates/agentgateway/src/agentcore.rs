use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCoreConfig {
	pub agent_runtime_arn: String,
	pub qualifier: Option<String>,
	pub region: String,
	pub account_id: String,
}

impl AgentCoreConfig {
	pub fn new(arn: String, qualifier: Option<String>) -> anyhow::Result<Self> {
		// arn:aws:bedrock-agentcore:{region}:{accountId}:runtime/{runtimeId}
		let parts: Vec<&str> = arn.splitn(6, ':').collect();
		anyhow::ensure!(parts.len() >= 5, "invalid AgentCore ARN: {}", arn);
		anyhow::ensure!(
			parts.get(2) == Some(&"bedrock-agentcore"),
			"invalid AgentCore ARN (expected service bedrock-agentcore): {}",
			arn
		);
		Ok(Self {
			region: parts[3].to_string(),
			account_id: parts[4].to_string(),
			agent_runtime_arn: arn,
			qualifier,
		})
	}

	pub fn get_host(&self) -> String {
		format!("bedrock-agentcore.{}.amazonaws.com", self.region)
	}

	pub fn get_path(&self) -> String {
		let encoded = utf8_percent_encode(&self.agent_runtime_arn, NON_ALPHANUMERIC);
		match &self.qualifier {
			Some(q) => format!("/runtimes/{encoded}/invocations?qualifier={q}"),
			None => format!("/runtimes/{encoded}/invocations"),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_valid_arn_parsing() {
		let config = AgentCoreConfig::new(
			"arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string(),
			None,
		)
		.unwrap();
		assert_eq!(config.region, "us-east-1");
		assert_eq!(config.account_id, "123456789012");
	}

	#[test]
	fn test_invalid_arn_too_few_parts() {
		let err = AgentCoreConfig::new("arn:aws:bedrock-agentcore".to_string(), None).unwrap_err();
		assert!(err.to_string().contains("invalid AgentCore ARN"));
	}

	#[test]
	fn test_wrong_service() {
		let err = AgentCoreConfig::new(
			"arn:aws:bedrock:us-east-1:123456789012:runtime/abc123".to_string(),
			None,
		)
		.unwrap_err();
		assert!(
			err
				.to_string()
				.contains("expected service bedrock-agentcore")
		);
	}

	#[test]
	fn test_get_host() {
		let config = AgentCoreConfig::new(
			"arn:aws:bedrock-agentcore:us-west-2:123456789012:runtime/xyz".to_string(),
			None,
		)
		.unwrap();
		assert_eq!(
			config.get_host(),
			"bedrock-agentcore.us-west-2.amazonaws.com"
		);
	}

	#[test]
	fn test_get_path_without_qualifier() {
		let config = AgentCoreConfig::new(
			"arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string(),
			None,
		)
		.unwrap();
		let path = config.get_path();
		assert!(path.starts_with("/runtimes/"));
		assert!(path.ends_with("/invocations"));
		assert!(!path.contains("qualifier"));
	}

	#[test]
	fn test_get_path_with_qualifier() {
		let config = AgentCoreConfig::new(
			"arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string(),
			Some("v1".to_string()),
		)
		.unwrap();
		let path = config.get_path();
		assert!(path.starts_with("/runtimes/"));
		assert!(path.contains("qualifier=v1"));
	}
}
