use std::collections::HashMap;

use super::methods::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Phase {
	Off,
	Request,
	Response,
	Both,
}

impl Phase {
	pub fn runs_request(self) -> bool {
		matches!(self, Phase::Request | Phase::Both)
	}
	pub fn runs_response(self) -> bool {
		matches!(self, Phase::Response | Phase::Both)
	}
}

pub fn resolve(method: &str, overrides: &HashMap<String, Phase>) -> Phase {
	if let Some(p) = overrides.get(method) {
		return *p;
	}
	default_phase(method)
}

// Defaults reflect methods with wired hooks. Subscribe/unsubscribe and completion/complete
// are Off (design intent: Request) until their hooks land — otherwise the table lies.
pub fn default_phase(method: &str) -> Phase {
	if is_list(method) {
		return Phase::Response;
	}
	match method {
		// tools/call + prompts/get default to Both: their responses flow into LLM
		// context, so response-side scrubbing matters more than the extra callout cost.
		// resources/read stays Request (variable volume, document-heavy servers).
		TOOLS_CALL | PROMPTS_GET => Phase::Both,
		RESOURCES_READ => Phase::Request,
		RESOURCES_SUBSCRIBE | RESOURCES_UNSUBSCRIBE => Phase::Off,
		COMPLETION_COMPLETE => Phase::Off,
		INITIALIZE | PING | LOGGING_SET_LEVEL => Phase::Off,
		m if m.starts_with("notifications/") => Phase::Off,
		_ => Phase::Off,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_match_wired_hooks() {
		// Wired: response-phase list filters.
		assert_eq!(default_phase("tools/list"), Phase::Response);
		assert_eq!(default_phase("prompts/list"), Phase::Response);
		assert_eq!(default_phase("resources/list"), Phase::Response);
		assert_eq!(default_phase("resources/templates/list"), Phase::Response);
		// tools/call + prompts/get: Both, response feeds the LLM.
		assert_eq!(default_phase("tools/call"), Phase::Both);
		assert_eq!(default_phase("prompts/get"), Phase::Both);
		// resources/read: Request only by default; operators opt into Both.
		assert_eq!(default_phase("resources/read"), Phase::Request);
		// Unwired: design intent is `Request`, but no hook exists yet. Default
		// to `Off` so the defaults table doesn't advertise behavior we don't
		// implement.
		assert_eq!(default_phase("resources/subscribe"), Phase::Off);
		assert_eq!(default_phase("resources/unsubscribe"), Phase::Off);
		assert_eq!(default_phase("completion/complete"), Phase::Off);
		// Handshake / chatter / unknown.
		assert_eq!(default_phase("initialize"), Phase::Off);
		assert_eq!(
			default_phase("notifications/tools/list_changed"),
			Phase::Off
		);
		assert_eq!(default_phase("ping"), Phase::Off);
		assert_eq!(default_phase("some/unknown"), Phase::Off);
	}

	#[test]
	fn overrides_win() {
		let mut o = HashMap::new();
		o.insert("tools/list".to_string(), Phase::Off);
		o.insert("initialize".to_string(), Phase::Both);
		assert_eq!(resolve("tools/list", &o), Phase::Off);
		assert_eq!(resolve("initialize", &o), Phase::Both);
		// Method not in overrides falls back to default.
		assert_eq!(resolve("tools/call", &o), Phase::Both);
	}
}
