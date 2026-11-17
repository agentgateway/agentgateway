//! Same-protocol requests: read settings and preserve provider fields.

use std::sync::Arc;

use ::http::header;
use agent_core::prelude::Strng;
use agent_llm_ir::{self as llm_ir, ClientProtocol, UpstreamProtocol};
use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::request::{
	ClientBody, input_format, llm_request, needs_prompt, original_model, route_type,
};
use super::{Converter, PassthroughReply, ReplyState, enrichment};
use crate::http::Request;
use crate::llm::policy::{Policy, PromptEnrichment};
use crate::llm::{
	AIError, AIProvider, LLMRequest, RequestResult, RequestType, SimpleChatCompletionMessage, openai,
	types,
};
use crate::telemetry::log::RequestLog;

impl AIProvider {
	/// What the request log and rate limits record for a passthrough, read from the body
	/// `read` began.
	pub(super) fn passthrough_llm_request(
		&self,
		client: &ClientBody,
		read: ReadBody<'_>,
		protocol: UpstreamProtocol,
		model: &Strng,
		log: &mut Option<&mut RequestLog>,
	) -> Result<LLMRequest, AIError> {
		let settings = match read {
			ReadBody::Parsed(parsed) => parsed.read_passthrough(client.endpoint),
			ReadBody::Fields(fields) => fields.read_passthrough(client.endpoint),
			ReadBody::Unread => llm_ir::RequestFields::read(&client.client_bytes)
				.and_then(|fields| fields.read_passthrough(client.endpoint)),
		}
		.map_err(super::refused)?;
		let original_model = original_model(&client.parts, model, log);
		Ok(llm_request(
			client.endpoint.protocol(),
			protocol,
			&self.provider(),
			model,
			original_model,
			&settings,
		))
	}

	/// Serves a request whose provider takes the client's own protocol, without converting it.
	/// It goes as the client sent it but for what legacy writes too: the served model, and
	/// what makes usage countable, plus configured prompt enrichment.
	pub(super) fn process_passthrough(
		&self,
		client: ClientBody,
		mut llm_request: LLMRequest,
		write_model: bool,
		protocol: UpstreamProtocol,
		policies: Option<&Policy>,
		log: &mut Option<&mut RequestLog>,
	) -> Result<RequestResult, AIError> {
		if let Some(log) = log.as_deref_mut() {
			log.llm_converter = Some(Converter::LlmIrPassthrough);
		}
		let ClientBody {
			endpoint,
			mut parts,
			mut body,
			client_bytes,
			..
		} = client;
		let model = llm_request.request_model.clone();
		let edited = self.passthrough_edits(
			endpoint.protocol(),
			&client_bytes,
			write_model.then_some(model.as_str()),
			llm_request.streaming,
			&model,
			self.inline_moderation(protocol),
			policies.and_then(|policy| policy.prompts.as_ref()),
		)?;
		if needs_prompt(log) {
			let sent = edited.as_deref().unwrap_or(&client_bytes);
			llm_request.prompt = Some(Arc::new(prompt(endpoint.protocol(), sent)?));
		}
		parts.headers.remove(header::CONTENT_LENGTH);
		parts.headers.remove(header::TRANSFER_ENCODING);
		let sent = match policies.filter(|policy| policy.has_final_transformations()) {
			Some(policy) => {
				let body = edited.unwrap_or_else(|| client_bytes.to_vec());
				policy.apply_final_transformations(body, log)?.into()
			},
			None => edited.map(Bytes::from).unwrap_or(client_bytes),
		};
		body.replace_bytes(sent);
		Ok(RequestResult::Success {
			request: Request::from_parts(parts, body),
			llm_request,
			ir_reply: Some(ReplyState::Passthrough(PassthroughReply {
				protocol,
				model,
			})),
			upstream_route_type: route_type(protocol),
		})
	}

	/// What legacy writes into a passthrough body: the served model, usage in a Chat
	/// Completions stream, the output limit OpenAI's GPT models take, inline moderation, and
	/// prompt enrichment. `None` when there is nothing to write. The model alone is replaced in
	/// place; other edits read the top level and preserve unrelated fields as raw JSON.
	#[allow(clippy::too_many_arguments)]
	fn passthrough_edits(
		&self,
		client: ClientProtocol,
		body: &[u8],
		write_model: Option<&str>,
		streaming: bool,
		model: &str,
		moderation: Option<&openai::ModerationParam>,
		prompts: Option<&PromptEnrichment>,
	) -> Result<Option<Vec<u8>>, AIError> {
		let prompts =
			prompts.filter(|prompts| !prompts.prepend.is_empty() || !prompts.append.is_empty());
		let chat = client == ClientProtocol::ChatCompletions;
		let usage = chat && streaming;
		let output_limit = chat
			&& matches!(
				self,
				AIProvider::OpenAI(_) | AIProvider::Copilot(_) | AIProvider::Azure(_)
			) && (model.starts_with("gpt-") || model.contains("openai.gpt-"));
		if !usage && !output_limit && moderation.is_none() && prompts.is_none() {
			return Ok(write_model.map(|model| {
				llm_ir::replace_model(body, model).expect("a body whose model was read names one")
			}));
		}
		let mut fields: indexmap::IndexMap<String, Box<RawValue>> = serde_json::from_slice(body)
			.map_err(|err| AIError::RequestParsing(input_format(client), err))?;
		if let Some(model) = write_model {
			fields.insert("model".into(), raw_json(&model));
		}
		if usage {
			let mut options: indexmap::IndexMap<String, Box<RawValue>> = fields
				.get("stream_options")
				.and_then(|raw| serde_json::from_str(raw.get()).ok())
				.unwrap_or_default();
			options.insert("include_usage".into(), raw_json(&true));
			fields.insert("stream_options".into(), raw_json(&options));
		}
		if output_limit
			&& let Some(limit) = fields.shift_remove("max_tokens")
			&& limit.get() != "null"
		{
			fields
				.entry("max_completion_tokens".into())
				.or_insert(limit);
		}
		if let Some(moderation) = moderation {
			fields.insert("moderation".into(), raw_json(moderation));
		}
		if let Some(prompts) = prompts {
			enrichment::apply_passthrough(prompts, client, &mut fields)
				.map_err(|err| AIError::RequestParsing(input_format(client), err))?;
		}
		serde_json::to_vec(&fields)
			.map(Some)
			.map_err(AIError::RequestMarshal)
	}
}

/// The client's body as far as reading its model took it.
pub(super) enum ReadBody<'a> {
	/// The model router's parse, which saves scanning the bytes again.
	Parsed(llm_ir::ParsedRequest<'a>),
	Fields(llm_ir::RequestFields<'a>),
	/// At most the model, which came before the conversation, or the URL named. A converted
	/// request is read as it is prepared, and that read refuses a repeated model.
	Unread,
}

/// What CEL reads as `llm.prompt`, from the body as sent before final transformations, read
/// as legacy reads the client's format.
fn prompt(
	client: ClientProtocol,
	body: &[u8],
) -> Result<Vec<SimpleChatCompletionMessage>, AIError> {
	fn read<T: RequestType + DeserializeOwned>(
		body: &[u8],
	) -> Result<Vec<SimpleChatCompletionMessage>, AIError> {
		serde_json::from_slice::<T>(body)
			.map(|request| request.get_messages())
			.map_err(|err| AIError::RequestParsing(T::input_format(), err))
	}
	match client {
		ClientProtocol::Messages => read::<types::messages::Request>(body),
		ClientProtocol::Responses => read::<types::responses::Request>(body),
		ClientProtocol::ChatCompletions => read::<types::completions::Request>(body),
		ClientProtocol::Gemini => read::<types::gemini::Request>(body),
	}
}

pub(super) fn raw_json(value: &impl serde::Serialize) -> Box<RawValue> {
	serde_json::value::to_raw_value(value).expect("a JSON value serializes")
}
