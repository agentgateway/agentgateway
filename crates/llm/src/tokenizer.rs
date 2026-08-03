use tiktoken_rs::CoreBPE;
use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};

use crate::{AIError, SimpleChatCompletionMessage};

pub fn num_tokens_from_messages(
	model: &str,
	messages: &[SimpleChatCompletionMessage],
) -> Result<u64, AIError> {
	let tokenizer = get_tokenizer(model).unwrap_or(Tokenizer::Cl100kBase);
	if tokenizer != Tokenizer::Cl100kBase && tokenizer != Tokenizer::O200kBase {
		return Err(AIError::UnsupportedModel);
	}
	let bpe = get_bpe_from_tokenizer(tokenizer);
	let tokens_per_message = 3;

	let mut num_tokens: u64 = 0;
	for message in messages {
		num_tokens += tokens_per_message;
		num_tokens += 1;
		num_tokens += bpe
			.encode_with_special_tokens(message.content.as_str())
			.len() as u64;
	}
	num_tokens += 3;
	Ok(num_tokens)
}

pub fn preload_tokenizers() {
	let _ = tiktoken_rs::cl100k_base_singleton();
	let _ = tiktoken_rs::o200k_base_singleton();
}

pub fn get_bpe_from_tokenizer<'a>(tokenizer: Tokenizer) -> &'a CoreBPE {
	match tokenizer {
		Tokenizer::O200kHarmony => tiktoken_rs::o200k_harmony_singleton(),
		Tokenizer::O200kBase => tiktoken_rs::o200k_base_singleton(),
		Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
		Tokenizer::R50kBase => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::P50kBase => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::P50kEdit => tiktoken_rs::r50k_base_singleton(),
		Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
	}
}
