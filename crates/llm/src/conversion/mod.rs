pub mod bedrock;
pub mod completions;
pub mod gemini;
pub mod messages;
pub mod namespace_tools;
pub mod openai_compat;
pub mod responses;
pub mod systemone;
pub mod vertex;
pub mod vertex_gemini;

#[derive(Default)]
struct ProviderUsage {
	input_tokens: u64,
	total_tokens: u64,
	reasoning_tokens: Option<u64>,
}

/// Keep provider token counts for accounting when translating client usage conventions.
struct ResponseWithProviderUsage<T> {
	response: T,
	provider_usage: Option<ProviderUsage>,
}

impl<T: crate::types::ResponseType> crate::types::ResponseType for ResponseWithProviderUsage<T> {
	fn to_llm_response(&self, log_content: crate::LogContentFields) -> crate::LLMResponse {
		let mut response = self.response.to_llm_response(log_content);
		if let Some(usage) = &self.provider_usage {
			response.input_tokens = Some(usage.input_tokens);
			response.total_tokens = Some(usage.total_tokens);
			response.reasoning_tokens = usage.reasoning_tokens.or(response.reasoning_tokens);
		}
		response
	}

	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
		self.response.to_webhook_choices()
	}

	fn set_webhook_choices(
		&mut self,
		choices: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		self.response.set_webhook_choices(choices)
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		self.response.serialize()
	}

	fn visit_text_mut(&mut self, f: &mut dyn FnMut(crate::types::ResponseText, &mut String)) {
		self.response.visit_text_mut(f)
	}
}

/// Older OpenAI chat models have no reasoning support and reject any reasoning effort.
pub(crate) fn supports_reasoning_effort(model: &str) -> bool {
	!["gpt-3.5", "gpt-4", "chatgpt-4o"]
		.iter()
		.any(|prefix| model.starts_with(prefix))
}

pub(crate) fn supports_prompt_cache_breakpoint(model: &str) -> bool {
	model
		.strip_prefix("gpt-")
		.and_then(|model| model.split('-').next())
		.and_then(|version| version.split_once('.'))
		.and_then(|(major, minor)| Some((major.parse::<u32>().ok()?, minor.parse::<u32>().ok()?)))
		.is_some_and(|version| version >= (5, 6))
}

/// Translate an OpenAI `tool_calls[].function.arguments` string into an Anthropic
/// `tool_use.input` value.
///
/// A tool call with no arguments is conventionally encoded as the empty string, which is not
/// valid JSON. Anthropic requires `tool_use.input` to be an object, so that becomes `{}`.
///
/// Everything else is parsed and passed through unchanged, including values that are malformed
/// or not an object (this would avoid changing silently the upstream response).
pub(crate) fn tool_arguments_to_input(arguments: &str) -> serde_json::Value {
	if arguments.is_empty() {
		return serde_json::json!({});
	}
	serde_json::from_str::<serde_json::Value>(arguments)
		.unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()))
}

#[cfg(test)]
mod rerank_tests;

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::tool_arguments_to_input;

	#[test]
	fn thinking_budget_buckets() {
		use crate::types::messages::typed::ThinkingEffort::{High, Low, Max, Medium, Xhigh};
		for (budget, expected) in [
			(0, Low),
			(1024, Low),
			(2047, Low),
			(2048, Medium),
			(4095, Medium),
			(4096, High),
			(8191, High),
			(8192, Xhigh),
			(16383, Xhigh),
			(16384, Max),
			(u64::MAX, Max),
		] {
			assert_eq!(
				crate::types::anthropic_effort_for_thinking_budget(budget),
				expected,
				"budget {budget}"
			);
		}
	}

	#[test]
	fn empty_arguments_become_an_empty_object() {
		assert_eq!(tool_arguments_to_input(""), json!({}));
	}

	#[test]
	fn everything_else_passes_through() {
		// Valid JSON is forwarded as parsed, whatever shape it has.
		assert_eq!(tool_arguments_to_input("{}"), json!({}));
		assert_eq!(
			tool_arguments_to_input("{\"a\":1,\"b\":[2,null]}"),
			json!({"a": 1, "b": [2, null]})
		);
		assert_eq!(tool_arguments_to_input("[]"), json!([]));
		// Non-object JSON is forwarded as parsed; malformed JSON is kept verbatim as a string instead of being degraded to `{}`
		assert_eq!(tool_arguments_to_input("null"), json!(null));
		assert_eq!(tool_arguments_to_input("5"), json!(5));
		assert_eq!(
			tool_arguments_to_input("{\"location\": \"Par"),
			json!("{\"location\": \"Par")
		);
		assert_eq!(tool_arguments_to_input("  "), json!("  "));
	}
}
