//! Prompt enrichment as an IR edit: the policy's prompts go before and after the client's,
//! whatever protocol either side speaks.

use agent_llm_ir::{
	Block, BlockId, CacheBreakpoint, ClientProtocol, Content, Conversation, Message, Role, Text,
};
use indexmap::IndexMap;
use serde_json::json;
use serde_json::value::{RawValue, to_raw_value};

use crate::llm::policy::PromptEnrichment;
use crate::llm::{AIError, SimpleChatCompletionMessage};

/// System prompts join the system text, and the others become turns, as legacy adds them.
/// A prompt with another role is refused: no other role means the same in every protocol.
pub(super) fn apply(
	prompts: &PromptEnrichment,
	conversation: &mut Conversation<'_>,
) -> Result<(), AIError> {
	let Split {
		system: system_before,
		turns: turns_before,
	} = split(&prompts.prepend)?;
	let Split {
		system: system_after,
		turns: turns_after,
	} = split(&prompts.append)?;
	if !system_before.is_empty() || !system_after.is_empty() {
		let system = conversation.system.get_or_insert_with(Vec::new);
		system.splice(
			0..0,
			system_before
				.iter()
				.map(|text| Text::from(text.to_string())),
		);
		system.extend(system_after.iter().map(|text| Text::from(text.to_string())));
		for breakpoint in &mut conversation.cache_breakpoints {
			if let CacheBreakpoint::System(at) = breakpoint {
				*at += system_before.len();
			}
		}
	}
	// Added turns are numbered past the client's, so no block shares a client block's ID.
	let mut next = conversation
		.messages
		.iter()
		.map(|message| message.index + 1)
		.max()
		.unwrap_or(0);
	let mut turn = |(role, text): &(Role, &str)| {
		let index = next;
		next += 1;
		Message {
			index,
			role: *role,
			blocks: vec![Block {
				id: BlockId {
					message: index,
					block: 0,
				},
				content: Content::Text(Text::from(text.to_string())),
			}],
			messages_output_config: None,
		}
	};
	let before: Vec<_> = turns_before.iter().map(&mut turn).collect();
	let after: Vec<_> = turns_after.iter().map(&mut turn).collect();
	conversation.messages.splice(0..0, before);
	conversation.messages.extend(after);
	Ok(())
}

/// Edits only native conversation fields. Existing entries stay raw so provider extensions,
/// signed blocks and client cache marks are not interpreted or rewritten.
pub(super) fn apply_passthrough(
	prompts: &PromptEnrichment,
	client: ClientProtocol,
	fields: &mut IndexMap<String, Box<RawValue>>,
) -> Result<(), serde_json::Error> {
	let separate_system = matches!(client, ClientProtocol::Messages | ClientProtocol::Gemini);
	let turns = |prompts: &[SimpleChatCompletionMessage]| {
		prompts
			.iter()
			.filter(|prompt| !separate_system || prompt.role.as_str() != "system")
			.map(|prompt| native_prompt(client, prompt))
			.collect::<Result<Vec<_>, _>>()
	};
	let before = turns(&prompts.prepend)?;
	let after = turns(&prompts.append)?;
	let field = match client {
		ClientProtocol::Messages | ClientProtocol::ChatCompletions => "messages",
		ClientProtocol::Responses => "input",
		ClientProtocol::Gemini => "contents",
	};
	if client == ClientProtocol::Responses
		&& (!before.is_empty() || !after.is_empty())
		&& let Some(input) = fields.get("input")
		&& input.get().starts_with('"')
	{
		let text: String = serde_json::from_str(input.get())?;
		let message = native_prompt(
			client,
			&SimpleChatCompletionMessage {
				role: "user".into(),
				content: text.into(),
			},
		)?;
		fields.insert("input".into(), to_raw_value(&[message])?);
	}
	add_native_parts(fields, field, before, after)?;
	if !separate_system {
		return Ok(());
	}
	// Legacy separates only the system role for Messages and Gemini. Other roles retain
	// their existing native representation rather than being normalized through the IR.
	let system = |prompts: &[SimpleChatCompletionMessage]| {
		prompts
			.iter()
			.filter(|prompt| prompt.role.as_str() == "system")
			.map(|prompt| {
				to_raw_value(&match client {
					ClientProtocol::Messages => json!({"type": "text", "text": prompt.content}),
					_ => json!({"text": prompt.content}),
				})
			})
			.collect::<Result<Vec<_>, _>>()
	};
	let before = system(&prompts.prepend)?;
	let after = system(&prompts.append)?;
	if before.is_empty() && after.is_empty() {
		return Ok(());
	}
	if client == ClientProtocol::Messages {
		if let Some(system) = fields.get("system") {
			if system.get() == "null" {
				fields.shift_remove("system");
			} else if system.get().starts_with('"') {
				let text: String = serde_json::from_str(system.get())?;
				fields.insert(
					"system".into(),
					to_raw_value(&[json!({"type": "text", "text": text})])?,
				);
			}
		}
		return add_native_parts(fields, "system", before, after);
	}
	let key = match (
		fields.contains_key("systemInstruction"),
		fields.contains_key("system_instruction"),
	) {
		(true, true) => {
			return Err(serde::de::Error::custom(
				"both systemInstruction and system_instruction are present",
			));
		},
		(false, true) => "system_instruction",
		_ => "systemInstruction",
	};
	let mut instruction = match fields.get(key) {
		Some(raw) if raw.get() != "null" => serde_json::from_str(raw.get())?,
		_ => IndexMap::new(),
	};
	add_native_parts(&mut instruction, "parts", before, after)?;
	fields.insert(key.into(), to_raw_value(&instruction)?);
	Ok(())
}

fn native_prompt(
	client: ClientProtocol,
	prompt: &SimpleChatCompletionMessage,
) -> Result<Box<RawValue>, serde_json::Error> {
	match client {
		ClientProtocol::Responses => to_raw_value(&async_openai::types::responses::InputItem::from(
			prompt.clone(),
		)),
		ClientProtocol::Messages | ClientProtocol::ChatCompletions => to_raw_value(prompt),
		ClientProtocol::Gemini => to_raw_value(&json!({
			"role": if prompt.role.as_str() == "assistant" { "model" } else { prompt.role.as_str() },
			"parts": [{"text": prompt.content}],
		})),
	}
}

fn add_native_parts(
	fields: &mut IndexMap<String, Box<RawValue>>,
	field: &str,
	mut before: Vec<Box<RawValue>>,
	after: Vec<Box<RawValue>>,
) -> Result<(), serde_json::Error> {
	if before.is_empty() && after.is_empty() {
		return Ok(());
	}
	let existing: Vec<Box<RawValue>> = match fields.get(field) {
		Some(raw) => serde_json::from_str(raw.get())?,
		None => Vec::new(),
	};
	before.extend(existing);
	before.extend(after);
	fields.insert(field.into(), to_raw_value(&before)?);
	Ok(())
}

/// A policy's prompts by where they go, each in order.
struct Split<'a> {
	system: Vec<&'a str>,
	turns: Vec<(Role, &'a str)>,
}

fn split(prompts: &[SimpleChatCompletionMessage]) -> Result<Split<'_>, AIError> {
	let mut system = Vec::new();
	let mut turns = Vec::new();
	for prompt in prompts {
		match prompt.role.as_str() {
			"system" | "developer" => system.push(prompt.content.as_str()),
			"user" => turns.push((Role::User, prompt.content.as_str())),
			"assistant" => turns.push((Role::Assistant, prompt.content.as_str())),
			role => {
				return Err(AIError::RequestRefused {
					problems: vec![format!(
						"prompt enrichment role `{role}` can't be converted; use system, developer, user or assistant"
					)],
				});
			},
		}
	}
	Ok(Split { system, turns })
}
