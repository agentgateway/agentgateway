//! Synchronous request and reply conversion between LLM API formats.
//!
//! # Requests
//!
//! [`prepare`] parses client bytes into a [`Conversation`]. Read or edit it, then
//! render a [`ConvertedRequest`] for the selected [`Upstream`].
//!
//! # Replies
//!
//! Build a [`ReplyContext`] from the protocols and the request's [`ReplyPlan`].
//! Feed a provider response to [`BufferedReply`] or [`StreamReply`], answer any
//! [`ReplyProgress::Waiting`] check with a [`Verdict`], and take the client's reply bytes.
//!
//! # Lower-level model
//!
//! [`Conversation`] is the request IR and [`Event`](ir::events::Event) is the reply IR.
//! [`ReplyRewrite`] changes decoded replies before client encoding, and [`OutputChecks`]
//! holds their output back for the caller to check. The [`protocols`] module exposes
//! format-specific parsers, decoders, renderers, and writers.
//!
//! The crate does no I/O and uses no async runtime. The caller owns transport,
//! credentials, model selection, retries, and waiting for outside policy decisions.

#![forbid(unsafe_code)]

pub mod carrier;
pub mod ir {
	pub mod conversation;
	pub mod events;
}
mod json;
pub mod prompt;
pub mod protocols;
pub mod reply;
pub mod sse;

pub use carrier::{IdentifierPseudonymizer, StateScope};
pub use ir::conversation::{
	Block, BlockId, CacheBreakpoint, Content, Conversation, Effort, Message, Role, Settings, Text,
	TextMut, TextPlace,
};
pub use ir::events::{DecodeError, Telemetry, Usage};
pub use json::{Json, JsonRef};
pub use prompt::passthrough::{
	LeadingModel, ParsedRequest, RequestFields, leading_model, replace_model,
};
pub use prompt::{
	ConvertedRequest, PreparedRequest, RenderSettings, ReplyPlan, RequestRefusal, Upstream, prepare,
	prepare_parsed, read_model,
};
pub use protocols::gemini::render::{ThinkingControls, ThinkingLevel, ThinkingLevels};
pub use protocols::messages::render::PromptCaching;
pub use protocols::reasoning::ReasoningControls;
pub use protocols::{ClientEndpoint, ClientProtocol, Delivery, HttpReply, UpstreamProtocol};
pub use reply::check::{Check, IncompleteCall, OutputChecks, Verdict};
pub use reply::rewrite::{ReplyPlace, ReplyRewrite, ReplyRewrites, ToolCall};
pub use reply::{
	BufferedReply, HeldReply, ObservedReply, ReplyContext, ReplyProgress, StreamError, StreamReply,
	error_reply,
};

#[cfg(test)]
mod test_support;
