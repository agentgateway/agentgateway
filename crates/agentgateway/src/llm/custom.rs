use agent_core::prelude::Strng;
use agent_core::strng;

use crate::llm::InputFormat;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	pub supported_formats: Vec<ProviderFormat>,
}

impl Provider {
	pub fn supports(&self, format: InputFormat) -> bool {
		self
			.supported_formats
			.iter()
			.any(|supported| supported.supports(format))
	}

	pub fn native_format_for(&self, input_format: InputFormat) -> Option<InputFormat> {
		let preferences: &[InputFormat] = match input_format {
			InputFormat::Completions => &[InputFormat::Completions, InputFormat::Messages],
			InputFormat::Messages => &[InputFormat::Messages, InputFormat::Completions],
			InputFormat::Responses => &[InputFormat::Responses, InputFormat::Completions],
			InputFormat::Embeddings => &[InputFormat::Embeddings],
			InputFormat::CountTokens => &[InputFormat::CountTokens],
			InputFormat::Realtime => &[InputFormat::Realtime],
			InputFormat::Detect => return Some(InputFormat::Detect),
		};
		preferences
			.iter()
			.copied()
			.find(|format| self.supports(*format))
	}
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("custom");
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
}

impl ProviderFormat {
	fn supports(self, format: InputFormat) -> bool {
		matches!(
			(self, format),
			(Self::Completions, InputFormat::Completions)
				| (Self::Messages, InputFormat::Messages)
				| (Self::Responses, InputFormat::Responses)
				| (Self::Embeddings, InputFormat::Embeddings)
				| (Self::AnthropicTokenCount, InputFormat::CountTokens)
				| (Self::Realtime, InputFormat::Realtime)
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn provider(supported_formats: Vec<ProviderFormat>) -> Provider {
		Provider { supported_formats }
	}

	#[test]
	fn native_format_selection_uses_preference_table() {
		let messages_only = provider(vec![ProviderFormat::Messages]);
		assert_eq!(
			messages_only.native_format_for(InputFormat::Completions),
			Some(InputFormat::Messages)
		);

		let completions_only = provider(vec![ProviderFormat::Completions]);
		assert_eq!(
			completions_only.native_format_for(InputFormat::Messages),
			Some(InputFormat::Completions)
		);
		assert_eq!(
			completions_only.native_format_for(InputFormat::Responses),
			Some(InputFormat::Completions)
		);

		let embeddings_only = provider(vec![ProviderFormat::Embeddings]);
		assert_eq!(
			embeddings_only.native_format_for(InputFormat::Completions),
			None
		);
	}
}
