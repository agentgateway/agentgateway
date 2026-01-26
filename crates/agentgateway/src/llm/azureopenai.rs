use agent_core::strng;
use agent_core::strng::Strng;

use crate::llm::RouteType;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // this is the Azure OpenAI model deployment name
	pub host: Strng, // required
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub api_version: Option<Strng>, // optional, defaults to "v1"
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("azure.openai");
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
		self.host.clone()
	}

	fn api_version(&self) -> &str {
		self.api_version.as_deref().unwrap_or("v1")
	}
}
