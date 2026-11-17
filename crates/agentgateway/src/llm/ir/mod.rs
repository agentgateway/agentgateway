//! Serves eligible Messages, Responses, Chat Completions and Gemini requests with the
//! `agent_llm_ir` core, to whichever protocol the provider serves for the model.
//!
//! Each attempt is selected once. A request the IR can't serve with every configured
//! behavior intact stays on the legacy converter, for a reason from a fixed set. A
//! selected request never goes back to legacy after conversion starts, so an IR refusal
//! reaches the client.
//!
//! `README.md` describes enablement, eligibility and the request and reply flow.

use std::num::NonZeroU64;

use agent_core::prelude::Strng;
use agent_llm_ir::{
	ClientProtocol, ReplyContext, ReplyPlan, RequestRefusal, StateScope, UpstreamProtocol,
};

mod enrichment;
mod guard;
mod identifier;
mod passthrough;
mod reply;
mod request;
mod response;
mod selection;

pub use response::ReplyFailed;
pub(super) use response::{process_passthrough_response, process_response};
pub use selection::{Converter, Ineligible, Selected};
pub(crate) use selection::{Selection, select};

/// What a Messages upstream gets when the client set no output limit. Messages requires one;
/// every Claude model from the 4 family takes this many, and legacy's 4096 cuts off long
/// tool calls.
const MESSAGES_DEFAULT_MAX_TOKENS: NonZeroU64 = NonZeroU64::new(32_000).unwrap();

/// Reply handling for one upstream attempt, owned separately from logged request metadata.
#[derive(Debug)]
pub enum ReplyState {
	Convert(Attempt),
	Passthrough(PassthroughReply),
}

#[derive(Debug)]
pub struct PassthroughReply {
	protocol: UpstreamProtocol,
	model: Strng,
}

#[derive(Debug)]
pub struct Attempt {
	context: AttemptContext,
	reply: ReplyPlan,
}

#[derive(Debug)]
struct AttemptContext {
	backend: Strng,
	provider: Strng,
	model: Strng,
	client: ClientProtocol,
	upstream: UpstreamProtocol,
	created: u64,
	// Call IDs derived from this value must be unique per attempt.
	fallback_reply_id: String,
}

impl Attempt {
	fn new(context: AttemptContext, reply: ReplyPlan) -> Self {
		Self { context, reply }
	}

	fn pair(&self, max_held_bytes: usize) -> ReplyContext<'_> {
		ReplyContext {
			client: self.context.client,
			upstream: self.context.upstream,
			scope: self.context.scope(),
			routes: &self.reply.routes,
			show_reasoning: self.reply.show_reasoning,
			include_usage: self.reply.include_usage,
			created: self.context.created,
			fallback_reply_id: &self.context.fallback_reply_id,
			stop_sequences: self.reply.emulated_stop_sequences.clone(),
			max_held_bytes,
		}
	}
}

impl AttemptContext {
	fn scope(&self) -> StateScope<'_> {
		StateScope {
			backend: &self.backend,
			provider: &self.provider,
			model: &self.model,
		}
	}
}

fn refused(refusal: RequestRefusal) -> agent_llm::AIError {
	agent_llm::AIError::RequestRefused {
		problems: refusal.problems,
	}
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
