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

	/// Client endpoints the gateway fully parses for the model, preferring those served natively.
	pub fn preferred_endpoints(&self, model: &str, catalog: &CatalogSnapshot) -> Vec<&'static str> {
		// Endpoints are model-specific, so an unexpanded wildcard advertises none.
		let Some(provider) = self
			.provider_config
			.as_ref()
			.filter(|_| !model.contains('*'))
		else {
			return Vec::new();
		};
		let custom = matches!(provider, AIProvider::Custom(_));
		let entry = catalog.model(&self.provider, model);
		let mut endpoints = provider
			.supported_chat_formats(model, Some(catalog))
			.into_iter()
			.flat_map(|format| match format {
				ChatFormat::OpenAICompletions => &["/chat/completions"][..],
				ChatFormat::OpenAIResponses => &["/responses"],
				ChatFormat::AnthropicMessages => &["/messages"],
				ChatFormat::VertexGemini => &["/models/{model}:generateContent"],
				// Converse is only reachable by raw passthrough, so advertise the best converted formats.
				// Bedrock catalog entries lack endpoints, so only entries with modalities are chat models.
				ChatFormat::BedrockConverse => match provider {
					AIProvider::Bedrock(_) if entry.is_none_or(|m| m.modalities.output.is_empty()) => &[][..],
					AIProvider::Bedrock(p) if p.is_anthropic_model(model) => &["/messages"],
					_ => &["/chat/completions", "/responses"],
				},
			})
			.copied()
			.collect::<Vec<_>>();
		let served = entry.and_then(|m| m.endpoints.as_ref());
		// Models without catalog endpoints are chat models, unless a custom provider declares more.
		if served.is_some() || custom {
			// Vertex and Bedrock convert embeddings and reranking.
			let converted = matches!(provider, AIProvider::Vertex(_) | AIProvider::Bedrock(_));
			endpoints.extend(
				provider
					.supported_formats(model)
					.into_iter()
					.filter_map(|format| match format {
						ProviderFormat::Embeddings if !converted => Some("/embeddings"),
						// Azure forwards rerank requests to Chat Completions.
						ProviderFormat::Rerank if !converted && !matches!(provider, AIProvider::Azure(_)) => {
							Some("/rerank")
						},
						ProviderFormat::Decisions if custom => Some("/decisions"),
						ProviderFormat::SystemOne if custom => Some("/systemone"),
						ProviderFormat::AnthropicTokenCount if custom => Some("/messages/count_tokens"),
						ProviderFormat::GeminiCountTokens if custom => Some("/models/{model}:countTokens"),
						_ => None,
					}),
			);
		}
		if let Some(served) = served {
			// Image and audio requests are forwarded with their OpenAI-style paths.
			if matches!(provider, AIProvider::OpenAI(_) | AIProvider::Custom(_)) {
				endpoints.extend([
					"/images/generations",
					"/images/edits",
					"/audio/speech",
					"/audio/transcriptions",
				]);
			}
			endpoints.retain(|e| served.iter().any(|s| s == e));
		}
		endpoints.sort();
		endpoints.dedup();
		endpoints
	}
}
