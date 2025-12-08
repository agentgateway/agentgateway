pub mod from_completions {
	use crate::json;
	use crate::llm::{AIError, anthropic, types};
	use std::collections::HashMap;
	use types::completions::typed as completions;
	use types::messages::typed as messages;

	/// translate an OpenAI completions request to an anthropic messages request
	pub fn translate(req: &types::completions::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(&typed);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	fn translate_internal(req: &completions::Request) -> messages::Request {
		let max_tokens = req.max_tokens();
		let stop_sequences = req.stop_sequence();
		// Anthropic has all system prompts in a single field. Join them
		let system = req
			.messages
			.iter()
			.filter_map(|msg| {
				if completions::message_role(msg) == completions::SYSTEM_ROLE {
					completions::message_text(msg).map(|s| s.to_string())
				} else {
					None
				}
			})
			.collect::<Vec<String>>()
			.join("\n");

		// Convert messages to Anthropic format
		let messages = req
			.messages
			.iter()
			.filter(|msg| completions::message_role(msg) != completions::SYSTEM_ROLE)
			.filter_map(|msg| {
				let role = match completions::message_role(msg) {
					completions::ASSISTANT_ROLE => messages::Role::Assistant,
					// Default to user for other roles
					_ => messages::Role::User,
				};

				completions::message_text(msg)
					.map(|s| {
						vec![messages::ContentBlock::Text(messages::ContentTextBlock {
							text: s.to_string(),
							citations: None,
							cache_control: None,
						})]
					})
					.map(|content| messages::Message { role, content })
			})
			.collect();

		let tools = if let Some(tools) = req.tools {
			let mapped_tools: Vec<_> = tools
				.iter()
				.map(|tool| messages::Tool {
					name: tool.function.name.clone(),
					description: tool.function.description.clone(),
					input_schema: tool.function.parameters.clone().unwrap_or_default(),
					cache_control: None,
				})
				.collect();
			Some(mapped_tools)
		} else {
			None
		};
		let metadata = req.user.map(|user| messages::Metadata {
			fields: HashMap::from([("user_id".to_string(), user)]),
		});

		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Named(completions::NamedToolChoice {
				r#type: _,
				function,
			})) => Some(messages::ToolChoice::Tool {
				name: function.name,
			}),
			Some(completions::ToolChoiceOption::Auto) => Some(messages::ToolChoice::Auto),
			Some(completions::ToolChoiceOption::Required) => Some(messages::ToolChoice::Any),
			Some(completions::ToolChoiceOption::None) => Some(messages::ToolChoice::None),
			None => None,
		};
		let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
			Some(messages::ThinkingInput::Enabled {
				budget_tokens: budget,
			})
		} else {
			match &req.reasoning_effort {
				// Arbitrary constants come from LiteLLM defaults.
				// OpenRouter uses percentages which may be more appropriate though (https://openrouter.ai/docs/use-cases/reasoning-tokens#reasoning-effort-level)
				// Note: Anthropic's minimum budget_tokens is 1024
				Some(completions::ReasoningEffort::Minimal) | Some(completions::ReasoningEffort::Low) => {
					Some(messages::ThinkingInput::Enabled {
						budget_tokens: 1024,
					})
				},
				Some(completions::ReasoningEffort::Medium) => Some(messages::ThinkingInput::Enabled {
					budget_tokens: 2048,
				}),
				Some(completions::ReasoningEffort::High) => Some(messages::ThinkingInput::Enabled {
					budget_tokens: 4096,
				}),
				None => None,
			}
		};
		messages::Request {
			messages,
			system: if system.is_empty() {
				None
			} else {
				Some(messages::SystemPrompt::Text(system))
			},
			model: req.model.unwrap_or_default(),
			max_tokens,
			stop_sequences,
			stream: req.stream.unwrap_or(false),
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: None, // OpenAI doesn't have top_k
			tools,
			tool_choice,
			metadata,
			thinking,
		}
	}
}
