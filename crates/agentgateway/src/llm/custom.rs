use agent_core::prelude::Strng;
use agent_core::strng;

use crate::llm::{InputFormat, RouteType};
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CustomProvider"))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	/// Provider identity for cost-catalog lookup and telemetry. Built-in named providers
	/// (cohere, mistral, ...) set this so their cost resolves under the right catalog key;
	/// a bare custom provider may set it to match a catalog entry. Falls back to "custom".
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub provider_override: Option<Strng>,
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
}
