//! Native endpoint handling, IR conversion and policy fallbacks, selected before the body is
//! read and completed by [`destination`] once the model is known.

use agent_core::prelude::Strng;
use agent_llm_ir::{ClientProtocol, UpstreamProtocol};

use crate::llm::policy::{Policy, PromptGuard, RequestGuardKind, ResponseGuardKind};
use crate::llm::{AIError, AIProvider, ChatFormat, NamedAIProvider, RouteType};
use crate::store::LLMRequestPolicies;
use crate::telemetry::log::RequestLog;
use crate::types::frontend::DatabaseLlmMode;

/// Why a request stays on the legacy converter. A fixed set, so it can label telemetry.
///
/// Remove a fallback only after its legacy behavior is implemented and covered by selection
/// and request/reply tests. Native endpoints and unsupported chat destinations are selected
/// separately. Guard integration must preserve existing scopes, failure modes, mutations,
/// rejection responses, audit behavior and telemetry; streaming masking is not legacy behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ineligible {
	/// Bedrock and Vertex attempts are not wired to the IR. Integrate their endpoint and model
	/// selection, authentication/signing, and request/reply handling, adding protocol adapters
	/// where their wire formats are not already supported.
	Provider,
	/// Request-side token counting is not wired to IR attempts. Compute the legacy-equivalent
	/// estimate after request policies, use it for rate-limit admission, and retain settlement
	/// from the provider's response usage.
	Tokenize,
	/// A prompt guard calls a webhook.
	/// Adapt IR requests and replies to the webhook's message/choice views and apply its
	/// returned masks or rejections. Preserve the existing buffered and streaming behavior.
	GuardWebhook,
	/// A request guard calls OpenAI moderation.
	/// Add asynchronous evaluation of the IR request's scoped text and enforce or audit the
	/// service's verdict before sending the request upstream.
	GuardModeration,
	/// A request or response guard calls Bedrock Guardrails. Evaluate scoped IR text and map
	/// blocked/anonymized assessments back to rejections or edits, as legacy does. This is
	/// separate from supporting a Bedrock inference provider.
	GuardBedrock,
	/// A request or response guard calls Google Model Armor. Connect its sanitization calls to
	/// scoped IR text and map the existing enforce/audit verdicts into request/reply handling.
	GuardModelArmor,
	/// A request or response guard calls Azure Content Safety. Connect its configured checks
	/// to scoped IR text and apply the existing thresholds and enforce/audit verdicts.
	GuardContentSafety,
	/// A Messages passthrough with a prompt caching policy, which legacy marks and a passthrough
	/// sends as the client wrote it.
	/// Apply the policy's cache-breakpoint rules to native Messages requests while preserving
	/// provider-only fields and legacy's treatment of client cache controls. Converted Messages
	/// requests already support it.
	PromptCaching,
	/// CEL needs completion/tool-call content, or database logging requests full conversations.
	/// Capture the required request and reply content for buffered and streaming exchanges and
	/// populate the log/CEL/database views, respecting capture flags and limits.
	ContentCapture,
	/// The provider serves the client's own protocol for this model, and a prompt guard is
	/// configured. A passthrough does not expose conversation content for checks or edits.
	/// Add guard evaluation and supported mutations for native requests and buffered/streamed
	/// replies while preserving provider-only content and existing policy behavior.
	PassthroughGuard,
}

impl Ineligible {
	pub fn as_str(self) -> &'static str {
		match self {
			Ineligible::Provider => "provider",
			Ineligible::Tokenize => "tokenize",
			Ineligible::GuardWebhook => "prompt_guard_webhook",
			Ineligible::GuardModeration => "prompt_guard_openai_moderation",
			Ineligible::GuardBedrock => "prompt_guard_bedrock_guardrails",
			Ineligible::GuardModelArmor => "prompt_guard_google_model_armor",
			Ineligible::GuardContentSafety => "prompt_guard_azure_content_safety",
			Ineligible::PromptCaching => "prompt_caching",
			Ineligible::ContentCapture => "content_capture",
			Ineligible::PassthroughGuard => "passthrough_prompt_guard",
		}
	}
}

/// Which converter served an attempt. Recorded only while the IR is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Converter {
	/// A non-chat endpoint uses its existing protocol-specific handler.
	Native,
	LlmIr,
	/// The provider serves the client's own protocol, so the request and reply go unconverted.
	LlmIrPassthrough,
	Legacy(Ineligible),
}

impl Converter {
	pub fn name(self) -> &'static str {
		match self {
			Converter::Native => "native",
			Converter::LlmIr => "llm_ir",
			Converter::LlmIrPassthrough => "llm_ir_passthrough",
			Converter::Legacy(_) => "legacy",
		}
	}

	pub fn legacy_reason(self) -> Option<&'static str> {
		match self {
			Converter::Native | Converter::LlmIr | Converter::LlmIrPassthrough => None,
			Converter::Legacy(reason) => Some(reason.as_str()),
		}
	}
}

/// A chat attempt selected for IR conversion or same-protocol passthrough.
pub struct Selected {
	/// Binds replay carriers: load-balanced entries and retries in one backend share them.
	pub(super) backend: Strng,
	pub(super) client: ClientProtocol,
}

pub(crate) enum Selection {
	/// Outside chat conversion; keep endpoint-specific request, policy and reply handling.
	Native,
	Ir(Selected),
}

/// Checks what is configured, before the body is read.
pub(crate) fn select(
	route_type: RouteType,
	provider: &NamedAIProvider,
	policies: &LLMRequestPolicies,
	backend: Strng,
	log: Option<&RequestLog>,
) -> Result<Selection, Ineligible> {
	let client = match route_type {
		RouteType::Messages => ClientProtocol::Messages,
		RouteType::Responses => ClientProtocol::Responses,
		RouteType::Completions => ClientProtocol::ChatCompletions,
		// The URL says whether it streams, and the body is read only once it is selected.
		RouteType::GenerateContent => ClientProtocol::Gemini,
		_ => return Ok(Selection::Native),
	};
	check_configuration(provider, policies, log)?;
	Ok(Selection::Ir(Selected { backend, client }))
}

/// Where a selected request goes.
pub(super) enum Target {
	Convert(UpstreamProtocol),
	/// The provider serves the client's own protocol, named here.
	Passthrough(UpstreamProtocol),
}

#[derive(Debug)]
pub(super) enum DestinationError {
	Legacy(Ineligible),
	Refused(AIError),
}

/// Decides where a selected request goes once its model is known: `model` is the one the
/// provider serves, after overrides and aliases. Returns it with where it goes.
pub(super) fn destination(
	provider: &AIProvider,
	client: ClientProtocol,
	model: Strng,
	policy: Option<&Policy>,
	catalog: agent_llm::model_catalog::Catalog<'_>,
) -> Result<(Strng, Target), DestinationError> {
	let target = provider
		.upstream_for(client, &model, catalog)
		.map_err(DestinationError::Refused)?;

	if let Target::Passthrough(protocol) = target
		&& let Some(policy) = policy
	{
		if policy.prompt_guard.is_some() {
			return Err(DestinationError::Legacy(Ineligible::PassthroughGuard));
		}
		if protocol == UpstreamProtocol::Messages && policy.prompt_caching.is_some() {
			return Err(DestinationError::Legacy(Ineligible::PromptCaching));
		}
	}
	Ok((model, target))
}

impl AIProvider {
	/// Where a request goes: native Gemini first, as legacy prefers it even over a Chat
	/// Completions passthrough; then the client's own protocol, which is a passthrough; then
	/// Messages, Responses and Chat Completions.
	fn upstream_for(
		&self,
		client: ClientProtocol,
		model: &str,
		catalog: agent_llm::model_catalog::Catalog<'_>,
	) -> Result<Target, AIError> {
		let supported = self.supported_chat_formats(model, catalog);
		let own = match client {
			ClientProtocol::Messages => ChatFormat::AnthropicMessages,
			ClientProtocol::Responses => ChatFormat::OpenAIResponses,
			ClientProtocol::ChatCompletions => ChatFormat::OpenAICompletions,
			ClientProtocol::Gemini => ChatFormat::VertexGemini,
		};
		if supported.contains(&ChatFormat::VertexGemini) && own != ChatFormat::VertexGemini {
			return Ok(Target::Convert(UpstreamProtocol::Gemini));
		}
		if supported.contains(&own) {
			return Ok(Target::Passthrough(match client {
				ClientProtocol::Messages => UpstreamProtocol::Messages,
				// Nothing is rendered, so the capabilities don't matter.
				ClientProtocol::Responses => UpstreamProtocol::Responses {
					safety_identifier: false,
					reasoning_context: false,
				},
				ClientProtocol::ChatCompletions => UpstreamProtocol::ChatCompletions,
				ClientProtocol::Gemini => UpstreamProtocol::Gemini,
			}));
		}
		if supported.contains(&ChatFormat::AnthropicMessages) {
			return Ok(Target::Convert(UpstreamProtocol::Messages));
		}
		if supported.contains(&ChatFormat::OpenAIResponses) {
			return Ok(Target::Convert(UpstreamProtocol::Responses {
				// Only OpenAI's own endpoint is known to take it.
				safety_identifier: matches!(self, AIProvider::OpenAI(_)),
				// reasoning_context is a protocol option and requires OpenAI's own endpoint.
				// Only this model and snapshot are verified to accept all_turns.
				reasoning_context: matches!(self, AIProvider::OpenAI(_))
					&& matches!(model, "gpt-5.5" | "gpt-5.5-2026-04-23"),
			}));
		}
		if supported.contains(&ChatFormat::OpenAICompletions) {
			return Ok(Target::Convert(UpstreamProtocol::ChatCompletions));
		}
		Err(AIError::RequestRefused {
			problems: vec![format!(
				"provider `{}` cannot serve {client:?} requests for model `{model}` through LLM IR: advertised formats {:?}, available chat formats {supported:?}",
				self.provider(),
				self.supported_formats(model),
			)],
		})
	}
}

/// Every field of each input is named, so a new field fails to build until someone decides
/// whether the IR supports it.
fn check_configuration(
	provider: &NamedAIProvider,
	policies: &LLMRequestPolicies,
	log: Option<&RequestLog>,
) -> Result<(), Ineligible> {
	let NamedAIProvider {
		provider, tokenize, ..
	} = provider;
	match provider {
		AIProvider::OpenAI(_)
		| AIProvider::Anthropic(_)
		| AIProvider::Gemini(_)
		| AIProvider::Azure(_)
		| AIProvider::Copilot(_)
		| AIProvider::Custom(_) => {},
		// Their paths, signing and model families are not yet wired to IR attempts.
		AIProvider::Bedrock(_) | AIProvider::Vertex(_) => return Err(Ineligible::Provider),
	}
	if *tokenize {
		return Err(Ineligible::Tokenize);
	}

	// Token rate limits settle from response usage, as on legacy without `tokenize`.
	if let LLMRequestPolicies { llm: Some(llm), .. } = policies {
		check_policy(llm)?;
	}

	if let Some(log) = log {
		if log.cel.cel_context.needs_llm_completion() || log.cel.cel_context.needs_llm_tool_calls() {
			return Err(Ineligible::ContentCapture);
		}
		match log.database_llm {
			None | Some(DatabaseLlmMode::Metadata) => {},
			Some(DatabaseLlmMode::Full) => return Err(Ineligible::ContentCapture),
		}
	}
	Ok(())
}

fn check_policy(policy: &Policy) -> Result<(), Ineligible> {
	// Only regex guards run on the IR. Each other kind keeps its own reason until its request
	// and reply integration preserves the existing policy behavior; then its check can be removed.
	if let Policy {
		prompt_guard: Some(PromptGuard {
			request, response, ..
		}),
		..
	} = policy
	{
		for guard in request {
			match guard.kind {
				RequestGuardKind::Webhook(_) => return Err(Ineligible::GuardWebhook),
				RequestGuardKind::OpenAIModeration(_) => return Err(Ineligible::GuardModeration),
				RequestGuardKind::BedrockGuardrails(_) => return Err(Ineligible::GuardBedrock),
				RequestGuardKind::GoogleModelArmor(_) => return Err(Ineligible::GuardModelArmor),
				RequestGuardKind::AzureContentSafety(_) => return Err(Ineligible::GuardContentSafety),
				_ => {},
			}
		}
		for guard in response {
			match guard.kind {
				ResponseGuardKind::Webhook(_) => return Err(Ineligible::GuardWebhook),
				ResponseGuardKind::BedrockGuardrails(_) => return Err(Ineligible::GuardBedrock),
				ResponseGuardKind::GoogleModelArmor(_) => return Err(Ineligible::GuardModelArmor),
				ResponseGuardKind::AzureContentSafety(_) => return Err(Ineligible::GuardContentSafety),
				_ => {},
			}
		}
	}
	Ok(())
}
