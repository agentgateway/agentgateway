use agent_llm::ChatFormat;
use agent_llm::copilot::Provider;

#[test]
fn selects_supported_formats_by_model() {
	let cases = [
		(None, vec![ChatFormat::OpenAICompletions]),
		(
			Some("claude-sonnet-4"),
			vec![ChatFormat::AnthropicMessages, ChatFormat::OpenAICompletions],
		),
		(Some("grok-4.5"), vec![ChatFormat::OpenAIResponses]),
		(Some("mai-ds-r1"), vec![ChatFormat::OpenAIResponses]),
		(Some("gemini-2.5-pro"), vec![ChatFormat::OpenAICompletions]),
		(Some("gpt-3.5-turbo"), vec![ChatFormat::OpenAICompletions]),
		(Some("gpt-4.1"), vec![ChatFormat::OpenAICompletions]),
		(
			Some("gpt-5.4"),
			vec![ChatFormat::OpenAICompletions, ChatFormat::OpenAIResponses],
		),
		(
			Some("gpt-5-mini"),
			vec![ChatFormat::OpenAICompletions, ChatFormat::OpenAIResponses],
		),
		(Some("gpt-5"), vec![ChatFormat::OpenAIResponses]),
		(Some("gpt-5.4-mini"), vec![ChatFormat::OpenAIResponses]),
		(Some("unknown-model"), vec![ChatFormat::OpenAICompletions]),
	];

	for (model, expected) in cases {
		assert_eq!(
			Provider::supported_formats_for_model(model),
			expected,
			"unexpected formats for {model:?}"
		);
	}
}
