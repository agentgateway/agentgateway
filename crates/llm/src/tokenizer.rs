use serde_json::Value;
use tiktoken::CoreBpe;

use crate::types::{NormalizedMessage, NormalizedMessagePartType};

/// Estimates input tokens for any model by counting the request content with o200k.
///
/// Claude's tokenizers produce more tokens than o200k for the same text (about 1.3x, and about
/// 1.6x from Claude 4.7), and Claude adds a hidden system prompt of about 600 tokens when tools
/// are defined. Other providers land close to o200k.
pub fn num_tokens<'a>(
	model: &str,
	messages: &[NormalizedMessage],
	tools: impl IntoIterator<Item = &'a Value>,
) -> u64 {
	let bpe = o200k_base();
	let mut content = 0;
	for part in messages.iter().flat_map(|m| &m.parts) {
		content += match part.r#type {
			NormalizedMessagePartType::Text => part
				.text
				.as_deref()
				.map_or(0, |t| bpe.count_with_special_tokens(t)),
			NormalizedMessagePartType::ToolCall => {
				part.name.as_deref().map_or(0, |n| bpe.count(n))
					+ part.arguments.as_ref().map_or(0, |a| count_json(bpe, a))
			},
			NormalizedMessagePartType::ToolResult => {
				part.content.as_ref().map_or(0, |c| count_strings(bpe, c))
			},
			// Providers drop reasoning from earlier turns, and its signatures would dominate the count.
			NormalizedMessagePartType::Reasoning => 0,
		};
	}
	let mut has_tools = false;
	for tool in tools {
		has_tools = true;
		content += count_json(bpe, tool);
	}

	let claude = claude_version(model);
	let multiplier = match claude {
		Some(version) if version >= (4, 7) => 1.6,
		Some(_) => 1.3,
		None => 1.0,
	};
	let mut tokens = (content as f64 * multiplier).round() as u64 + 4 * messages.len() as u64 + 3;
	if claude.is_some() && has_tools {
		tokens += 600;
	}
	tokens
}

/// Counts a JSON value as it would be serialized; strings are counted without quotes, matching
/// OpenAI's string-encoded tool call arguments.
fn count_json(bpe: &CoreBpe, value: &Value) -> usize {
	match value {
		Value::String(s) => bpe.count(s),
		v => bpe.count(&v.to_string()),
	}
}

/// Counts only the string leaves of a JSON value, so tool result blocks like
/// `[{"type":"text","text":"..."}]` cost about what their text does.
fn count_strings(bpe: &CoreBpe, value: &Value) -> usize {
	match value {
		Value::String(s) => bpe.count(s),
		Value::Array(items) => items.iter().map(|v| count_strings(bpe, v)).sum(),
		Value::Object(fields) => fields.values().map(|v| count_strings(bpe, v)).sum(),
		_ => 0,
	}
}

/// Parses the (major, minor) version out of Claude model names in any provider's naming scheme,
/// e.g. `claude-opus-4-7`, `claude-3-5-sonnet-20241022`, `us.anthropic.claude-sonnet-4-5-v1:0`,
/// `claude-opus-4@20250514`. A name without a version is treated as an older model.
fn claude_version(model: &str) -> Option<(u32, u32)> {
	let rest = &model[model.find("claude")? + "claude".len()..];
	let is_version = |s: &&str| (1..=2).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
	let mut segments = rest
		.split(['-', '.', '@', ':', '_'])
		.skip_while(|s| !is_version(s));
	let major = segments.next().map_or(0, |s| s.parse().unwrap_or(0));
	let minor = segments
		.next()
		.filter(is_version)
		.map_or(0, |s| s.parse().unwrap_or(0));
	Some((major, minor))
}

pub fn preload_tokenizers() {
	let _ = o200k_base();
}

fn o200k_base() -> &'static CoreBpe {
	tiktoken::get_encoding("o200k_base").expect("o200k_base is enabled")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn claude_versions() {
		for (model, want) in [
			("claude-opus-4-7", Some((4, 7))),
			("claude-sonnet-4-5-20250929", Some((4, 5))),
			("claude-opus-4-20250514", Some((4, 0))),
			("claude-opus-4@20250514", Some((4, 0))),
			("claude-3-7-sonnet-20250219", Some((3, 7))),
			("us.anthropic.claude-sonnet-5-5-v1:0", Some((5, 5))),
			("anthropic.claude-v2", Some((0, 0))),
			("gpt-5", None),
		] {
			assert_eq!(claude_version(model), want, "{model}");
		}
	}
}
