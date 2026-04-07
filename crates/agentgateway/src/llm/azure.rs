use agent_core::strng;
use agent_core::strng::Strng;

use crate::http::auth::AzureCredentialCache;
use crate::llm::RouteType;
use crate::*;

/// The type of Azure endpoint to connect to.
#[apply(schema!)]
pub enum AzureResourceType {
	/// Azure OpenAI Service endpoint: `{resourceName}.openai.azure.com`
	#[serde(alias = "azureOpenAI")]
	OpenAI,
	/// Azure AI Foundry (project) endpoint: `{resourceName}.services.ai.azure.com`
	/// Requires `project_name` to construct paths like `/api/projects/{project}/openai/v1/...`
	#[serde(alias = "aiServices")]
	Foundry,
}

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// The Azure resource name used to construct the endpoint host.
	pub resource_name: Strng,
	/// The type of Azure endpoint. Determines the host suffix.
	pub resource_type: AzureResourceType,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_version: Option<Strng>,
	/// The Foundry project name, required when `resource_type` is `Foundry`.
	/// Used to construct paths: `/api/projects/{project_name}/openai/v1/...`.
	/// This is distinct from `resource_name` which is used for the host.
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
	pub fn get_path_for_model(&self, route: RouteType, model: &str) -> Strng {
		let t = if route == RouteType::Embeddings {
			strng::literal!("embeddings")
		} else if route == RouteType::Responses {
			strng::literal!("responses")
		} else {
			strng::literal!("chat/completions")
		};

		// Foundry uses the project path prefix, no api-version needed.
		if matches!(self.resource_type, AzureResourceType::Foundry) {
			let project = self.project_name.as_deref().unwrap_or(self.resource_name.as_str());
			return strng::format!("/api/projects/{project}/openai/v1/{t}");
		}

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
				strng::format!("{}-resource.services.ai.azure.com", self.resource_name)
			},
		}
	}

	/// Parse a full host string back into (resource_name, resource_type).
	/// Used for backward compatibility with XDS/proto which stores the full host.
	pub fn parse_host(host: &str) -> (Strng, AzureResourceType) {
		if let Some(name) = host.strip_suffix(".openai.azure.com") {
			(strng::new(name), AzureResourceType::OpenAI)
		} else if let Some(name) = host.strip_suffix(".services.ai.azure.com") {
			(strng::new(name), AzureResourceType::Foundry)
		} else {
			// Fallback: treat the whole host as the resource name with OpenAI type
			(strng::new(host), AzureResourceType::OpenAI)
		}
	}

	fn api_version(&self) -> &str {
		self.api_version.as_deref().unwrap_or("v1")
	}
}
