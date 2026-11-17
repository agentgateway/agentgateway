//! Request dispatch, conversion, and per-attempt state for reply handling.

use std::borrow::Cow;
use std::num::NonZeroU64;
use std::sync::Arc;

use ::http::request::Parts;
use ::http::{HeaderValue, header};
use agent_core::prelude::Strng;
use agent_llm_ir::{
	self as llm_ir, ClientEndpoint, ClientProtocol, Content, Conversation, Delivery, Effort,
	LeadingModel, PreparedRequest, RenderSettings, Role, Settings, StateScope, Text,
	ThinkingControls, ThinkingLevel, ThinkingLevels, Upstream, UpstreamProtocol,
};
use bytes::Bytes;
use serde::Deserialize;
use serde_json::value::RawValue;
use tracing::debug;

use super::passthrough::{ReadBody, raw_json};
use super::selection::{self, Target};
use super::{
	Attempt, AttemptContext, Converter, Ineligible, MESSAGES_DEFAULT_MAX_TOKENS, ReplyState,
	Selected, enrichment, guard,
};
use crate::http::{Body, Request};
use crate::json::ParsedJson;
use crate::llm::policy::Policy;
use crate::llm::{
	AIError, AIProvider, CacheTokenConvention, InputFormat, LLMRequest, LLMRequestParams,
	ProviderState, RequestResult, RouteType, SimpleChatCompletionMessage, model_router, openai,
	read_decoded_body,
};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;

impl AIProvider {
	/// Serves a selected request: reads the body once, decides from its model where it goes,
	/// and converts it unless it stays on legacy or the provider serves the client's protocol.
	pub async fn process_request_ir(
		&self,
		selected: Selected,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		req: Request,
		log: &mut Option<&mut RequestLog>,
		catalog: agent_llm::model_catalog::Catalog<'_>,
	) -> Result<RequestResult, AIError> {
		let endpoint = match client_endpoint(selected.client, req.uri()) {
			Ok(endpoint) => endpoint,
			Err(rejection) => return Ok(RequestResult::Rejected(rejection)),
		};
		let mut client = self.read_client_body(endpoint, policies, req, log).await?;
		let (read, client_model) = read_model(
			client.cached.as_ref(),
			&client.client_bytes,
			client.parts.uri.path(),
			endpoint.protocol(),
			client.mutated,
		)?;
		let (read, served) = match &client_model.name {
			Some(name) => {
				let served = self.served_model(name, &mut client.parts, client.mutated, policies);
				(read, served)
			},
			None => {
				drop(read);
				let served = self.unnamed_model(policies, &mut client.parts)?;
				// The IR reads a model from the body, as legacy, which writes this one in.
				if endpoint.protocol() != ClientProtocol::Gemini {
					client.name_model(&served)?;
				}
				(ReadBody::Unread, served)
			},
		};

		let target = selection::destination(self, selected.client, served, policies, catalog);
		let (model, protocol) = match target {
			Ok((model, Target::Convert(protocol))) => (model, protocol),
			Ok((model, Target::Passthrough(protocol))) => {
				let write_model = client_model.in_body && client_model.name.as_ref() != Some(&model);
				let llm_request = self.passthrough_llm_request(&client, read, protocol, &model, log)?;
				return self.process_passthrough(client, llm_request, write_model, protocol, policies, log);
			},
			Err(selection::DestinationError::Refused(error)) => return Err(error),
			Err(selection::DestinationError::Legacy(reason)) => {
				return self
					.process_legacy_request(reason, client, backend_info, policies, log, catalog)
					.await;
			},
		};

		let provider = self.provider();
		let mut prepared = prepare(&client, &selected.backend, protocol, &provider, &model)?;
		let original_model = original_model(&client.parts, &model, log);
		let mut llm_request = llm_request(
			selected.client,
			protocol,
			&provider,
			&model,
			original_model,
			&prepared.conversation().settings,
		);
		let rejection = before_render(
			policies,
			prepared.conversation_mut(),
			backend_info,
			&client.parts,
			log,
		)?;
		if needs_prompt(log) {
			llm_request.prompt = Some(Arc::new(prompt(selected.client, prepared.conversation())));
		}
		if let Some((response, guardrail)) = rejection {
			if let Some(log) = log.as_deref_mut() {
				log.llm_request = Some(llm_request);
			}
			return Ok(RequestResult::GuardrailRejected {
				response,
				guardrail,
			});
		}
		let settings = render_settings(protocol, &provider, &model, policies, catalog);
		let converted = prepared.render(settings).map_err(super::refused)?;
		if !converted.dropped.is_empty() {
			debug!(dropped = ?converted.dropped, "llm-ir: fields the upstream can't take were dropped");
		}
		debug_assert_eq!(llm_request.streaming, converted.delivery.is_streaming());

		let ir_reply = Some(ReplyState::Convert(Attempt::new(
			AttemptContext {
				backend: selected.backend,
				provider,
				model,
				client: selected.client,
				upstream: protocol,
				created: unix_now(),
				fallback_reply_id: uuid::Uuid::new_v4().simple().to_string(),
			},
			converted.reply,
		)));
		let request = provider_request(
			client,
			converted.body,
			converted.anthropic_beta,
			self.inline_moderation(protocol),
			policies,
			log,
		)?;
		Ok(RequestResult::Success {
			request,
			llm_request,
			ir_reply,
			upstream_route_type: route_type(protocol),
		})
	}

	/// The model the provider serves for the one the client named.
	fn served_model(
		&self,
		name: &Strng,
		parts: &mut Parts,
		mutated: bool,
		policies: Option<&Policy>,
	) -> Strng {
		if mutated {
			// Mutating wrote the provider's model in and recorded the client's.
			aliased(policies, name.clone())
		} else {
			self.mapped_model(policies, parts, name)
		}
	}

	/// Resolves the model as legacy does for a body that names none: the provider override,
	/// else a model the path names; then any alias. Mutating writes one in, so this is reached
	/// only without mutations, and a Gemini path's model is already the client's.
	fn unnamed_model(&self, policies: Option<&Policy>, parts: &mut Parts) -> Result<Strng, AIError> {
		let path_model = agent_llm::types::detect::extract_model_from_path(parts.uri.path());
		if let Some(path_model) = &path_model {
			model_router::OriginalModel::record(&mut parts.extensions, path_model.to_string());
		}
		let model = self
			.override_model()
			.or(path_model)
			.ok_or_else(|| AIError::MissingField("model not specified".into()))?;
		Ok(aliased(policies, model))
	}

	/// The OpenAI provider's inline moderation, which legacy writes into each Chat Completions
	/// and Responses request it sends, replacing the client's.
	pub(super) fn inline_moderation(
		&self,
		protocol: UpstreamProtocol,
	) -> Option<&openai::ModerationParam> {
		match (self, protocol) {
			(
				AIProvider::OpenAI(provider),
				UpstreamProtocol::ChatCompletions | UpstreamProtocol::Responses { .. },
			) => provider.moderation.as_ref(),
			_ => None,
		}
	}

	/// Reads the body once and applies the policy's body mutations to it. Legacy, if it
	/// continues, mutates the original bytes itself.
	async fn read_client_body(
		&self,
		endpoint: ClientEndpoint,
		policies: Option<&Policy>,
		req: Request,
		log: &mut Option<&mut RequestLog>,
	) -> Result<ClientBody, AIError> {
		let (mut parts, body, bytes, mut cached) = read_decoded_body(req).await?;
		let mutated = match policies.filter(|policy| policy.has_request_body_mutations()) {
			Some(policy) => Some(self.mutate_body(
				policy,
				&mut parts,
				&bytes,
				// Mutating takes the router's parse, which no longer matches the bytes.
				cached.take(),
				endpoint.protocol(),
				log,
			)?),
			None => None,
		};
		Ok(ClientBody {
			endpoint,
			parts,
			body,
			mutated: mutated.is_some(),
			client_bytes: mutated.map_or_else(|| bytes.clone(), Bytes::from),
			bytes,
			cached,
		})
	}

	/// Hands a selected request the IR leaves alone to the legacy converter, with the bytes and
	/// parse it arrived with. Selection already rules out `tokenize`.
	async fn process_legacy_request(
		&self,
		reason: Ineligible,
		client: ClientBody,
		backend_info: &crate::http::auth::BackendInfo,
		policies: Option<&Policy>,
		log: &mut Option<&mut RequestLog>,
		catalog: agent_llm::model_catalog::Catalog<'_>,
	) -> Result<RequestResult, AIError> {
		debug!(
			reason = reason.as_str(),
			"llm-ir: request stays on the legacy converter"
		);
		if let Some(log) = log.as_deref_mut() {
			log.llm_converter = Some(Converter::Legacy(reason));
		}
		let ClientBody {
			endpoint,
			parts,
			mut body,
			bytes,
			cached,
			..
		} = client;
		body.replace_bytes(bytes);
		if let Some(cached) = cached {
			body.insert_extension(cached);
		}
		let req = Request::from_parts(parts, body);
		match endpoint.protocol() {
			ClientProtocol::Messages => {
				Box::pin(self.process_messages_request(backend_info, policies, req, false, log, catalog))
					.await
			},
			ClientProtocol::Responses => {
				Box::pin(self.process_responses_request(backend_info, policies, req, false, log, catalog))
					.await
			},
			ClientProtocol::ChatCompletions => {
				Box::pin(self.process_completions_request(backend_info, policies, req, false, log, catalog))
					.await
			},
			ClientProtocol::Gemini => {
				Box::pin(self.process_gemini_request(backend_info, policies, req, false, log, catalog))
					.await
			},
		}
	}

	/// Resolves the model as legacy does for a body that names one: the provider override,
	/// else the body model; then any alias.
	fn mapped_model(&self, policies: Option<&Policy>, parts: &mut Parts, body_model: &str) -> Strng {
		model_router::OriginalModel::record(&mut parts.extensions, body_model.to_string());
		let model = self
			.override_model()
			.unwrap_or_else(|| agent_core::strng::new(body_model));
		aliased(policies, model)
	}

	/// Applies the policy's overrides, transformations and defaults to the client's JSON as
	/// legacy does: after the provider's model, or for Gemini the path's, is written in, so a
	/// mutation of `model` sees and can replace the model that will be served.
	fn mutate_body(
		&self,
		policy: &Policy,
		parts: &mut Parts,
		bytes: &[u8],
		cached: Option<ParsedJson>,
		client: ClientProtocol,
		log: &mut Option<&mut RequestLog>,
	) -> Result<Vec<u8>, AIError> {
		let mut request = match cached {
			Some(ParsedJson(value)) => value,
			None => serde_json::from_slice(bytes)
				.map_err(|err| AIError::RequestParsing(input_format(client), err))?,
		};
		self.set_provider_request_model(parts, &mut request, client == ClientProtocol::Gemini)?;
		let mut request = policy.apply_request_body_mutations(request, log)?;
		self.finalize_request_model(&mut request)?;
		serde_json::to_vec(&request).map_err(AIError::RequestMarshal)
	}
}

/// The request's policy point: after the request is parsed and before it is rendered, which
/// validates what the policies changed. Returns a prompt guard's rejection.
fn before_render(
	policies: Option<&Policy>,
	conversation: &mut Conversation<'_>,
	backend_info: &crate::http::auth::BackendInfo,
	parts: &Parts,
	log: &Option<&mut RequestLog>,
) -> Result<Option<(crate::http::Response, &'static str)>, AIError> {
	let Some(policy) = policies else {
		return Ok(None);
	};
	if let Some(prompts) = &policy.prompts {
		enrichment::apply(prompts, conversation)?;
	}
	let Some(guard) = &policy.prompt_guard else {
		return Ok(None);
	};
	let client =
		PolicyClient::new(backend_info.inputs.clone()).with_parent_extensions(&parts.extensions);
	let guardrails = log.as_ref().map(|log| log.guardrails.clone());
	Ok(guard::request(
		guard,
		conversation,
		&client,
		guardrails.as_ref(),
	))
}

pub(super) fn needs_prompt(log: &Option<&mut RequestLog>) -> bool {
	log
		.as_ref()
		.is_some_and(|log| log.cel.cel_context.needs_llm_prompt())
}

/// What CEL reads as `llm.prompt`: the system text, then each message's text, after enrichment
/// and guards. Legacy reads it from the client's own format, which the IR doesn't keep, so the
/// roles are the IR's but for Gemini's `model`, and a turn of only tool calls or results has
/// no text.
fn prompt(
	client: ClientProtocol,
	conversation: &Conversation<'_>,
) -> Vec<SimpleChatCompletionMessage> {
	let system = conversation
		.system
		.iter()
		.flatten()
		.map(Text::read)
		.collect::<Vec<_>>()
		.join("\n");
	let system = (!system.is_empty()).then(|| SimpleChatCompletionMessage {
		role: "system".into(),
		content: system.into(),
	});
	let separator = if client == ClientProtocol::Responses {
		"\n"
	} else {
		" "
	};
	let messages = conversation.messages.iter().map(|message| {
		let role = match (message.role, client) {
			(Role::System, _) => "system",
			(Role::User, _) => "user",
			(Role::Assistant, ClientProtocol::Gemini) => "model",
			(Role::Assistant, _) => "assistant",
		};
		let content = message
			.blocks
			.iter()
			.filter_map(|block| match &block.content {
				Content::Text(text) => Some(text.read()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join(separator);
		SimpleChatCompletionMessage {
			role: role.into(),
			content: content.into(),
		}
	});
	system.into_iter().chain(messages).collect()
}

impl ClientBody {
	fn name_model(&mut self, model: &str) -> Result<(), AIError> {
		let client = self.endpoint.protocol();
		let mut fields: indexmap::IndexMap<String, Box<RawValue>> =
			serde_json::from_slice(&self.client_bytes)
				.map_err(|err| AIError::RequestParsing(input_format(client), err))?;
		fields.insert("model".into(), raw_json(&model));
		self.client_bytes = serde_json::to_vec(&fields)
			.map_err(AIError::RequestMarshal)?
			.into();
		self.cached = None;
		Ok(())
	}
}

/// The client's request as read, after any policy mutations.
pub(super) struct ClientBody {
	pub(super) endpoint: ClientEndpoint,
	pub(super) parts: Parts,
	pub(super) body: Body,
	/// As the client sent them, for legacy.
	bytes: Bytes,
	/// The model router's parse of `bytes`, unless mutating took it.
	cached: Option<ParsedJson>,
	/// After any policy mutations.
	pub(super) client_bytes: Bytes,
	mutated: bool,
}

/// The model a client named.
struct ClientModel {
	name: Option<Strng>,
	/// Whether the body a passthrough sends names `name`: not when the URL names it, nor when a
	/// policy mutation wrote the served model in.
	in_body: bool,
}

/// The client's protocol and how it asked for its reply. A Gemini stream that isn't SSE is
/// rejected as legacy rejects it: the JSON-array stream can't be read as it arrives.
fn client_endpoint(
	client: ClientProtocol,
	uri: &::http::Uri,
) -> Result<ClientEndpoint, crate::http::Response> {
	Ok(match client {
		ClientProtocol::Messages => ClientEndpoint::Messages,
		ClientProtocol::Responses => ClientEndpoint::Responses,
		ClientProtocol::ChatCompletions => ClientEndpoint::ChatCompletions,
		ClientProtocol::Gemini if !uri.path().ends_with(":streamGenerateContent") => {
			ClientEndpoint::Gemini(Delivery::Buffered)
		},
		ClientProtocol::Gemini if crate::llm::query_requests_sse(uri) => {
			ClientEndpoint::Gemini(Delivery::Streaming)
		},
		ClientProtocol::Gemini => {
			return Err(crate::llm::google_invalid_argument(
				"streamGenerateContent requires alt=sse; the JSON-array streaming variant is not supported",
			));
		},
	})
}

/// Reads the model the client named, and the body only as far as that takes, from the router's
/// parse when there is one.
fn read_model<'a>(
	cached: Option<&'a ParsedJson>,
	client_bytes: &'a [u8],
	path: &str,
	client: ClientProtocol,
	mutated: bool,
) -> Result<(ReadBody<'a>, ClientModel), AIError> {
	let parsed = cached
		.map(|ParsedJson(parsed)| llm_ir::ParsedRequest::new(parsed))
		.transpose()
		.map_err(super::refused)?;
	if client == ClientProtocol::Gemini && !mutated {
		// The Gemini body has no model; the URL names it.
		let read = parsed.map_or(ReadBody::Unread, ReadBody::Parsed);
		let name = agent_llm::types::detect::extract_model_from_path(path);
		return Ok((
			read,
			ClientModel {
				name,
				in_body: false,
			},
		));
	}
	let (read, name) = match parsed {
		Some(parsed) => {
			let name = parsed.model().map_err(super::refused)?;
			(ReadBody::Parsed(parsed), name.map(agent_core::strng::new))
		},
		None => match llm_ir::leading_model(client_bytes).map_err(super::refused)? {
			LeadingModel::Read(name) => (ReadBody::Unread, name.map(agent_core::strng::new)),
			LeadingModel::AfterConversation => {
				let fields = llm_ir::RequestFields::read(client_bytes).map_err(super::refused)?;
				let name = fields.model().map_err(super::refused)?;
				(ReadBody::Fields(fields), name.map(agent_core::strng::new))
			},
		},
	};
	Ok((
		read,
		ClientModel {
			name,
			in_body: !mutated,
		},
	))
}

/// Parses the client's request for conversion, from the router's parse when there is one.
fn prepare<'a>(
	client: &'a ClientBody,
	backend: &'a str,
	protocol: UpstreamProtocol,
	provider: &'a str,
	model: &'a str,
) -> Result<PreparedRequest<'a, 'a>, AIError> {
	let upstream = Upstream {
		identifier_pseudonymizer: &super::identifier::IdentifierHasher,
		scope: StateScope {
			backend,
			provider,
			model,
		},
		protocol,
		default_max_tokens: (protocol == UpstreamProtocol::Messages)
			.then_some(MESSAGES_DEFAULT_MAX_TOKENS),
	};
	match &client.cached {
		Some(ParsedJson(parsed)) => llm_ir::prepare_parsed(parsed, client.endpoint, upstream),
		None => llm_ir::prepare(&client.client_bytes, client.endpoint, upstream),
	}
	.map_err(super::refused)
}

/// The request sent to the provider: the rendered body after the policy's final
/// transformations, with the headers the body needs.
fn provider_request(
	client: ClientBody,
	rendered: String,
	anthropic_beta: Option<&'static str>,
	moderation: Option<&openai::ModerationParam>,
	policies: Option<&Policy>,
	log: &mut Option<&mut RequestLog>,
) -> Result<Request, AIError> {
	let ClientBody {
		mut parts,
		mut body,
		..
	} = client;
	if let Some(beta) = anthropic_beta {
		add_anthropic_beta(&mut parts.headers, beta);
	}
	parts.headers.remove(header::CONTENT_LENGTH);
	parts.headers.remove(header::TRANSFER_ENCODING);
	let rendered = match moderation {
		Some(moderation) => {
			let mut fields: indexmap::IndexMap<String, Box<RawValue>> =
				serde_json::from_str(&rendered).expect("the IR renders a JSON object");
			fields.insert("moderation".into(), raw_json(moderation));
			serde_json::to_vec(&fields).map_err(AIError::RequestMarshal)?
		},
		None => rendered.into_bytes(),
	};
	let rendered = match policies {
		Some(policy) => policy.apply_final_transformations(rendered, log)?,
		None => rendered,
	};
	body.replace_bytes(rendered.into());
	Ok(Request::from_parts(parts, body))
}

/// What the request log and rate limits record for a converted or passthrough request.
pub(super) fn llm_request(
	client: ClientProtocol,
	protocol: UpstreamProtocol,
	provider: &Strng,
	model: &Strng,
	original_model: Option<Strng>,
	settings: &Settings<'_>,
) -> LLMRequest {
	LLMRequest {
		input_tokens: None,
		input_format: input_format(client),
		// The IR's usage always counts cached tokens in the input.
		cache_convention: CacheTokenConvention::InputIncludesCache,
		request_model: model.clone(),
		original_model,
		provider: provider.clone(),
		streaming: settings.stream.unwrap_or(false),
		params: request_params(settings),
		prompt: None,
		provider_state: (protocol == UpstreamProtocol::Gemini).then_some(ProviderState::VertexGemini),
	}
}

/// What the renderer takes from the catalog and policy for this provider and model.
fn render_settings(
	protocol: UpstreamProtocol,
	provider: &str,
	model: &str,
	policies: Option<&Policy>,
	catalog: agent_llm::model_catalog::Catalog<'_>,
) -> RenderSettings {
	let reasoning = || catalog.and_then(|catalog| catalog.reasoning(provider, model));
	match protocol {
		UpstreamProtocol::ChatCompletions => RenderSettings::ChatCompletions(
			reasoning()
				.map(|reasoning| openai_reasoning(&reasoning))
				.unwrap_or_default(),
		),
		UpstreamProtocol::Responses { .. } => RenderSettings::Responses(
			reasoning()
				.map(|reasoning| openai_reasoning(&reasoning))
				.unwrap_or_default(),
		),
		UpstreamProtocol::Gemini => reasoning()
			.and_then(|reasoning| gemini_thinking(&reasoning))
			.map_or(RenderSettings::ModelDefaults, RenderSettings::Gemini),
		UpstreamProtocol::Messages => policies
			.and_then(|policy| policy.prompt_caching.as_ref())
			.map_or(RenderSettings::ModelDefaults, |caching| {
				RenderSettings::Messages(prompt_caching(caching))
			}),
	}
}

fn aliased(policies: Option<&Policy>, model: Strng) -> Strng {
	match policies.and_then(|p| p.resolve_model_alias(&model)) {
		Some(aliased) => aliased.clone(),
		None => model,
	}
}

/// The client's model when the served one differs, recorded in the log too.
pub(super) fn original_model(
	parts: &Parts,
	model: &Strng,
	log: &mut Option<&mut RequestLog>,
) -> Option<Strng> {
	let original_model = parts
		.extensions
		.get::<model_router::OriginalModel>()
		.map(|m| agent_core::strng::new(&m.0))
		.filter(|m| m != model);
	if let Some(log) = log
		&& log.original_model.is_none()
	{
		log.original_model = original_model.as_ref().map(|m| m.to_string());
	}
	original_model
}

pub(super) fn input_format(client: ClientProtocol) -> InputFormat {
	match client {
		ClientProtocol::Messages => InputFormat::Messages,
		ClientProtocol::Responses => InputFormat::Responses,
		ClientProtocol::ChatCompletions => InputFormat::Completions,
		ClientProtocol::Gemini => InputFormat::Gemini,
	}
}

pub(super) fn route_type(upstream: UpstreamProtocol) -> RouteType {
	match upstream {
		UpstreamProtocol::Messages => RouteType::Messages,
		UpstreamProtocol::Responses { .. } => RouteType::Responses,
		UpstreamProtocol::ChatCompletions => RouteType::Completions,
		UpstreamProtocol::Gemini => RouteType::GenerateContent,
	}
}

/// Adds to any betas the client asked for itself.
fn add_anthropic_beta(headers: &mut ::http::HeaderMap, beta: &'static str) {
	const NAME: &str = "anthropic-beta";
	let value = match headers.get(NAME).and_then(|value| value.to_str().ok()) {
		Some(existing) if existing.split(',').any(|item| item.trim() == beta) => return,
		Some(existing) => HeaderValue::try_from(format!("{existing},{beta}"))
			.expect("joining two valid header values with a comma"),
		None => HeaderValue::from_static(beta),
	};
	headers.insert(NAME, value);
}

/// The parameters legacy records for a request.
pub(super) fn request_params(settings: &Settings<'_>) -> LLMRequestParams {
	LLMRequestParams {
		temperature: settings.temperature.and_then(number),
		top_p: settings.top_p.and_then(number),
		max_tokens: settings.max_tokens.map(NonZeroU64::get),
		reasoning_effort: settings.effort.map(effort).or_else(|| {
			settings
				.messages_input
				.output_config
				.and_then(output_effort)
		}),
		..LLMRequestParams::default()
	}
}

fn prompt_caching(caching: &agent_llm::PromptCachingConfig) -> llm_ir::PromptCaching {
	llm_ir::PromptCaching {
		system: caching.cache_system,
		messages: caching.cache_messages,
		tools: caching.cache_tools,
		min_tokens: caching.min_tokens,
		message_offset: caching.cache_message_offset,
	}
}

/// What the catalog says a Gemini model takes to set its thinking. `None` when it records
/// neither levels nor a budget, so the renderer guesses from the model's name.
fn gemini_thinking(reasoning: &agent_llm::model_catalog::Reasoning) -> Option<ThinkingControls> {
	let levels = reasoning
		.effort
		.iter()
		.filter_map(|level| match level.as_str() {
			"minimal" => Some(ThinkingLevel::Minimal),
			"low" => Some(ThinkingLevel::Low),
			"medium" => Some(ThinkingLevel::Medium),
			"high" => Some(ThinkingLevel::High),
			_ => None,
		})
		.fold(ThinkingLevels::default(), ThinkingLevels::with);
	if !levels.is_empty() {
		return Some(ThinkingControls::Levels(levels));
	}
	let (min, max) = reasoning.budget_tokens?;
	let tokens = |tokens: u64| u32::try_from(tokens).unwrap_or(u32::MAX);
	Some(ThinkingControls::Budget {
		min: min.map_or(0, tokens),
		max: max.map_or(u32::MAX, tokens),
		can_stop: reasoning.toggle == Some(true) || min == Some(0),
	})
}

fn openai_reasoning(reasoning: &agent_llm::model_catalog::Reasoning) -> llm_ir::ReasoningControls {
	let controls = llm_ir::ReasoningControls::new(reasoning.supported);
	reasoning
		.effort
		.iter()
		.filter_map(|level| Effort::read(level))
		.fold(controls, llm_ir::ReasoningControls::with_effort)
}

fn effort(effort: Effort) -> agent_llm::ReasoningEffort {
	use agent_llm::ReasoningEffort as Recorded;
	match effort {
		Effort::Off => Recorded::None,
		Effort::Minimal => Recorded::Minimal,
		Effort::Low => Recorded::Low,
		Effort::Medium => Recorded::Medium,
		Effort::High => Recorded::High,
		Effort::XHigh => Recorded::Xhigh,
	}
}

fn number(raw: llm_ir::JsonRef<'_>) -> Option<f64> {
	raw.read().ok()
}

fn output_effort(raw: llm_ir::JsonRef<'_>) -> Option<agent_llm::ReasoningEffort> {
	#[derive(Deserialize)]
	struct OutputConfig<'a> {
		#[serde(borrow)]
		effort: Option<Cow<'a, str>>,
	}
	let config: OutputConfig = raw.read().ok()?;
	agent_llm::ReasoningEffort::parse(&config.effort?)
}

fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_anthropic_beta_joins_the_clients_own() {
		let mut headers = ::http::HeaderMap::new();
		add_anthropic_beta(&mut headers, "b");
		assert_eq!(headers["anthropic-beta"], "b");
		headers.insert("anthropic-beta", HeaderValue::from_static("a"));
		add_anthropic_beta(&mut headers, "b");
		add_anthropic_beta(&mut headers, "b");
		assert_eq!(headers["anthropic-beta"], "a,b");
	}
}
