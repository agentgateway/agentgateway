use agent_core::prelude::Strng;

use crate::llm::catalog::CatalogSnapshot;
use crate::llm::custom::ProviderFormat;
use crate::llm::model_transform::ModelTransformation;
use crate::llm::{AIProvider, ChatFormat};
use crate::{apply, cel, schema_enum, schema_ser_schema};

#[apply(schema_enum!)]
#[derive(Default)]
pub enum Discovery {
	/// Expand wildcard model names using the local model catalog.
	#[default]
	Catalog,
	/// List configured model names without expanding wildcards.
	Disabled,
}

/// Links a served model name to its model catalog entries.
#[apply(schema_ser_schema!)]
pub struct ModelDiscovery {
	pub provider: Strng,
	/// Maps catalog model IDs to served model names.
	pub transformation: ModelTransformation,
	/// The upstream model when it is fixed regardless of the served name.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// The configured provider, when known, used to advertise conversion-free client endpoints.
	#[serde(skip)]
	pub provider_config: Option<AIProvider>,
}

impl ModelDiscovery {
	/// Returns None when the upstream model cannot be derived statically from the served name.
	pub fn new(
		provider: Strng,
		model: Option<Strng>,
		transformation: Option<&cel::Expression>,
	) -> Option<Self> {
		let transformation = match transformation {
			Some(expression) => crate::llm::model_transform::reverse_model_transformation(expression)?,
			None => ModelTransformation::Identity,
		};
		Some(Self {
			provider,
			transformation,
			model,
			provider_config: None,
		})
	}

	/// Whether wildcard names can be expanded from the catalog.
	pub fn expands(&self, name: &str) -> bool {
		self.model.is_none() && name.contains('*')
	}

	/// Finds the catalog model ID backing a served name.
	pub fn catalog_model<'a>(&self, served: &str, mut ids: impl Iterator<Item = &'a str>) -> String {
		// A fixed model replaces the served name before the transformation runs.
		let target = self.model.as_deref().unwrap_or(served);
		if !matches!(self.transformation, ModelTransformation::Identity)
			&& let Some(id) = ids.find(|id| {
				self
					.transformation
					.apply(*id)
					.is_some_and(|name| name == target)
			}) {
			return id.to_owned();
		}
		// Otherwise the target is the upstream model: either there is no transformation, or it left
		// the name unchanged (e.g. stripPrefix without the prefix). The catalog resolves provider
		// aliases such as Vertex publisher prefixes.
		target.to_string()
	}

	/// Client endpoints that reach the upstream model without cross-protocol conversion.
	pub fn preferred_endpoints(&self, model: &str, catalog: &CatalogSnapshot) -> Vec<&'static str> {
		let Some(provider) = &self.provider_config else {
			return Vec::new();
		};
		let chat = provider
			.supported_chat_formats(model, Some(catalog))
			.into_iter()
			.filter_map(|format| match format {
				ChatFormat::OpenAICompletions => Some("/chat/completions"),
				ChatFormat::OpenAIResponses => Some("/responses"),
				ChatFormat::AnthropicMessages => Some("/messages"),
				ChatFormat::VertexGemini | ChatFormat::BedrockConverse => None,
			});
		let mut endpoints = match catalog
			.model(&self.provider, model)
			.and_then(|m| m.endpoints.as_ref())
		{
			None => chat.collect::<Vec<_>>(),
			Some(served) => {
				// Vertex and Bedrock convert embeddings and reranking.
				let converted = matches!(provider, AIProvider::Vertex(_) | AIProvider::Bedrock(_));
				let other = provider
					.supported_formats(model)
					.into_iter()
					.filter_map(|format| match format {
						ProviderFormat::Embeddings if !converted => Some("/embeddings"),
						ProviderFormat::Rerank if !converted => Some("/rerank"),
						_ => None,
					});
				// Image and audio requests are forwarded unmodified, matching only OpenAI's paths.
				let passthrough: &[&'static str] = if matches!(provider, AIProvider::OpenAI(_)) {
					&[
						"/images/generations",
						"/images/edits",
						"/audio/speech",
						"/audio/transcriptions",
					]
				} else {
					&[]
				};
				chat
					.chain(other)
					.chain(passthrough.iter().copied())
					.filter(|e| served.iter().any(|s| s == e))
					.collect()
			},
		};
		endpoints.sort();
		endpoints.dedup();
		endpoints
	}
}
