use agent_llm_ir::{ClientProtocol as Client, UpstreamProtocol as Target};

// Pin refusal reasons independently of accepting updated wire snapshots.
pub fn assert_request_refusal(name: &str, client: Client, target: Target, error: &str) {
	let issues: &[&str] = match (client, name, target) {
		(Client::ChatCompletions, "file-inline", _) => &["`messages[0].content[1]` part type `file`"],
		(Client::ChatCompletions, "file-uri", _) => &[
			"`messages[0].content[1]` part type `file`",
			"`messages[0].content[2]` part type `file`",
			"`messages[0].content[3]` part type `file`",
		],
		(Client::ChatCompletions, "full", _) => &[
			"`service_tier` `priority`",
			"`n` other than 1",
			"`presence_penalty` other than 0",
			"`frequency_penalty` other than 0",
			"`logit_bias`",
			"`response_format` `json_object` has no Messages form",
			"field `seed`",
			"field `functions`",
			"field `function_call`",
			"field `messages[0].name`",
			"field `messages[1].name`",
			"field `messages[2].name`",
			"`tool_choice` names no declared tool `my_function`",
		],
		(Client::ChatCompletions, "generation-config", _) => &[
			"field `top_k`",
			"`n` other than 1",
			"`presence_penalty` other than 0",
			"`frequency_penalty` other than 0",
			"field `seed`",
		],
		(Client::ChatCompletions, "image-file", _) => {
			&["`messages[0].content[1]` must be a base64 data URL of a supported image"]
		},
		(Client::ChatCompletions, "image-inline", _) => {
			&["`messages[0].content[1]` must be a base64 data URL of a supported image"]
		},
		(Client::ChatCompletions, "image-url", _) => {
			&["`messages[0].content[1]` must be a base64 data URL of a supported image"]
		},
		(Client::ChatCompletions, "prompt-cache-breakpoint", _) => &[
			"field `messages[0].content[0].prompt_cache_breakpoint`",
			"field `messages[1].content[0].prompt_cache_breakpoint`",
			"field `messages[2].content[0].prompt_cache_breakpoint`",
			"field `messages[2].content[1].prompt_cache_breakpoint`",
			"field `messages[3].content[0].prompt_cache_breakpoint`",
			"field `messages[4].content[0].prompt_cache_breakpoint`",
		],
		(Client::ChatCompletions, "reasoning_replay", _) => {
			&["field `messages[1].reasoning_signature`"]
		},
		(Client::Gemini, "image-inline", _) => &[
			"`contents[0].parts[2]` file reference",
			"field `contents[0].parts[2].videoMetadata`",
		],
		(Client::Gemini, "passthrough-fields", _) => &[
			"field `systemInstruction.someNewSystemField`",
			"field `tools[0].functionDeclarations[0].someNewDeclField`",
			"`tools[0].someNewToolField` tool",
			"field `generationConfig.thinkingConfig.someNewThinkingField`",
			"field `generationConfig.someNewGenerationField`",
			"field `modelArmorConfig`",
			"field `someNewTopLevelField`",
			"field `toolConfig.functionCallingConfig.someNewKnob`",
			"field `toolConfig.someNewToolConfigField`",
			"field `contents[0].someNewContentField`",
			"field `contents[0].parts[0].someNewPartField`",
			"field `contents[1].parts[0].someNewPartKind`",
		],
		(Client::Gemini, "text", _) => &["field `generationConfig.seed`", "field `cachedContent`"],
		(Client::Gemini, "thinking", _) => &[
			"`contents[1].parts[0].thoughtSignature` was not written by this gateway",
			"`contents[1].parts[1].thoughtSignature` was not written by this gateway",
		],
		(Client::Gemini, "tools", _) => &[
			"`tools[1].googleSearch` tool",
			"`tools[2].codeExecution` tool",
			"`tools[3].urlContext` tool",
			"`contents[1].parts[0].thoughtSignature` was not written by this gateway",
		],
		(Client::Messages, "metadata", _) => &["field `metadata.department`"],
		(Client::Messages, "reasoning", Target::ChatCompletions | Target::Gemini) => {
			&["field `output_format`"]
		},
		(Client::Messages, "reasoning", Target::Responses { .. }) => {
			&["field `output_format`", "`thinking.display` `summarized`"]
		},
		(Client::Messages, "reasoning_enabled", _) => &[
			"`thinking.type` `enabled`",
			"field `thinking.budget_tokens`",
		],
		(Client::Messages, "reasoning_replay", _) => &[
			"`messages[1].content[1]` block type `redacted_thinking`",
			"`messages[1].content[0]`: unknown carrier format",
			"`messages[3].content[0]`: unknown carrier format",
			"`messages[5].content[0]`: unknown carrier format",
		],
		(Client::Messages, "reasoning_unsupported_model", _) => {
			&["adaptive `thinking` without an effort"]
		},
		(Client::Messages, "responses_agent_subset", _) => &[
			"field `metadata.trace_id`",
			"field `messages[0].content[2].source.url`",
			"`messages[0].content[2].source` must be base64",
			"field `messages[0].content[3].source.file_id`",
			"`messages[0].content[3].source` must be base64",
		],
		(Client::Messages, "server_tools", _) => &[
			"field `tools[0].type`",
			"field `tools[0].max_uses`",
			"`tools[0]` needs `name` and `input_schema`",
		],
		(Client::Messages, "stop_sequences", _) => &["setting `top_k`"],
		(Client::Messages, "structured-output", _) => &["setting `top_k`"],
		(Client::Messages, "tool_result_image", _) => &[
			"field `messages[2].content[0].content[1].source.url`",
			"`messages[2].content[0].content[1].source` must be base64",
		],
		(Client::Messages, "tool_result_unknown_part", _) => {
			&["`messages[2].content[0].content[1]` block type `future_block`"]
		},
		(Client::Messages, "unknown_content", _) => &[
			"`messages[0].content[0]` block type `future_block`",
			"`messages[1].content[0]` block type `future_block`",
			"`messages[2].content[0]` block type `future_block`",
		],
		(Client::Responses, "assistant-history", _) => &["`input[0].content[1]` block type `refusal`"],
		(Client::Responses, "cache_control", _) => &[
			"field `input[0].content[0][0].prompt_cache_breakpoint`",
			"field `input[0].content[1].prompt_cache_breakpoint`",
			"`input[0].content[2][0]` block type `input_file`",
			"field `input[1].content[0].prompt_cache_breakpoint`",
			"field `input[2].content[0].prompt_cache_breakpoint`",
		],
		(Client::Responses, "cache_control_tool_outputs", _) => &[
			"field `input[1].output[0][0].prompt_cache_breakpoint`",
			"field `input[3].output[0][0].prompt_cache_breakpoint`",
		],
		(Client::Responses, "input-list", Target::ChatCompletions | Target::Gemini) => &[
			"`input[4]` has no carrier",
			"`input[8]` has no carrier",
			"`messages[1].content[0]`: unknown carrier format",
			"`messages[6].content[0]`: unknown carrier format",
		],
		(Client::Responses, "input-list", Target::Messages) => &[
			"`input[4]` has no carrier",
			"`input[8]` has no carrier",
			"`input[1]`: unknown carrier format",
			"`input[6]`: unknown carrier format",
		],
		(Client::Responses, "input-media", _) => &[
			"`input[0].content[2][0]` block type `input_file`",
			"`input[0].content[3][0]` block type `input_file`",
			"`input[0].content[4][0]` block type `input_file`",
			"`input[0].content[5][0]` block type `input_file`",
			"`input[0].content[6][0]` block type `input_file`",
		],
		(Client::Responses, "instructions", _) => &["field `service_tier`"],
		(Client::Responses, "parallel-tool-call", Target::ChatCompletions | Target::Gemini) => &[
			"`input[3].arguments` must be a valid object",
			"`input[6]` answers no open tool call",
			"`messages[6].content[0]` has no call waiting for it",
		],
		(Client::Responses, "parallel-tool-call", Target::Messages) => &[
			"`input[3].arguments` must be a valid object",
			"`input[6]` answers no open tool call",
		],
		_ => panic!("{name}: refusal needs specific assertions for {client:?} -> {target:?}"),
	};
	assert_eq!(
		error,
		format!("unsupported request: {}", issues.join("; ")),
		"{name}: refusal reasons"
	);
}
