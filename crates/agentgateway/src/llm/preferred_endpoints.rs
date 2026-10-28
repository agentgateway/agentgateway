//! Discovery of client protocols that avoid cross-protocol conversion.
use std::collections::BTreeSet;

use super::{AIProvider, ChatFormat, InputFormat, catalog, custom};

/// Client APIs that can access a configured backend without cross-protocol conversion.
/// Versionless identifiers, not URLs to append to a base URL. Model placeholders
/// identify APIs whose request paths contain the model. Streaming uses the same
/// client format. Variant order follows the serialized names for stable output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum PreferredEndpoint {
	#[serde(rename = "/chat/completions")]
	OpenAIChatCompletions,
	#[serde(rename = "/decisions")]
	OpenAIDecisions,
	#[serde(rename = "/embeddings")]
	OpenAIEmbeddings,
	#[serde(rename = "/messages")]
	AnthropicMessages,
	#[serde(rename = "/messages/count_tokens")]
	AnthropicCountTokens,
	#[serde(rename = "/model/{model}/converse")]
	BedrockConverse,
	#[serde(rename = "/models/{model}:countTokens")]
	GeminiCountTokens,
	#[serde(rename = "/models/{model}:generateContent")]
	GeminiGenerateContent,
	#[serde(rename = "/rerank")]
	CohereRerank,
	#[serde(rename = "/responses")]
	OpenAIResponses,
	#[serde(rename = "/systemone")]
	TypesafeSystemOne,
}

impl AIProvider {
	pub(super) fn preferred_endpoints(
		&self,
		model: &str,
		catalog: &catalog::CatalogSnapshot,
	) -> Vec<PreferredEndpoint> {
		use custom::ProviderFormat as F;
		let mut protocols = BTreeSet::new();
		for (input, output, protocol) in [
			(
				InputFormat::Completions,
				ChatFormat::OpenAICompletions,
				PreferredEndpoint::OpenAIChatCompletions,
			),
			(
				InputFormat::Responses,
				ChatFormat::OpenAIResponses,
				PreferredEndpoint::OpenAIResponses,
			),
			(
				InputFormat::Messages,
				ChatFormat::AnthropicMessages,
				PreferredEndpoint::AnthropicMessages,
			),
			(
				InputFormat::Gemini,
				ChatFormat::VertexGemini,
				PreferredEndpoint::GeminiGenerateContent,
			),
		] {
			if self
				.chat_translation(input, model, Some(catalog))
				.is_ok_and(|t| t.output == output)
			{
				protocols.insert(protocol);
			}
		}
		if matches!(self, Self::Bedrock(_))
			&& self
				.supported_chat_formats(model, Some(catalog))
				.contains(&ChatFormat::BedrockConverse)
		{
			protocols.insert(PreferredEndpoint::BedrockConverse);
		}

		match self {
			Self::Anthropic(_) => {
				protocols.insert(PreferredEndpoint::AnthropicCountTokens);
			},
			Self::Azure(p)
				if matches!(p.resource_type, super::azure::AzureResourceType::Foundry)
					&& p.is_anthropic_model(model) =>
			{
				protocols.insert(PreferredEndpoint::AnthropicCountTokens);
			},
			Self::Vertex(p) if p.is_anthropic_model(model) => {
				protocols.insert(PreferredEndpoint::AnthropicCountTokens);
			},
			Self::Gemini(_) => {
				protocols.insert(PreferredEndpoint::GeminiCountTokens);
			},
			Self::Vertex(p) if p.is_gemini_model(model) => {
				protocols.insert(PreferredEndpoint::GeminiCountTokens);
			},
			Self::Bedrock(p)
				if p.is_anthropic_model(model)
					&& matches!(
						p.resolve_endpoint(
							super::RouteType::AnthropicTokenCount,
							Some(model),
							Some(catalog)
						),
						super::bedrock::BedrockEndpoint::Mantle
					) =>
			{
				protocols.insert(PreferredEndpoint::AnthropicCountTokens);
			},
			_ => {},
		}

		// Vertex/Bedrock embeddings and reranking require cross-protocol conversion.
		// Realtime is not exposed by the model-serving router.
		for format in self.supported_formats(model) {
			let protocol = match format {
				F::Embeddings if !matches!(self, Self::Vertex(_) | Self::Bedrock(_)) => {
					PreferredEndpoint::OpenAIEmbeddings
				},
				F::Rerank if !matches!(self, Self::Vertex(_) | Self::Bedrock(_)) => {
					PreferredEndpoint::CohereRerank
				},
				F::AnthropicTokenCount if matches!(self, Self::Custom(_)) => {
					PreferredEndpoint::AnthropicCountTokens
				},
				F::GeminiCountTokens if matches!(self, Self::Custom(_)) => {
					PreferredEndpoint::GeminiCountTokens
				},
				F::Decisions => PreferredEndpoint::OpenAIDecisions,
				F::SystemOne => PreferredEndpoint::TypesafeSystemOne,
				_ => continue,
			};
			protocols.insert(protocol);
		}
		protocols.into_iter().collect()
	}
}

#[cfg(test)]
mod tests {
	use agent_core::strng;

	use super::*;

	fn custom(formats: &[custom::ProviderFormat]) -> AIProvider {
		AIProvider::Custom(custom::Provider {
			model_override: None,
			provider_override: None,
			formats: formats
				.iter()
				.map(|format| custom::ProviderFormatConfig {
					format: *format,
					path: None,
				})
				.collect(),
		})
	}

	#[test]
	fn custom_protocols_are_a_set_and_preserve_distinct_operations() {
		use custom::ProviderFormat::*;
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let formats = [
			Responses,
			Embeddings,
			Messages,
			Completions,
			Responses,
			Realtime,
		];
		let expected = vec![
			PreferredEndpoint::OpenAIChatCompletions,
			PreferredEndpoint::OpenAIEmbeddings,
			PreferredEndpoint::AnthropicMessages,
			PreferredEndpoint::OpenAIResponses,
		];
		assert_eq!(
			custom(&formats).preferred_endpoints("unknown", &catalog),
			expected
		);
		assert_eq!(
			custom(&formats.into_iter().rev().collect::<Vec<_>>())
				.preferred_endpoints("unknown", &catalog),
			expected
		);
		assert_eq!(
			custom(&[Embeddings]).preferred_endpoints("unknown", &catalog),
			[PreferredEndpoint::OpenAIEmbeddings]
		);
		assert_eq!(
			custom(&[Responses]).preferred_endpoints("unknown", &catalog),
			[PreferredEndpoint::OpenAIResponses]
		);
	}

	#[test]
	fn custom_non_generation_protocols_do_not_imply_generation() {
		use custom::ProviderFormat::*;
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let mut provider = custom(&[
			Embeddings,
			Rerank,
			AnthropicTokenCount,
			GeminiCountTokens,
			Decisions,
			SystemOne,
		]);
		if let AIProvider::Custom(p) = &mut provider {
			p.provider_override = Some(strng::literal!("openai"));
		}
		assert_eq!(
			provider.preferred_endpoints("unknown", &catalog),
			[
				PreferredEndpoint::OpenAIDecisions,
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::AnthropicCountTokens,
				PreferredEndpoint::GeminiCountTokens,
				PreferredEndpoint::CohereRerank,
				PreferredEndpoint::TypesafeSystemOne
			]
		);
	}

	#[test]
	fn discovery_respects_gemini_conversion_precedence() {
		use custom::ProviderFormat::*;
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let provider = custom(&[GenerateContent, Completions, Responses]);
		assert_eq!(
			provider.preferred_endpoints("unknown", &catalog),
			[
				PreferredEndpoint::GeminiGenerateContent,
				PreferredEndpoint::OpenAIResponses
			]
		);
	}

	#[test]
	fn provider_defaults_do_not_require_model_capability_evidence() {
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let provider = AIProvider::OpenAI(super::super::openai::Provider {
			model_override: None,
			moderation: None,
		});
		assert_eq!(
			provider.preferred_endpoints("unknown", &catalog),
			[
				PreferredEndpoint::OpenAIChatCompletions,
				PreferredEndpoint::OpenAIDecisions,
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::CohereRerank,
				PreferredEndpoint::OpenAIResponses
			]
		);
		let provider = AIProvider::Custom(custom::ProviderPreset::Ollama.provider(None));
		assert_eq!(
			provider.preferred_endpoints("unknown", &catalog),
			[
				PreferredEndpoint::OpenAIChatCompletions,
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::OpenAIResponses
			]
		);
	}

	#[test]
	fn builtin_native_protocols_without_capability_evidence() {
		let catalog = catalog::CatalogSnapshot::parse(
			r#"{"providers":{"gcp.gemini":{"models":{"known":{"capabilities":{"toolCall":true}}}}}}"#,
		)
		.unwrap();
		let gemini = AIProvider::Gemini(super::super::gemini::Provider {
			model_override: None,
		});
		assert_eq!(
			gemini.preferred_endpoints("known", &catalog),
			[
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::GeminiCountTokens,
				PreferredEndpoint::GeminiGenerateContent
			]
		);
		assert_eq!(
			gemini.preferred_endpoints("unknown", &catalog),
			[
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::GeminiCountTokens,
				PreferredEndpoint::GeminiGenerateContent
			]
		);
		let bedrock: AIProvider = serde_json::from_str(
			r#"{"bedrock":{"region":"us-east-1","endpointPreference":"mantleOnly"}}"#,
		)
		.unwrap();
		assert_eq!(
			bedrock.preferred_endpoints("anthropic.claude-example", &catalog),
			[
				PreferredEndpoint::AnthropicMessages,
				PreferredEndpoint::AnthropicCountTokens
			]
		);
		let runtime: AIProvider = serde_json::from_str(
			r#"{"bedrock":{"region":"us-east-1","endpointPreference":"runtimeOnly"}}"#,
		)
		.unwrap();
		assert_eq!(
			runtime.preferred_endpoints("unknown", &catalog),
			[PreferredEndpoint::BedrockConverse]
		);
	}

	#[test]
	fn native_anthropic_and_responses_only_copilot() {
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let provider = AIProvider::Anthropic(super::super::anthropic::Provider {
			model_override: None,
		});
		assert_eq!(
			provider.preferred_endpoints("claude-example", &catalog),
			[
				PreferredEndpoint::AnthropicMessages,
				PreferredEndpoint::AnthropicCountTokens
			]
		);
		let provider = AIProvider::Copilot(super::super::copilot::Provider {
			model_override: Some(strng::literal!("grok-2")),
		});
		assert_eq!(
			provider.preferred_endpoints("grok-2", &catalog),
			[
				PreferredEndpoint::OpenAIEmbeddings,
				PreferredEndpoint::CohereRerank,
				PreferredEndpoint::OpenAIResponses
			]
		);
	}

	fn assert_wire_endpoints(
		provider: &AIProvider,
		model: &str,
		catalog: &catalog::CatalogSnapshot,
		expected: &str,
	) {
		let mut expected = expected.split_whitespace().collect::<Vec<_>>();
		expected.sort_unstable();
		assert_eq!(
			serde_json::to_value(provider.preferred_endpoints(model, catalog)).unwrap(),
			serde_json::json!(expected),
			"provider={} model={model}",
			provider.provider(),
		);
	}

	#[rstest::rstest]
	#[case::cohere(
		custom::ProviderPreset::Cohere,
		"/chat/completions /embeddings /rerank"
	)]
	#[case::ollama(
		custom::ProviderPreset::Ollama,
		"/chat/completions /responses /embeddings"
	)]
	#[case::baseten(custom::ProviderPreset::Baseten, "/chat/completions /messages")]
	#[case::cerebras(custom::ProviderPreset::Cerebras, "/chat/completions")]
	#[case::deepinfra(
		custom::ProviderPreset::Deepinfra,
		"/chat/completions /messages /embeddings"
	)]
	#[case::deepseek(
		custom::ProviderPreset::Deepseek,
		"/chat/completions /messages /responses"
	)]
	#[case::groq(custom::ProviderPreset::Groq, "/chat/completions /responses")]
	#[case::huggingface(custom::ProviderPreset::Huggingface, "/chat/completions /responses")]
	#[case::mistral(custom::ProviderPreset::Mistral, "/chat/completions /embeddings")]
	#[case::openrouter(
		custom::ProviderPreset::Openrouter,
		"/chat/completions /messages /responses /embeddings /rerank"
	)]
	#[case::togetherai(
		custom::ProviderPreset::Togetherai,
		"/chat/completions /embeddings /rerank"
	)]
	#[case::xai(custom::ProviderPreset::XAI, "/chat/completions /responses")]
	#[case::fireworks(
		custom::ProviderPreset::Fireworks,
		"/chat/completions /messages /responses /embeddings /rerank"
	)]
	#[case::meta(custom::ProviderPreset::Meta, "/chat/completions /messages /responses")]
	#[case::perplexity(custom::ProviderPreset::Perplexity, "/responses")]
	#[case::typesafe(custom::ProviderPreset::Typesafe, "/systemone")]
	fn all_builtin_presets_use_canonical_client_endpoints(
		#[case] preset: custom::ProviderPreset,
		#[case] expected: &str,
	) {
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		assert_wire_endpoints(
			&AIProvider::Custom(preset.provider(None)),
			"uncatalogued",
			&catalog,
			expected,
		);
	}

	#[rstest::rstest]
	#[case::completions(custom::ProviderFormat::Completions, "/chat/completions")]
	#[case::responses(custom::ProviderFormat::Responses, "/responses")]
	#[case::messages(custom::ProviderFormat::Messages, "/messages")]
	#[case::embeddings(custom::ProviderFormat::Embeddings, "/embeddings")]
	#[case::rerank(custom::ProviderFormat::Rerank, "/rerank")]
	#[case::anthropic_count(custom::ProviderFormat::AnthropicTokenCount, "/messages/count_tokens")]
	#[case::gemini(
		custom::ProviderFormat::GenerateContent,
		"/models/{model}:generateContent"
	)]
	#[case::gemini_count(
		custom::ProviderFormat::GeminiCountTokens,
		"/models/{model}:countTokens"
	)]
	#[case::decisions(custom::ProviderFormat::Decisions, "/decisions")]
	#[case::systemone(custom::ProviderFormat::SystemOne, "/systemone")]
	#[case::realtime_is_deferred(custom::ProviderFormat::Realtime, "")]
	fn every_custom_format_has_a_wire_mapping(
		#[case] format: custom::ProviderFormat,
		#[case] expected: &str,
	) {
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let mut provider = custom(&[format]);
		if let AIProvider::Custom(p) = &mut provider {
			// Upstream path overrides must not leak into client endpoint identifiers.
			p.formats[0].path = Some(strng::literal!("/vendor/specific/path"));
		}
		assert_wire_endpoints(&provider, "custom-model", &catalog, expected);
	}

	#[test]
	fn dedicated_providers_and_model_families_have_correct_wire_endpoints() {
		let catalog = catalog::CatalogSnapshot::parse(r#"{"providers":{}}"#).unwrap();
		let cases = [
			(
				r#"{"openAI":{}}"#,
				"uncatalogued",
				"/chat/completions /responses /embeddings /rerank /decisions",
			),
			(
				r#"{"anthropic":{}}"#,
				"claude-sonnet-4",
				"/messages /messages/count_tokens",
			),
			(
				r#"{"gemini":{}}"#,
				"gemini-2.5-pro",
				"/models/{model}:generateContent /models/{model}:countTokens /embeddings",
			),
			(
				r#"{"azure":{"resourceName":"example","resourceType":"openAI"}}"#,
				"deployment",
				"/chat/completions /responses /embeddings /rerank",
			),
			(
				r#"{"azure":{"resourceName":"example","resourceType":"foundry"}}"#,
				"gpt-4o",
				"/chat/completions /responses /embeddings /rerank /systemone",
			),
			(
				r#"{"azure":{"resourceName":"example","resourceType":"foundry"}}"#,
				"claude-sonnet-4",
				"/messages /messages/count_tokens /embeddings /rerank /systemone",
			),
			(
				r#"{"vertex":{"projectId":"example"}}"#,
				"gemini-2.5-pro",
				"/models/{model}:generateContent /models/{model}:countTokens",
			),
			(
				r#"{"vertex":{"projectId":"example"}}"#,
				"publishers/google/models/gemini-2.5-pro",
				"/models/{model}:generateContent /models/{model}:countTokens",
			),
			(
				r#"{"vertex":{"projectId":"example"}}"#,
				"claude-sonnet-4",
				"/messages /messages/count_tokens",
			),
			(
				r#"{"vertex":{"projectId":"example"}}"#,
				"publishers/anthropic/models/claude-sonnet-4",
				"/messages /messages/count_tokens",
			),
			(
				r#"{"vertex":{"projectId":"example"}}"#,
				"other-model",
				"/chat/completions",
			),
			(
				r#"{"bedrock":{"region":"us-east-1","endpointPreference":"runtimeOnly"}}"#,
				"anthropic.claude-sonnet-4",
				"/model/{model}/converse",
			),
			(
				r#"{"bedrock":{"region":"us-east-1","endpointPreference":"mantleOnly"}}"#,
				"anthropic.claude-sonnet-4",
				"/messages /messages/count_tokens",
			),
			(
				r#"{"bedrock":{"region":"us-east-1","endpointPreference":"mantleOnly"}}"#,
				"openai.gpt-oss",
				"/chat/completions /responses",
			),
			(
				r#"{"bedrock":{"region":"us-east-1","endpointPreference":"mantleOnly"}}"#,
				"other-model",
				"/chat/completions",
			),
			(
				r#"{"copilot":{}}"#,
				"claude-sonnet-4",
				"/chat/completions /messages",
			),
			(
				r#"{"copilot":{}}"#,
				"grok-4.6",
				"/responses /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"mai-model",
				"/responses /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"gemini-2.5-pro",
				"/chat/completions /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"gpt-4o",
				"/chat/completions /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"gpt-5.4",
				"/chat/completions /responses /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"gpt-5-mini",
				"/chat/completions /responses /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"gpt-other",
				"/responses /embeddings /rerank",
			),
			(
				r#"{"copilot":{}}"#,
				"unknown",
				"/chat/completions /embeddings /rerank",
			),
		];
		for (config, model, expected) in cases {
			let provider: AIProvider = serde_json::from_str(config).unwrap();
			assert_wire_endpoints(&provider, model, &catalog, expected);
		}
	}

	#[test]
	fn existing_catalog_format_and_endpoint_rules_are_respected() {
		let catalog = catalog::CatalogSnapshot::parse(
			r#"{"providers":{"copilot":{"models":{
			"grok-4.6":{"tags":["openai_completions"]}
		}},"aws.bedrock":{"models":{
			"mantle-model":{"tags":["mantle","openai_responses"]},
			"runtime-model":{"tags":["runtime","openai_responses"]}
		}}}}"#,
		)
		.unwrap();
		let copilot: AIProvider = serde_json::from_str(r#"{"copilot":{}}"#).unwrap();
		assert_wire_endpoints(
			&copilot,
			"grok-4.6",
			&catalog,
			"/chat/completions /embeddings /rerank",
		);
		let bedrock: AIProvider = serde_json::from_str(
			r#"{"bedrock":{"region":"us-east-1","endpointPreference":"runtimePreferred"}}"#,
		)
		.unwrap();
		assert_wire_endpoints(&bedrock, "mantle-model", &catalog, "/responses");
		assert_wire_endpoints(
			&bedrock,
			"runtime-model",
			&catalog,
			"/model/{model}/converse",
		);
	}
}
