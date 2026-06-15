use agent_core::prelude::Strng;
use agent_core::strng;
use std::time::Duration;

use crate::llm::{InputFormat, RouteType};
use crate::qurl::QurlProviderConfig;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
	pub formats: Vec<ProviderFormatConfig>,
	/// qURL-specific configuration for providers that use qURL/OpenNHP
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub qurl_config: Option<QurlProviderConfig>,
}

impl Provider {
	pub fn supports(&self, format: ProviderFormat) -> bool {
		self
			.formats
			.iter()
			.any(|supported| supported.format == format)
	}

	pub fn native_format_for(&self, input_format: InputFormat) -> Option<ProviderFormat> {
		let preferences: &[ProviderFormat] = match input_format {
			InputFormat::Completions => &[ProviderFormat::Completions, ProviderFormat::Messages],
			InputFormat::Messages => &[ProviderFormat::Messages, ProviderFormat::Completions],
			InputFormat::Responses => &[ProviderFormat::Responses, ProviderFormat::Completions],
			InputFormat::Embeddings => &[ProviderFormat::Embeddings],
			InputFormat::CountTokens => &[ProviderFormat::AnthropicTokenCount],
			InputFormat::Realtime => &[ProviderFormat::Realtime],
			InputFormat::Rerank => &[ProviderFormat::Rerank],
			InputFormat::Detect => return None,
		};
		preferences
			.iter()
			.copied()
			.find(|format| self.supports(*format))
	}

	pub fn path_for(&self, format: ProviderFormat) -> Option<&str> {
		self
			.formats
			.iter()
			.find(|supported| supported.format == format)
			.and_then(|supported| supported.path.as_deref())
	}

	/// Check if this provider uses qURL/OpenNHP for dynamic endpoint resolution
	pub fn uses_qurl(&self) -> bool {
		self.formats.iter().any(|f| f.format == ProviderFormat::QurlNHP)
	}
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("custom");
}

#[apply(schema!)]
pub struct ProviderFormatConfig {
	#[serde(rename = "type")]
	pub format: ProviderFormat,
	pub path: Option<Strng>,
}

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
	/// qURL + OpenNHP: resolves access token via qURL API (triggers NHP knock),
	/// then uses the resolved target_url as the backend endpoint
	QurlNHP,
}

impl ProviderFormat {
	pub fn input_format(self) -> InputFormat {
		match self {
			Self::Completions => InputFormat::Completions,
			Self::Messages => InputFormat::Messages,
			Self::Responses => InputFormat::Responses,
			Self::Embeddings => InputFormat::Embeddings,
			Self::AnthropicTokenCount => InputFormat::CountTokens,
			Self::Realtime => InputFormat::Realtime,
			Self::Rerank => InputFormat::Rerank,
			Self::QurlNHP => InputFormat::Completions, // qURL uses OpenAI-compatible format
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
			Self::QurlNHP => RouteType::Completions,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn provider(supported_formats: Vec<ProviderFormat>) -> Provider {
		Provider {
			model: None,
			formats: supported_formats
				.into_iter()
				.map(|format| ProviderFormatConfig { format, path: None })
				.collect(),
			qurl_config: None,
		}
	}

	#[test]
	fn native_format_selection_uses_preference_table() {
		let messages_only = provider(vec![ProviderFormat::Messages]);
		assert_eq!(
			messages_only.native_format_for(InputFormat::Completions),
			Some(ProviderFormat::Messages)
		);

		let completions_only = provider(vec![ProviderFormat::Completions]);
		assert_eq!(
			completions_only.native_format_for(InputFormat::Messages),
			Some(ProviderFormat::Completions)
		);
		assert_eq!(
			completions_only.native_format_for(InputFormat::Responses),
			Some(ProviderFormat::Completions)
		);

		let embeddings_only = provider(vec![ProviderFormat::Embeddings]);
		assert_eq!(
			embeddings_only.native_format_for(InputFormat::Completions),
			None
		);
	}

	#[test]
	fn path_for_returns_format_path() {
		let provider = Provider {
			model: None,
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
			qurl_config: None,
		};

		assert_eq!(
			provider.path_for(ProviderFormat::Messages),
			Some("/api/messages")
		);
		assert_eq!(provider.path_for(ProviderFormat::Responses), None);
	}

	#[test]
	fn qurl_nhp_format_recognized() {
		let provider = provider(vec![ProviderFormat::QurlNHP]);
		assert!(provider.supports(ProviderFormat::QurlNHP));
		assert!(provider.uses_qurl());
		assert_eq!(provider.native_format_for(InputFormat::Completions), Some(ProviderFormat::QurlNHP));
	}

	#[test]
	fn qurl_nhp_input_format_is_completions() {
		assert_eq!(ProviderFormat::QurlNHP.input_format(), InputFormat::Completions);
		assert_eq!(ProviderFormat::QurlNHP.route_type(), RouteType::Completions);
	}
}