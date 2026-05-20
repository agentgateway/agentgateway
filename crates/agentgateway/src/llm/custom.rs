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
