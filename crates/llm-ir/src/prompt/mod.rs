#![doc = include_str!("README.md")]

use std::num::NonZeroU64;

use diagnostics::{Diagnostics, Unread};

use crate::carrier::{IdentifierPseudonymizer, StateScope};
use crate::ir::conversation::{Block, Content, Conversation, Message, Role};
use crate::json::input::RequestInput;
use crate::protocols::chat_completions::{parse as completions_parse, render as completions};
use crate::protocols::gemini::render::{ThinkingControls, signs_text};
use crate::protocols::gemini::{parse as gemini_parse, render as gemini};
use crate::protocols::messages::render::PromptCaching;
use crate::protocols::messages::{parse as messages, render as messages_render};
pub use crate::protocols::read_model;
use crate::protocols::reasoning::ReasoningControls;
use crate::protocols::responses::{parse as responses, render as responses_render};
use crate::protocols::{ClientEndpoint, ClientProtocol, Delivery, ToolRoute, UpstreamProtocol};

pub(crate) mod diagnostics;
pub mod passthrough;

/// Marks a failed tool call where the target has no failure flag.
pub(crate) const TOOL_ERROR_MARKER: &str = "[tool error]\n";

pub(crate) const NOTHING_TO_SEND: &str = "the request has nothing left to send";

#[cfg(test)]
mod tests;

/// Problems that prevent a request from being represented for its target.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("unsupported request: {}", problems.join("; "))]
pub struct RequestRefusal {
	pub problems: Vec<String>,
}

/// A selected upstream, without transport, credentials, or a model catalog.
#[derive(Clone, Copy)]
pub struct Upstream<'a> {
	pub identifier_pseudonymizer: &'a dyn IdentifierPseudonymizer,
	/// Binds replay carriers; `model` is the mapped model name.
	pub scope: StateScope<'a>,
	pub protocol: UpstreamProtocol,
	/// Used for a Responses or Gemini client that supplied no output limit.
	pub default_max_tokens: Option<NonZeroU64>,
}

/// Provider-specific settings that override model-derived rendering defaults.
#[derive(Debug, Clone, Copy)]
pub enum RenderSettings {
	ModelDefaults,
	/// Accepted reasoning levels; mapped after the request is edited.
	ChatCompletions(ReasoningControls),
	Responses(ReasoningControls),
	/// What the model takes, from a catalog; the client's effort is mapped onto it.
	Gemini(ThinkingControls),
	/// Cache marks a policy sets in place of the renderer's own.
	Messages(PromptCaching),
}

/// A parsed [`Conversation`] bound to its client and selected upstream.
///
/// Read or edit the conversation, then call [`PreparedRequest::render`]. Borrowed
/// request input and scope strings must outlive this value, including across awaits.
///
/// The selected upstream cannot be replaced after parsing:
///
/// ```compile_fail,E0616
/// use agent_llm_ir::{PreparedRequest, Upstream};
///
/// fn replace_upstream(request: &mut PreparedRequest<'_, '_>, upstream: Upstream<'_>) {
///     request.upstream = upstream;
/// }
/// ```
pub struct PreparedRequest<'a, 'c> {
	conversation: Conversation<'a>,
	diagnostics: Diagnostics,
	client: ClientProtocol,
	upstream: Upstream<'c>,
}

/// A rendered provider request and the state needed to convert its reply.
#[derive(Debug)]
pub struct ConvertedRequest {
	pub body: String,
	pub delivery: Delivery,
	pub dropped: Vec<&'static str>,
	/// A Messages upstream requires this `anthropic-beta` header, if present.
	pub anthropic_beta: Option<&'static str>,
	pub reply: ReplyPlan,
}

/// State carried from request conversion into reply conversion.
#[derive(Debug)]
pub struct ReplyPlan {
	/// Pass to a client writer that sends usage at the end of a stream only when asked.
	pub include_usage: bool,
	/// Restores flattened tool names when writing the client reply.
	pub routes: Vec<ToolRoute>,
	/// A Responses or Gemini client asked to see reasoning summaries.
	pub show_reasoning: bool,
	/// Stop sequences the provider can't take. Pass the decoder's events through
	/// `reply::stop::StopAt`, which ends the answer at the first match.
	pub emulated_stop_sequences: Vec<String>,
}

/// Parses a request for editing before it is rendered for the selected upstream.
/// Edits only see a clean parse; parsing and rendering problems are reported together.
pub fn prepare<'a, 'c>(
	body: &'a [u8],
	client: impl Into<ClientEndpoint>,
	upstream: Upstream<'c>,
) -> Result<PreparedRequest<'a, 'c>, RequestRefusal> {
	let client = client.into();
	let protocol = client.protocol();
	refuse_same_protocol(protocol, upstream.protocol)?;
	prepared(
		parse(body.into(), client, upstream, true),
		protocol,
		upstream,
	)
}

/// Prepares an already parsed request without reading or encoding its original JSON.
/// The parsed value is authoritative; duplicate-field rules belong to its initial parser.
pub fn prepare_parsed<'a, 'c>(
	parsed: &'a serde_json::Value,
	client: impl Into<ClientEndpoint>,
	upstream: Upstream<'c>,
) -> Result<PreparedRequest<'a, 'c>, RequestRefusal> {
	let client = client.into();
	let protocol = client.protocol();
	refuse_same_protocol(protocol, upstream.protocol)?;
	let input = RequestInput::Parsed(parsed);
	prepared(parse(input, client, upstream, true), protocol, upstream)
}

fn refuse_same_protocol(
	client: ClientProtocol,
	protocol: UpstreamProtocol,
) -> Result<(), RequestRefusal> {
	if matches!(
		(client, protocol),
		(ClientProtocol::Messages, UpstreamProtocol::Messages)
			| (
				ClientProtocol::Responses,
				UpstreamProtocol::Responses { .. }
			) | (
			ClientProtocol::ChatCompletions,
			UpstreamProtocol::ChatCompletions
		) | (ClientProtocol::Gemini, UpstreamProtocol::Gemini)
	) {
		return Err(RequestRefusal {
			problems: vec!["same-protocol requests are not supported by the conversion pipeline".into()],
		});
	}
	Ok(())
}

fn prepared<'a, 'c>(
	parsed: Parsed<'a>,
	client: ClientProtocol,
	upstream: Upstream<'c>,
) -> Result<PreparedRequest<'a, 'c>, RequestRefusal> {
	if !parsed.diagnostics.problems.is_empty() {
		let rendered = render_target(
			&parsed.conversation,
			&parsed.unread,
			parsed.diagnostics,
			upstream,
			RenderSettings::ModelDefaults,
			ResolvedCapabilities::for_model(upstream.protocol, upstream.scope.model),
		);
		return Err(RequestRefusal {
			problems: rendered.diagnostics.problems,
		});
	}
	debug_assert!(
		parsed.unread.is_empty(),
		"only a refused request leaves content unread"
	);
	Ok(PreparedRequest {
		conversation: parsed.conversation,
		diagnostics: parsed.diagnostics,
		client,
		upstream,
	})
}

impl<'a, 'c> PreparedRequest<'a, 'c> {
	#[must_use]
	pub const fn conversation(&self) -> &Conversation<'a> {
		&self.conversation
	}

	/// The conversation to edit before rendering, which validates the result.
	pub const fn conversation_mut(&mut self) -> &mut Conversation<'a> {
		&mut self.conversation
	}

	/// Validates the edited request and renders it for the selected upstream.
	/// Refuses settings for a different protocol or content the upstream cannot represent.
	pub fn render(self, settings: RenderSettings) -> Result<ConvertedRequest, RequestRefusal> {
		let Self {
			conversation,
			diagnostics,
			client,
			upstream,
		} = self;
		let matching = matches!(
			(settings, upstream.protocol),
			(RenderSettings::ModelDefaults, _)
				| (
					RenderSettings::ChatCompletions(_),
					UpstreamProtocol::ChatCompletions
				) | (
				RenderSettings::Responses(_),
				UpstreamProtocol::Responses { .. }
			) | (RenderSettings::Gemini(_), UpstreamProtocol::Gemini)
				| (RenderSettings::Messages(_), UpstreamProtocol::Messages)
		);
		if !matching {
			return Err(RequestRefusal {
				problems: vec!["render settings do not match the prepared target protocol".into()],
			});
		}
		let capabilities = ResolvedCapabilities::for_model(upstream.protocol, upstream.scope.model);
		let mut rendered = render_target(
			&conversation,
			&Unread::default(),
			diagnostics,
			upstream,
			settings,
			capabilities,
		);
		after_edits(&conversation, upstream.protocol, &mut rendered.diagnostics);
		let emulated_stop_sequences = if capabilities.native_stop_sequences {
			Vec::new()
		} else {
			conversation.settings.stop_sequences.clone()
		};
		// Avoid retaining unused buffer capacity while the request waits to be sent.
		let mut body = rendered.body;
		body.shrink_to_fit();
		Ok(ConvertedRequest {
			body,
			delivery: if conversation.settings.stream.unwrap_or(false) {
				Delivery::Streaming
			} else {
				Delivery::Buffered
			},
			dropped: rendered.diagnostics.finish()?,
			anthropic_beta: rendered.beta,
			reply: ReplyPlan {
				include_usage: conversation.settings.include_usage,
				routes: rendered.routes,
				show_reasoning: client != ClientProtocol::Messages
					&& conversation.settings.show_reasoning_summary,
				emulated_stop_sequences,
			},
		})
	}
}

/// Compares raw parsing paths and parsed values using production validation and
/// rendering. For tests and fuzzing.
#[doc(hidden)]
pub fn one_pass_parse_agrees(
	body: &[u8],
	client: impl Into<ClientEndpoint>,
	upstream: Upstream<'_>,
) -> bool {
	let client = client.into();
	let outcome = |input, one_pass| {
		let parsed = parse(input, client, upstream, one_pass);
		let mut rendered = render_target(
			&parsed.conversation,
			&parsed.unread,
			parsed.diagnostics,
			upstream,
			RenderSettings::ModelDefaults,
			ResolvedCapabilities::for_model(upstream.protocol, upstream.scope.model),
		);
		if client == ClientEndpoint::Responses {
			rendered.diagnostics.problems.sort();
			rendered.diagnostics.problems.dedup();
			rendered.diagnostics.dropped.sort_unstable();
		}
		(
			rendered.body,
			rendered.routes,
			rendered.beta,
			parsed.conversation.settings.stream,
			rendered.diagnostics.problems,
			rendered.diagnostics.dropped,
		)
	};
	if outcome(body.into(), true) != outcome(body.into(), false) {
		return false;
	}
	let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
		return true;
	};
	let Ok(canonical) = serde_json::to_vec(&value) else {
		unreachable!("a JSON value serializes")
	};
	outcome(canonical.as_slice().into(), true) == outcome(RequestInput::Parsed(&value), false)
}

struct Parsed<'a> {
	conversation: Conversation<'a>,
	diagnostics: Diagnostics,
	unread: Unread<'a>,
}

fn parse<'a>(
	body: RequestInput<'a>,
	client: ClientEndpoint,
	upstream: Upstream<'_>,
	one_pass: bool,
) -> Parsed<'a> {
	match client {
		ClientEndpoint::Messages => {
			let shapes = messages_shapes(upstream.protocol);
			let parsed = messages::parse_input(body, upstream.scope, shapes, one_pass);
			Parsed {
				conversation: parsed.conversation,
				diagnostics: parsed.diagnostics,
				unread: parsed.unread,
			}
		},
		ClientEndpoint::Responses => {
			let mut parsed = responses::parse(
				body,
				upstream.scope,
				upstream.default_max_tokens,
				upstream.protocol == UpstreamProtocol::Messages,
				one_pass,
			);
			match upstream.protocol {
				UpstreamProtocol::ChatCompletions => join_turns(&mut parsed.conversation),
				UpstreamProtocol::Gemini => responses_to_gemini(&mut parsed.conversation),
				_ => {},
			}
			Parsed {
				conversation: parsed.conversation,
				diagnostics: parsed.diagnostics,
				unread: Unread::default(),
			}
		},
		ClientEndpoint::ChatCompletions => {
			let mut parsed = completions_parse::parse(body, upstream, one_pass);
			join_turns(&mut parsed.conversation);
			Parsed {
				conversation: parsed.conversation,
				diagnostics: parsed.diagnostics,
				unread: Unread::default(),
			}
		},
		ClientEndpoint::Gemini(delivery) => {
			let mut parsed = gemini_parse::parse(
				body,
				upstream.scope,
				upstream.default_max_tokens,
				upstream.protocol == UpstreamProtocol::Messages,
				one_pass,
			);
			parsed.conversation.settings.stream = Some(delivery.is_streaming());
			Parsed {
				conversation: parsed.conversation,
				diagnostics: parsed.diagnostics,
				unread: Unread::default(),
			}
		},
	}
}

const fn messages_shapes(protocol: UpstreamProtocol) -> messages::Shapes {
	match protocol {
		UpstreamProtocol::Responses { .. } => messages::Shapes {
			system: "must be text",
			content: "must be a string or a list",
		},
		_ => messages::MUST_BE_A_LIST,
	}
}

struct Rendered {
	body: String,
	routes: Vec<ToolRoute>,
	beta: Option<&'static str>,
	diagnostics: Diagnostics,
}

fn render_target(
	conversation: &Conversation<'_>,
	unread: &Unread<'_>,
	mut diagnostics: Diagnostics,
	upstream: Upstream<'_>,
	settings: RenderSettings,
	capabilities: ResolvedCapabilities,
) -> Rendered {
	if !capabilities.cache_breakpoints && !conversation.cache_breakpoints.is_empty() {
		diagnostics.drop_field("cache_control");
	}
	match upstream.protocol {
		UpstreamProtocol::Messages => {
			let (rendered, diagnostics) = messages_render::render(
				conversation,
				diagnostics,
				upstream.scope.model,
				match settings {
					RenderSettings::Messages(caching) => Some(caching),
					_ => None,
				},
			);
			Rendered {
				body: rendered.body,
				routes: rendered.routes,
				beta: rendered.beta,
				diagnostics,
			}
		},
		UpstreamProtocol::Responses {
			safety_identifier,
			reasoning_context,
		} => {
			let (body, diagnostics) = responses_render::render(
				conversation,
				unread,
				diagnostics,
				responses_render::Options {
					identifier_pseudonymizer: upstream.identifier_pseudonymizer,
					model: upstream.scope.model,
					safety_identifier,
					reasoning_context,
					capabilities,
					reasoning: match settings {
						RenderSettings::Responses(controls) => controls,
						_ => ReasoningControls::default(),
					},
					cache_breakpoints: if capabilities.cache_breakpoints {
						&conversation.cache_breakpoints
					} else {
						&[]
					},
				},
			);
			Rendered {
				body,
				routes: Vec::new(),
				beta: None,
				diagnostics,
			}
		},
		UpstreamProtocol::ChatCompletions => {
			let (body, routes, diagnostics) = completions::render(
				conversation,
				unread,
				diagnostics,
				completions::Options {
					identifier_pseudonymizer: upstream.identifier_pseudonymizer,
					model: upstream.scope.model,
					capabilities,
					reasoning: match settings {
						RenderSettings::ChatCompletions(controls) => controls,
						_ => ReasoningControls::default(),
					},
				},
			);
			Rendered {
				body,
				routes,
				beta: None,
				diagnostics,
			}
		},
		UpstreamProtocol::Gemini => {
			let thinking = match settings {
				RenderSettings::Gemini(controls) => controls,
				_ => ThinkingControls::by_model_name(upstream.scope.model),
			};
			let (body, routes, diagnostics) =
				gemini::render_with_routes(conversation, unread, diagnostics, thinking);
			Rendered {
				body,
				routes,
				beta: None,
				diagnostics,
			}
		},
	}
}

/// Resolved request capabilities and the fallback for reasoning effort.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedCapabilities {
	pub native_stop_sequences: bool,
	pub reasoning_with_tools: bool,
	/// Used when reasoning controls supply no explicit effort levels.
	pub default_reasoning_effort: bool,
	pub cache_breakpoints: bool,
}

impl ResolvedCapabilities {
	/// What each model accepts, judged from its name.
	pub(crate) fn for_model(protocol: UpstreamProtocol, model: &str) -> Self {
		Self {
			native_stop_sequences: match protocol {
				UpstreamProtocol::Responses { .. } => false,
				UpstreamProtocol::ChatCompletions => !model.starts_with("gpt-5.4"),
				_ => true,
			},
			reasoning_with_tools: protocol != UpstreamProtocol::ChatCompletions
				|| effort_works_with_tools(model),
			default_reasoning_effort: !matches!(
				protocol,
				UpstreamProtocol::ChatCompletions | UpstreamProtocol::Responses { .. }
			) || accepts_effort(model),
			cache_breakpoints: matches!(protocol, UpstreamProtocol::Responses { .. })
				&& takes_cache_breakpoints(model),
		}
	}
}

/// Models before reasoning effort reject the parameter.
fn accepts_effort(model: &str) -> bool {
	!["gpt-3.5", "gpt-4", "chatgpt-4o"]
		.iter()
		.any(|prefix| model.starts_with(prefix))
}

/// Later GPT-5 Chat Completions requests must turn reasoning off alongside tools.
fn effort_works_with_tools(model: &str) -> bool {
	!model.starts_with("gpt-5")
		|| model == "gpt-5"
		|| model.starts_with("gpt-5-")
		|| model.starts_with("gpt-5.1")
		|| model.starts_with("gpt-5.2")
}

/// GPT-5.6 and later accept explicit Responses prompt cache breakpoints.
fn takes_cache_breakpoints(model: &str) -> bool {
	model
		.strip_prefix("gpt-")
		.and_then(|model| model.split('-').next())
		.and_then(|version| {
			let (major, minor) = version.split_once('.').unwrap_or((version, "0"));
			Some((major.parse::<u32>().ok()?, minor.parse::<u32>().ok()?))
		})
		.is_some_and(|version| version >= (5, 6))
}

const MAX_STOP_SEQUENCES: usize = 64;
const MAX_STOP_SEQUENCE_BYTES: usize = 4096;

/// Checks an edited request before it is rendered.
fn after_edits(
	conversation: &Conversation<'_>,
	upstream: UpstreamProtocol,
	diagnostics: &mut Diagnostics,
) {
	if conversation.settings.stop_sequences.len() > MAX_STOP_SEQUENCES {
		diagnostics.problem(format!(
			"at most {MAX_STOP_SEQUENCES} stop sequences are supported"
		));
	}
	if conversation
		.settings
		.stop_sequences
		.iter()
		.any(|stop| stop.len() > MAX_STOP_SEQUENCE_BYTES)
	{
		diagnostics.problem(format!(
			"a stop sequence may be at most {MAX_STOP_SEQUENCE_BYTES} bytes"
		));
	}
	if upstream == UpstreamProtocol::Messages && conversation.settings.max_tokens.is_none() {
		diagnostics.problem("setting `max_tokens` is required by Messages");
	}
	for message in &conversation.messages {
		for block in &message.blocks {
			let valid = match message.role {
				Role::User => matches!(
					&block.content,
					Content::Text(_) | Content::Image(_) | Content::DataImage(_) | Content::ToolResult { .. }
				),
				Role::Assistant => matches!(
					&block.content,
					Content::Text(_) | Content::ToolCall { .. } | Content::Reasoning { .. }
				),
				Role::System => matches!(&block.content, Content::Text(_)),
			};
			if !valid {
				let turn = match message.role {
					Role::User => "user",
					Role::Assistant => "model",
					Role::System => "system",
				};
				diagnostics.problem(format!(
					"`{}` doesn't belong in a {turn} turn",
					block.id.path()
				));
			}
		}
	}
}

/// Gemini signatures arrive after signed text in Responses history, but the
/// renderer needs an explicit link before the signed block.
pub(super) fn responses_to_gemini(conversation: &mut Conversation<'_>) {
	join_turns(conversation);
	for message in &mut conversation.messages {
		if message.role != Role::Assistant {
			continue;
		}
		let blocks = &mut message.blocks;
		for at in 0..blocks.len() {
			let Content::Reasoning { carrier, .. } = &blocks[at].content else {
				continue;
			};
			let signed_text = carrier.opened().is_ok_and(signs_text);
			let after_text = at > 0 && matches!(blocks[at - 1].content, Content::Text(_));
			let linked = if signed_text && after_text {
				blocks.swap(at - 1, at);
				at - 1
			} else {
				at
			};
			let target = blocks.get(linked + 1).map(|block| block.id);
			if let Content::Reasoning { next, .. } = &mut blocks[linked].content {
				*next = target;
			}
		}
	}
}

/// Responses gives each item its own message. Targets with whole turns want a turn's
/// calls together, and all their results together.
pub(super) fn join_turns(conversation: &mut Conversation<'_>) {
	let mut messages: Vec<Message<'_>> = Vec::with_capacity(conversation.messages.len());
	for message in std::mem::take(&mut conversation.messages) {
		match messages.last_mut() {
			Some(last) if same_turn(last, &message) => last.blocks.extend(message.blocks),
			_ => messages.push(message),
		}
	}
	conversation.messages = messages;
}

fn same_turn(last: &Message<'_>, next: &Message<'_>) -> bool {
	let only_results = |message: &Message<'_>| {
		let result = |block: &Block<'_>| matches!(block.content, Content::ToolResult { .. });
		message.blocks.iter().all(result)
	};
	match (last.role, next.role) {
		(Role::Assistant, Role::Assistant) => true,
		(Role::User, Role::User) => only_results(last) && only_results(next),
		_ => false,
	}
}
