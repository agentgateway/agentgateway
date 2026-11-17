//! The prompt guard on an IR attempt. Selection keeps any guard kind but regex on the legacy
//! converter. Each guard matches with the guard's own rules and records the same log entries
//! and metrics as on legacy.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use agent_llm_ir::{Conversation, ReplyPlace, ReplyRewrite, ReplyRewrites, Role, TextPlace};

use crate::http::Response;
use crate::llm::ContentScope;
use crate::llm::policy::{
	Action, Policy, PromptGuard, RegexResult, RegexRules, RequestGuardKind, ResponseGuard,
	ResponseGuardKind, record_guardrail,
};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::GuardrailLog;
use crate::telemetry::metrics::{GuardrailAction, GuardrailPhase};

/// Runs the request guards in order over the conversation's texts, masking in place. Returns
/// the first rejection and the kind of guard that made it.
pub(super) fn request(
	guard: &PromptGuard,
	conversation: &mut Conversation<'_>,
	client: &PolicyClient,
	log: Option<&GuardrailLog>,
) -> Option<(Response, &'static str)> {
	// A retry runs the guards again.
	if let Some(log) = log {
		log.non_atomic_mutate(|entries| entries.retain(|entry| entry.phase != "request"));
	}
	for guard in &guard.request {
		let RequestGuardKind::Regex(rules) = &guard.kind else {
			unreachable!("selection keeps other guard kinds on legacy");
		};
		let action = regex_request(rules, &guard.scope, conversation);
		record_guardrail(
			log,
			GuardrailPhase::Request,
			guard.kind.name(),
			action,
			None,
		);
		Policy::record_guardrail_trip(client, GuardrailPhase::Request, action);
		if action == GuardrailAction::Reject {
			return Some((guard.rejection.as_response(), guard.kind.name()));
		}
	}
	None
}

/// A guard's rules have one action, so it either masks or rejects and audits.
fn regex_request(
	rules: &RegexRules,
	scope: &[ContentScope],
	conversation: &mut Conversation<'_>,
) -> GuardrailAction {
	let in_scope = |place| scope.contains(&request_scope(place));
	let check = |text: &str| Policy::apply_prompt_guard_regex(text, rules, GuardrailPhase::Request);
	match rules.action {
		Action::Mask => {
			let mut masked = false;
			conversation.edit_texts(|mut text| {
				if !in_scope(text.place()) {
					return;
				}
				let result = check(&text.read());
				if let Some(RegexResult::Mask(replacement)) = result {
					text.replace(replacement);
					masked = true;
				}
			});
			if masked {
				GuardrailAction::Mask
			} else {
				GuardrailAction::Allow
			}
		},
		Action::Reject | Action::Audit => {
			let matched = conversation
				.texts()
				.any(|(place, text)| in_scope(place) && check(&text.read()).is_some());
			match (matched, &rules.action) {
				(false, _) => GuardrailAction::Allow,
				(true, Action::Audit) => GuardrailAction::Audit,
				(true, _) => GuardrailAction::Reject,
			}
		},
	}
}

/// The guard's scope for a text in the request.
fn request_scope(place: TextPlace) -> ContentScope {
	match place {
		TextPlace::System(_) | TextPlace::Message(Role::System, _) => ContentScope::SystemPrompt,
		TextPlace::Message(..) => ContentScope::Messages,
		TextPlace::ToolResult { .. } => ContentScope::ToolOutput,
		TextPlace::ToolCall { .. } => ContentScope::ToolInput,
	}
}

/// The response guards, and where they record what they did.
pub(super) struct ReplyGuards<'a> {
	pub(super) guards: &'a [ResponseGuard],
	pub(super) client: &'a PolicyClient,
	pub(super) log: Option<&'a GuardrailLog>,
}

/// The response guards on a buffered reply. Each text is kept until it is whole, then every
/// guard runs on it in order, and a masking guard replaces it for the guards after it. Legacy
/// runs each guard over every text before the next guard; masks apply to one text at a time,
/// so the texts sent and the first guard to reject come out the same.
pub(super) struct BufferedGuard<'a> {
	guards: ReplyGuards<'a>,
	seen: Arc<Mutex<Seen>>,
}

struct Seen {
	/// The most severe action of each guard.
	actions: Vec<GuardrailAction>,
	/// The first guard to reject.
	rejected: Option<usize>,
}

impl<'a> BufferedGuard<'a> {
	/// The guard, and the rewrite that runs it as the reply is converted.
	pub(super) fn new(guards: ReplyGuards<'a>) -> (Self, ReplyRewrites) {
		let seen = Arc::new(Mutex::new(Seen {
			actions: vec![GuardrailAction::Allow; guards.guards.len()],
			rejected: None,
		}));
		let rewrite = GuardRewrite {
			guards: guards.guards.to_vec(),
			kept: Vec::new(),
			seen: seen.clone(),
		};
		(
			Self { guards, seen },
			ReplyRewrites::new(vec![Box::new(rewrite)]),
		)
	}

	/// Records each guard that ran, as legacy does, and returns the first rejection.
	pub(super) fn finish(self) -> Option<Response> {
		let ReplyGuards {
			guards,
			client,
			log,
		} = self.guards;
		let seen = lock(&self.seen);
		let ran = seen.rejected.map_or(guards.len(), |at| at + 1);
		for (guard, action) in guards.iter().zip(&seen.actions).take(ran) {
			record_guardrail(
				log,
				GuardrailPhase::Response,
				guard.kind.name(),
				*action,
				None,
			);
			Policy::record_guardrail_trip(client, GuardrailPhase::Response, *action);
		}
		seen.rejected.map(|at| guards[at].rejection.as_response())
	}
}

struct GuardRewrite {
	guards: Vec<ResponseGuard>,
	/// Text kept for each place until it is whole.
	kept: Vec<(ReplyPlace, String)>,
	seen: Arc<Mutex<Seen>>,
}

impl ReplyRewrite for GuardRewrite {
	fn text(&mut self, place: ReplyPlace, text: &str, _out: &mut String) {
		match self.kept.iter_mut().find(|(kept, _)| *kept == place) {
			Some((_, kept)) => kept.push_str(text),
			None => self.kept.push((place, text.to_string())),
		}
	}

	fn finish(&mut self, place: ReplyPlace, out: &mut String) {
		let Some(at) = self.kept.iter().position(|(kept, _)| *kept == place) else {
			return;
		};
		let (_, mut text) = self.kept.swap_remove(at);
		let scope = response_scope(place);
		let mut seen = lock(&self.seen);
		for (at, guard) in self.guards.iter().enumerate() {
			if !guard.scope.contains(&scope) {
				continue;
			}
			let ResponseGuardKind::Regex(rules) = &guard.kind else {
				unreachable!("selection keeps other guard kinds on legacy");
			};
			let action = match Policy::apply_prompt_guard_regex(&text, rules, GuardrailPhase::Response) {
				None => continue,
				Some(RegexResult::Mask(masked)) => {
					text = masked;
					GuardrailAction::Mask
				},
				Some(RegexResult::Reject) => {
					seen.rejected = Some(seen.rejected.map_or(at, |first| first.min(at)));
					GuardrailAction::Reject
				},
				Some(RegexResult::Audit) => GuardrailAction::Audit,
			};
			seen.actions[at] = seen.actions[at].max(action);
		}
		out.push_str(&text);
	}
}

fn lock(seen: &Mutex<Seen>) -> MutexGuard<'_, Seen> {
	seen.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The guard's scope for a text in the reply. Reasoning is replayed from the IR's carriers,
/// not from the text the client is shown, so it can be masked.
fn response_scope(place: ReplyPlace) -> ContentScope {
	match place {
		ReplyPlace::Answer(_) | ReplyPlace::Refusal(_) | ReplyPlace::Reasoning(_) => {
			ContentScope::Messages
		},
		ReplyPlace::ToolCall(_) | ReplyPlace::ToolCallArgument { .. } => ContentScope::ToolInput,
	}
}
