#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsBackendConfig {
	#[serde(flatten)]
	pub service: AwsService,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AwsService {
	AgentCore(crate::agentcore::AgentCoreConfig),
}

impl AwsBackendConfig {
	pub fn region(&self) -> &str {
		match &self.service {
			AwsService::AgentCore(c) => &c.region,
		}
	}

	pub fn service_name(&self) -> &'static str {
		match &self.service {
			AwsService::AgentCore(_) => "bedrock-agentcore",
		}
	}

	pub fn get_host(&self) -> String {
		match &self.service {
			AwsService::AgentCore(c) => c.get_host(),
		}
	}

	pub fn get_path(&self) -> String {
		match &self.service {
			AwsService::AgentCore(c) => c.get_path(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_aws_backend_config_delegates_to_agentcore() {
		let agentcore_config = crate::agentcore::AgentCoreConfig::new(
			"arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string(),
			None,
		)
		.unwrap();
		let config = AwsBackendConfig {
			service: AwsService::AgentCore(agentcore_config),
		};

		assert_eq!(config.region(), "us-east-1");
		assert_eq!(config.service_name(), "bedrock-agentcore");
		assert_eq!(
			config.get_host(),
			"bedrock-agentcore.us-east-1.amazonaws.com"
		);
		assert!(config.get_path().starts_with("/runtimes/"));
		assert!(config.get_path().ends_with("/invocations"));
	}
}
