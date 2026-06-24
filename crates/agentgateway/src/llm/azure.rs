use agent_core::strng;
use agent_core::strng::Strng;

use crate::http::auth::azure::AzureCredentialCache;
use crate::llm::RouteType;
use crate::*;

/// The type of Azure endpoint to connect to.
#[apply(schema!)]
pub enum AzureResourceType {
	/// Azure OpenAI Service endpoint: `{resourceName}.openai.azure.com`
	OpenAI,
	/// Azure AI Foundry (project) endpoint: `{resourceName}.services.ai.azure.com`
	/// Requires `project_name` to construct paths like `/api/projects/{project}/openai/v1/...`
	#[serde(alias = "aiServices")]
	Foundry,
}

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "AzureProvider"))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// The Azure resource name used to construct the endpoint host.
	pub resource_name: Strng,
	/// The type of Azure endpoint. Determines the host suffix.
	pub resource_type: AzureResourceType,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_version: Option<Strng>,
	/// The Foundry project name, required when `resourceType` is `foundry`.
	/// Used to construct paths: `/api/projects/{projectName}/openai/v1/...`.
	/// This is distinct from `resourceName` which is used for the host.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub project_name: Option<Strng>,
	/// Per-provider credential cache, shared across requests via Arc.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub cached_cred: AzureCredentialCache,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("azure");
}

impl Provider {
	/// Returns true if `model` (or the provider's configured default model) is a Claude model.
	/// Used to select between Foundry's Anthropic-native and OpenAI-compatible endpoints.
	pub fn is_anthropic_model(&self, model: Option<&str>) -> bool {
		let effective = self.model.as_deref().or(model).unwrap_or_default();
		effective.to_ascii_lowercase().starts_with("claude")
	}

	pub fn get_path_for_model(&self, route: RouteType, model: &str) -> Strng {
		// Foundry exposes both OpenAI-compatible and Anthropic-native endpoints.
		// Route to the Anthropic-native path only for Claude models; GPT and other
		// models use the project-scoped OpenAI-compatible path.
		if matches!(self.resource_type, AzureResourceType::Foundry) {
			if self.is_anthropic_model(Some(model)) {
				if route == RouteType::Messages {
					return strng::literal!("/anthropic/v1/messages");
				}
				if route == RouteType::AnthropicTokenCount {
					return strng::literal!("/anthropic/v1/messages/count_tokens");
				}
			}
			let t = if route == RouteType::Embeddings {
				strng::literal!("embeddings")
			} else if route == RouteType::Responses {
				strng::literal!("responses")
			} else {
				strng::literal!("chat/completions")
			};
			let project = self
				.project_name
				.as_deref()
				.unwrap_or(self.resource_name.as_str());
			return strng::format!("/api/projects/{project}/openai/v1/{t}");
		}

		let t = if route == RouteType::Embeddings {
			strng::literal!("embeddings")
		} else if route == RouteType::Responses {
			strng::literal!("responses")
		} else {
			strng::literal!("chat/completions")
		};

		let api_version = self.api_version();
		if api_version == "v1" {
			strng::format!("/openai/v1/{t}")
		} else if api_version == "preview" {
			// v1 preview API
			strng::format!("/openai/v1/{t}?api-version=preview")
		} else {
			let model = self.model.as_deref().unwrap_or(model);
			strng::format!(
				"/openai/deployments/{}/{t}?api-version={}",
				model,
				api_version
			)
		}
	}

	pub fn get_host(&self) -> Strng {
		match &self.resource_type {
			AzureResourceType::OpenAI => {
				strng::format!("{}.openai.azure.com", self.resource_name)
			},
			AzureResourceType::Foundry => {
				strng::format!("{}.services.ai.azure.com", self.resource_name)
			},
		}
	}

	fn api_version(&self) -> &str {
		self.api_version.as_deref().unwrap_or("v1")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_provider(resource_name: &str, resource_type: AzureResourceType) -> Provider {
		Provider {
			model: None,
			resource_name: strng::new(resource_name),
			resource_type,
			api_version: None,
			project_name: None,
			cached_cred: AzureCredentialCache::default(),
		}
	}

	#[rstest::rstest]
	#[case::openai(AzureResourceType::OpenAI, "my-resource.openai.azure.com")]
	#[case::foundry(AzureResourceType::Foundry, "my-resource.services.ai.azure.com")]
	fn test_get_host(#[case] resource_type: AzureResourceType, #[case] expected: &str) {
		let p = make_provider("my-resource", resource_type);
		assert_eq!(p.get_host().as_str(), expected);
	}

	#[rstest::rstest]
	// Foundry + Claude model: Anthropic-native paths
	#[case::foundry_claude_messages(
		AzureResourceType::Foundry,
		RouteType::Messages,
		None,
		"claude-haiku-4-5",
		"/anthropic/v1/messages"
	)]
	#[case::foundry_claude_token_count(
		AzureResourceType::Foundry,
		RouteType::AnthropicTokenCount,
		None,
		"claude-haiku-4-5",
		"/anthropic/v1/messages/count_tokens"
	)]
	// Foundry + Claude model: completions still goes to OpenAI-compatible path
	#[case::foundry_claude_completions(
		AzureResourceType::Foundry,
		RouteType::Completions,
		None,
		"claude-haiku-4-5",
		"/api/projects/my-resource/openai/v1/chat/completions"
	)]
	// Foundry + GPT model: all routes use OpenAI-compatible path
	#[case::foundry_gpt_messages(
		AzureResourceType::Foundry,
		RouteType::Messages,
		None,
		"gpt-4o-mini",
		"/api/projects/my-resource/openai/v1/chat/completions"
	)]
	#[case::foundry_gpt_token_count(
		AzureResourceType::Foundry,
		RouteType::AnthropicTokenCount,
		None,
		"gpt-4o-mini",
		"/api/projects/my-resource/openai/v1/chat/completions"
	)]
	#[case::foundry_gpt_completions(
		AzureResourceType::Foundry,
		RouteType::Completions,
		None,
		"gpt-4o-mini",
		"/api/projects/my-resource/openai/v1/chat/completions"
	)]
	// Foundry: project name override
	#[case::foundry_project_name(
		AzureResourceType::Foundry,
		RouteType::Completions,
		Some("my-project"),
		"gpt-4o-mini",
		"/api/projects/my-project/openai/v1/chat/completions"
	)]
	// Foundry: embeddings
	#[case::foundry_embeddings(
		AzureResourceType::Foundry,
		RouteType::Embeddings,
		None,
		"text-embedding-3-small",
		"/api/projects/my-resource/openai/v1/embeddings"
	)]
	// OpenAI resource: standard v1 paths (model irrelevant)
	#[case::openai_completions(
		AzureResourceType::OpenAI,
		RouteType::Completions,
		None,
		"gpt-4o-mini",
		"/openai/v1/chat/completions"
	)]
	#[case::openai_messages(
		AzureResourceType::OpenAI,
		RouteType::Messages,
		None,
		"gpt-4o-mini",
		"/openai/v1/chat/completions"
	)]
	fn test_get_path_for_model(
		#[case] resource_type: AzureResourceType,
		#[case] route: RouteType,
		#[case] project_name: Option<&str>,
		#[case] model: &str,
		#[case] expected: &str,
	) {
		let mut p = make_provider("my-resource", resource_type);
		p.project_name = project_name.map(strng::new);
		assert_eq!(p.get_path_for_model(route, model).as_str(), expected);
	}
}
