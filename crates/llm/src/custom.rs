use agent_core::prelude::Strng;
use agent_core::strng;

use crate::{InputFormat, RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CustomProvider"))]
pub struct Provider {
	/// Model ID to send to the provider, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// Provider identity for cost-catalog lookup and telemetry. Built-in named providers
	/// (cohere, mistral, ...) set this so their cost resolves under the right catalog key;
	/// a bare custom provider may set it to match a catalog entry. Falls back to "custom".
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub provider_override: Option<Strng>,
	/// Supported API payload formats and optional path overrides for this provider.
	pub formats: Vec<ProviderFormatConfig>,
}

impl Provider {
	pub fn supports(&self, format: ProviderFormat) -> bool {
		self
			.formats
			.iter()
			.any(|supported| supported.format == format)
	}

	pub fn path_for(&self, format: ProviderFormat) -> Option<&str> {
		self
			.formats
			.iter()
			.find(|supported| supported.format == format)
			.and_then(|supported| supported.path.as_deref())
	}

	pub fn path_for_route(&self, route_type: RouteType) -> Option<&str> {
		ProviderFormat::from_route_type(route_type).and_then(|format| self.path_for(format))
	}
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("custom");
}

/// A named custom-provider configuration maintained by agentgateway.
///
/// These presets deliberately live beside `Provider`: both standalone and xDS
/// configuration expand them here, keeping endpoint and format behavior alike.
#[apply(schema!)]
#[derive(Copy, PartialEq, Eq)]
pub enum ProviderPreset {
	Cohere,
	Ollama,
	Baseten,
	Cerebras,
	Deepinfra,
	Deepseek,
	Groq,
	Huggingface,
	Mistral,
	Openrouter,
	Togetherai,
	#[serde(rename = "xai")]
	XAI,
	Fireworks,
}

impl ProviderPreset {
	pub const fn base_url(self) -> &'static str {
		match self {
			Self::Cohere => "https://api.cohere.ai",
			Self::Ollama => "http://localhost:11434/v1",
			Self::Baseten => "https://inference.baseten.co/v1",
			Self::Cerebras => "https://api.cerebras.ai/v1",
			Self::Deepinfra => "https://api.deepinfra.com/v1/openai",
			Self::Deepseek => "https://api.deepseek.com/v1",
			Self::Groq => "https://api.groq.com/openai/v1",
			Self::Huggingface => "https://router.huggingface.co/v1",
			Self::Mistral => "https://api.mistral.ai/v1",
			Self::Openrouter => "https://openrouter.ai/api/v1",
			Self::Togetherai => "https://api.together.xyz/v1",
			Self::XAI => "https://api.x.ai/v1",
			Self::Fireworks => "https://api.fireworks.ai/inference/v1",
		}
	}

	pub fn provider(self, model: Option<Strng>) -> Provider {
		use ProviderFormat::*;
		let format = |format: ProviderFormat, path: Option<&str>| ProviderFormatConfig {
			format,
			path: path.map(strng::new),
		};
		let (provider_override, formats) = match self {
			Self::Cohere => (
				"cohere",
				vec![
					format(Completions, Some("/compatibility/v1/chat/completions")),
					format(Embeddings, Some("/compatibility/v1/embeddings")),
					format(Rerank, Some("/v2/rerank")),
				],
			),
			Self::Ollama => (
				"ollama",
				vec![
					format(Completions, None),
					format(Responses, None),
					format(Embeddings, None),
				],
			),
			Self::Baseten => (
				"baseten",
				vec![format(Completions, None), format(Messages, None)],
			),
			Self::Cerebras => ("cerebras", vec![format(Completions, None)]),
			Self::Deepinfra => (
				"deepinfra",
				vec![
					format(Completions, None),
					format(Messages, Some("/anthropic/v1/messages")),
					format(Embeddings, None),
				],
			),
			Self::Deepseek => (
				"deepseek",
				vec![
					format(Completions, None),
					format(Messages, Some("/anthropic/v1/messages")),
				],
			),
			Self::Groq => (
				"groq",
				vec![format(Completions, None), format(Responses, None)],
			),
			Self::Huggingface => (
				"huggingface",
				vec![format(Completions, None), format(Responses, None)],
			),
			Self::Mistral => (
				"mistral",
				vec![format(Completions, None), format(Embeddings, None)],
			),
			Self::Openrouter => (
				"openrouter",
				vec![
					format(Completions, None),
					format(Messages, None),
					format(Responses, None),
					format(Embeddings, None),
					format(Rerank, None),
				],
			),
			Self::Togetherai => (
				"togetherai",
				vec![
					format(Completions, None),
					format(Embeddings, None),
					format(Rerank, None),
				],
			),
			Self::XAI => (
				"xai",
				vec![
					format(Completions, None),
					format(Responses, None),
					format(Realtime, None),
				],
			),
			Self::Fireworks => (
				"fireworks",
				vec![
					format(Completions, None),
					format(Messages, None),
					format(Responses, None),
					format(Embeddings, None),
					format(Rerank, None),
				],
			),
		};
		Provider {
			model,
			provider_override: Some(strng::new(provider_override)),
			formats,
		}
	}
}

#[apply(schema!)]
pub struct ProviderFormatConfig {
	/// Upstream API shape this custom provider says it accepts.
	#[serde(rename = "type")]
	pub format: ProviderFormat,
	/// Optional path override for this specific upstream format.
	pub path: Option<Strng>,
}

/// A custom provider's advertised upstream wire format.
///
/// Unlike `InputFormat`, this describes what the backend accepts, not what the
/// client sent. Unlike `RouteType`, it is only for LLM payload endpoints that
/// can be converted or passed through; generic routes such as models,
/// passthrough, and detect do not have a `ProviderFormat`.
#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderFormat {
	Completions,
	Messages,
	Responses,
	Embeddings,
	AnthropicTokenCount,
	Realtime,
	Rerank,
}

impl ProviderFormat {
	pub fn from_route_type(route_type: RouteType) -> Option<Self> {
		Some(match route_type {
			RouteType::Completions => Self::Completions,
			RouteType::Messages => Self::Messages,
			RouteType::Responses => Self::Responses,
			RouteType::Embeddings => Self::Embeddings,
			RouteType::AnthropicTokenCount => Self::AnthropicTokenCount,
			RouteType::Realtime => Self::Realtime,
			RouteType::Rerank => Self::Rerank,
			RouteType::Models | RouteType::Passthrough | RouteType::Detect => return None,
		})
	}

	pub fn input_format(self) -> InputFormat {
		match self {
			Self::Completions => InputFormat::Completions,
			Self::Messages => InputFormat::Messages,
			Self::Responses => InputFormat::Responses,
			Self::Embeddings => InputFormat::Embeddings,
			Self::AnthropicTokenCount => InputFormat::CountTokens,
			Self::Realtime => InputFormat::Realtime,
			Self::Rerank => InputFormat::Rerank,
		}
	}

	pub fn route_type(self) -> RouteType {
		match self {
			Self::Completions => RouteType::Completions,
			Self::Messages => RouteType::Messages,
			Self::Responses => RouteType::Responses,
			Self::Embeddings => RouteType::Embeddings,
			Self::AnthropicTokenCount => RouteType::AnthropicTokenCount,
			Self::Realtime => RouteType::Realtime,
			Self::Rerank => RouteType::Rerank,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn path_for_returns_format_path() {
		let provider = Provider {
			model: None,
			provider_override: None,
			formats: vec![
				ProviderFormatConfig {
					format: ProviderFormat::Completions,
					path: Some(strng::literal!("/v1/chat/completions")),
				},
				ProviderFormatConfig {
					format: ProviderFormat::Messages,
					path: Some(strng::literal!("/api/messages")),
				},
			],
		};

		assert_eq!(
			provider.path_for(ProviderFormat::Messages),
			Some("/api/messages")
		);
		assert_eq!(provider.path_for(ProviderFormat::Responses), None);
	}

	#[test]
	fn preset_supplies_endpoint_and_formats() {
		assert_eq!(
			serde_json::to_string(&ProviderPreset::XAI).unwrap(),
			"\"xai\""
		);

		let provider = ProviderPreset::Cohere.provider(None);
		assert_eq!(ProviderPreset::Cohere.base_url(), "https://api.cohere.ai");
		assert_eq!(
			provider.path_for(ProviderFormat::Rerank),
			Some("/v2/rerank")
		);
		assert!(provider.supports(ProviderFormat::Embeddings));
		assert_eq!(
			ProviderPreset::Ollama.base_url(),
			"http://localhost:11434/v1"
		);
		assert!(
			ProviderPreset::Ollama
				.provider(None)
				.supports(ProviderFormat::Responses)
		);
	}
}
