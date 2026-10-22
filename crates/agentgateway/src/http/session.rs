use ::cel::Value;
use ::http::HeaderMap;

use crate::cel::{AgentContext, Executor, Expression};
use crate::*;

// Codex sends its conversation id in unprefixed headers; older builds used `session_id` and
// `conversation_id`.
const CODEX_SESSION_HEADERS: [&str; 4] =
	["session-id", "session_id", "thread-id", "conversation_id"];

/// Resolves `request.agent.session` from `standardAttributes.session` and attaches it to the request.
/// Without an expression, or if the expression fails, the session is detected from well-known
/// agent headers. An expression returning null or an empty string means the request has no session.
pub fn apply(expression: Option<&Expression>, req: &mut http::Request) {
	let session = match expression {
		None => detect(req.headers()).map(Strng::from),
		Some(expression) => match Executor::new_request(req).eval(expression) {
			Ok(Value::Null) => None,
			Ok(Value::String(s)) => Some(Strng::from(s.as_ref())),
			Ok(value) => {
				debug!(
					expression = %expression.original_expression,
					value_type = value.type_of().as_str(),
					"session expression did not return a string; using detected session"
				);
				detect(req.headers()).map(Strng::from)
			},
			Err(err) => {
				trace!(
					expression = %expression.original_expression,
					error = %err,
					"session expression failed; using detected session"
				);
				detect(req.headers()).map(Strng::from)
			},
		},
	};
	if let Some(session) = session.filter(|s| !s.is_empty()) {
		req.extensions_mut().insert(AgentContext {
			session: Some(session),
		});
	}
}

/// Detects the session id sent by common agents, in priority order:
/// 1. Any `x-<vendor>-session-id` header, such as Claude Code's `x-claude-code-session-id`.
/// 2. Codex's unprefixed session headers, only when the User-Agent identifies Codex.
/// 3. A vendor-less `x-session-id` header, such as opencode's.
fn detect(headers: &HeaderMap) -> Option<&str> {
	fn valid(value: &::http::HeaderValue) -> Option<&str> {
		value.to_str().ok().filter(|v| {
			v.len() >= 8
				&& v
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
		})
	}
	let vendor = headers.iter().find_map(|(name, value)| {
		let vendor = name
			.as_str()
			.strip_prefix("x-")?
			.strip_suffix("-session-id")?;
		(!vendor.is_empty()).then(|| valid(value)).flatten()
	});
	let codex = || {
		let user_agent = headers.get(::http::header::USER_AGENT)?.as_bytes();
		let is_codex = user_agent.len() > 5
			&& user_agent[..5].eq_ignore_ascii_case(b"codex")
			&& matches!(user_agent[5], b'-' | b'_' | b' ' | b'/');
		if !is_codex {
			return None;
		}
		CODEX_SESSION_HEADERS
			.iter()
			.find_map(|h| headers.get(*h).and_then(valid))
	};
	vendor
		.or_else(codex)
		.or_else(|| headers.get("x-session-id").and_then(valid))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn detect_priority() {
		let detect_from = |headers: &[(&str, &str)]| {
			let mut map = HeaderMap::new();
			for (k, v) in headers {
				map.append(
					::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
					v.parse().unwrap(),
				);
			}
			detect(&map).map(str::to_owned)
		};
		let id = "e96634a3-fa28-4083-b354-55542e2dca01";
		assert_eq!(
			detect_from(&[("x-claude-code-session-id", id)]).as_deref(),
			Some(id)
		);
		assert_eq!(
			detect_from(&[
				("x-session-id", "opencode-1"),
				("x-claude-code-session-id", id)
			])
			.as_deref(),
			Some(id)
		);
		// Codex headers are only trusted from Codex.
		assert_eq!(detect_from(&[("session-id", id)]), None);
		assert_eq!(
			detect_from(&[("user-agent", "codex_cli_rs/0.1"), ("thread-id", id)]).as_deref(),
			Some(id)
		);
		assert_eq!(
			detect_from(&[("user-agent", "codexfoo"), ("session-id", id)]),
			None
		);
		// Values must look like an identifier.
		assert_eq!(detect_from(&[("x-session-id", "short")]), None);
		assert_eq!(detect_from(&[("x-session-id", "has spaces in it")]), None);
	}
}
