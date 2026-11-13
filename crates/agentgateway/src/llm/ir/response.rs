//! HTTP response handling and policy accounting for IR attempts.

use std::sync::Arc;

use ::http::{HeaderValue, StatusCode, header};
use agent_llm_ir::HttpReply;
use futures_util::StreamExt;
use headers::{ContentEncoding, HeaderMapExt};

use super::guard::ReplyGuards;
use super::{Attempt, PassthroughReply, reply};
use crate::cel::{self, LLMContext, RequestSnapshot};
use crate::http::{self, Response};
use crate::llm::policy::{PromptGuard, PromptGuardStreamingMode};
use crate::llm::{
	AIError, AIProvider, AmendOnDrop, BufferedResponse, LLMInfo, LLMLogging, LLMRequest, LLMResponse,
	amend_tokens, catalog, map_response_compression_error, normalize_sse_response_headers,
	response_prompt_guard_headers,
};
use crate::proxy::dtrace;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::LLMResponsePolicies;
use crate::telemetry::log::AsyncLog;

/// Converts a reply for an IR attempt, or the provider's HTTP error, into the client's
/// protocol.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_response(
	client: PolicyClient,
	attempt: Attempt,
	req: LLMRequest,
	policies: LLMResponsePolicies,
	req_snapshot: Option<Arc<RequestSnapshot>>,
	logging: LLMLogging,
	model_catalog: Option<&Arc<catalog::ModelCatalog>>,
	resp: Response,
) -> Result<Response, AIError> {
	// Named in full so a new response policy fails to build until someone decides how the IR
	// handles it.
	let LLMResponsePolicies {
		local_rate_limit: _,
		remote_rate_limit: _,
		request_traceparent,
		prompt_guard,
		streaming_prompt_guard_enabled,
	} = &policies;
	let LLMLogging {
		response: log,
		guardrails,
		content,
	} = logging;
	debug_assert!(
		!content.completion && !content.tool_calls,
		"content capture keeps the request on legacy"
	);

	if !resp.status().is_success() {
		let BufferedResponse {
			mut parts,
			bytes,
			mut managed_body,
		} = AIProvider::buffer_response(resp).await?;
		// The provider's status stays, so the client's retry logic sees what it said. This is
		// the provider's failure, not the conversion's, so it isn't recorded as `ReplyFailed`.
		let reply = reply::error(&attempt, parts.status.as_u16(), &bytes);
		parts.headers.insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static("application/json"),
		);
		parts.headers.remove(header::CONTENT_LENGTH);
		parts.headers.remove(header::CONTENT_ENCODING);
		let llm_info = LLMInfo::new(req, LLMResponse::default());
		parts.extensions.insert(LLMContext::from_llm_info(
			llm_info.clone(),
			model_catalog.map(Arc::as_ref),
		));
		managed_body.replace_bytes(reply.body.into());
		let resp = Response::from_parts(parts, managed_body);
		settle(&resp, llm_info, policies, req_snapshot, log);
		return Ok(resp);
	}

	if !req.streaming {
		let max_output_bytes = http::response_buffer_limit(&resp);
		let BufferedResponse {
			mut parts,
			bytes,
			mut managed_body,
		} = AIProvider::buffer_response(resp).await?;
		let guards = (!prompt_guard.is_empty()).then_some(ReplyGuards {
			guards: prompt_guard,
			client: &client,
			log: Some(&guardrails),
		});
		let (reply, llm_resp) = reply::buffered(attempt, &bytes, max_output_bytes, guards);
		let llm_info = LLMInfo::new(req, llm_resp);
		let resp = match reply {
			Ok(reply) => {
				set_head(&mut parts, &reply);
				parts.extensions.insert(LLMContext::from_llm_info(
					llm_info.clone(),
					model_catalog.map(Arc::as_ref),
				));
				managed_body.replace_bytes(reply.body.into());
				Response::from_parts(parts, managed_body)
			},
			// The guard's response replaces the reply, as on legacy.
			Err(rejection) => rejection.map(|replacement| {
				managed_body.replace_content(replacement.into_boxed().into());
				managed_body
			}),
		};
		settle(&resp, llm_info, policies, req_snapshot, log);
		return Ok(resp);
	}

	let guards = if *streaming_prompt_guard_enabled && !prompt_guard.is_empty() {
		PromptGuard {
			streaming: PromptGuardStreamingMode::Enabled,
			request: Vec::new(),
			response: prompt_guard.clone(),
		}
		.begin_streaming_response_guard(
			&client,
			// Guards that call out see the provider's response headers, as on legacy.
			&response_prompt_guard_headers(resp.headers(), request_traceparent.as_ref()),
			req_snapshot.clone(),
			guardrails,
		)
	} else {
		Vec::new()
	};
	let OpenStream {
		mut parts,
		mut body,
		buffer,
		usage,
	} = open_stream(resp, req, log, policies, req_snapshot, model_catalog)?;
	let mut output = Box::pin(reply::stream(
		attempt,
		body.take_content(),
		buffer,
		usage,
		guards,
	));
	// The head waits for the first output, so a failure before any becomes an HTTP error.
	match output.next().await {
		Some(reply::Output::Data(first)) => {
			let rest = output.map(|output| match output {
				reply::Output::Data(data) => Ok::<_, std::convert::Infallible>(data),
				reply::Output::Failed { .. } => unreachable!("a failure after output is an event"),
			});
			let stream = futures_util::stream::once(std::future::ready(Ok(first))).chain(rest);
			body.replace_content(crate::http::RawBody::from_stream(stream).into());
			Ok(normalize_sse_response_headers(Response::from_parts(
				parts, body,
			)))
		},
		Some(reply::Output::Failed { reply, headers }) => {
			set_head(&mut parts, &reply);
			if let Some(headers) = headers {
				parts.headers = headers;
			}
			parts.headers.remove(header::CONTENT_LENGTH);
			body.replace_bytes(reply.body.into());
			Ok(Response::from_parts(parts, body))
		},
		// Every stream ends with output or a failure.
		None => Err(AIError::IncompleteResponse),
	}
}

/// Sends a passthrough reply on as the provider sent it, read for usage on the way. A
/// provider's error is the client's own protocol already, so it goes on too.
pub(crate) async fn process_passthrough_response(
	passthrough: PassthroughReply,
	req: LLMRequest,
	policies: LLMResponsePolicies,
	req_snapshot: Option<Arc<RequestSnapshot>>,
	logging: LLMLogging,
	model_catalog: Option<&Arc<catalog::ModelCatalog>>,
	resp: Response,
) -> Result<Response, AIError> {
	// Named in full so a new response policy fails to build until someone decides how a
	// passthrough handles it.
	let LLMResponsePolicies {
		local_rate_limit: _,
		remote_rate_limit: _,
		request_traceparent: _,
		prompt_guard,
		streaming_prompt_guard_enabled: _,
	} = &policies;
	let LLMLogging {
		response: log,
		guardrails: _,
		content,
	} = logging;
	debug_assert!(
		prompt_guard.is_empty(),
		"a prompt guard keeps the request on legacy"
	);
	debug_assert!(
		!content.completion && !content.tool_calls,
		"content capture keeps the request on legacy"
	);

	if !resp.status().is_success() || !req.streaming {
		let max_held_bytes = http::response_buffer_limit(&resp);
		let BufferedResponse {
			mut parts,
			bytes,
			mut managed_body,
		} = AIProvider::buffer_response(resp).await?;
		let llm_resp = if parts.status.is_success() {
			reply::observed(&passthrough, &bytes, max_held_bytes)
		} else {
			LLMResponse::default()
		};
		let llm_info = LLMInfo::new(req, llm_resp);
		parts.extensions.insert(LLMContext::from_llm_info(
			llm_info.clone(),
			model_catalog.map(Arc::as_ref),
		));
		managed_body.replace_bytes(bytes);
		let resp = Response::from_parts(parts, managed_body);
		settle(&resp, llm_info, policies, req_snapshot, log);
		return Ok(resp);
	}

	let OpenStream {
		parts,
		mut body,
		buffer,
		usage,
	} = open_stream(resp, req, log, policies, req_snapshot, model_catalog)?;
	let stream = reply::observed_stream(passthrough, body.take_content(), buffer, usage);
	body.replace_content(crate::http::RawBody::from_stream(stream).into());
	Ok(normalize_sse_response_headers(Response::from_parts(
		parts, body,
	)))
}

/// Records a whole reply: its usage against token rate limits, and the request log entry.
fn settle(
	resp: &Response,
	llm_info: LLMInfo,
	policies: LLMResponsePolicies,
	req_snapshot: Option<Arc<RequestSnapshot>>,
	log: AsyncLog<LLMInfo>,
) {
	if !policies.local_rate_limit.is_empty() || policies.remote_rate_limit.is_some() {
		let exec = cel::Executor::new_response(req_snapshot.as_deref(), resp);
		amend_tokens(policies, &llm_info, exec);
	}
	log.store(Some(llm_info));
}

/// A streamed reply opened for reading.
struct OpenStream {
	parts: ::http::response::Parts,
	body: http::Body,
	buffer: usize,
	/// Records usage against token rate limits once the stream settles.
	usage: agent_llm::StreamingUsageGuard,
}

/// Opens a streamed reply: logs the request now, since the reply's fields aren't known yet,
/// then traces and decompresses the provider's body.
fn open_stream(
	resp: Response,
	req: LLMRequest,
	log: AsyncLog<LLMInfo>,
	policies: LLMResponsePolicies,
	req_snapshot: Option<Arc<RequestSnapshot>>,
	model_catalog: Option<&Arc<catalog::ModelCatalog>>,
) -> Result<OpenStream, AIError> {
	let buffer = http::response_buffer_limit(&resp);
	let (mut parts, body) = resp.into_parts();
	let llm_info = LLMInfo::new(req, LLMResponse::default());
	// Request-side context for response CEL.
	parts
		.extensions
		.insert(LLMContext::from_llm_info(llm_info.clone(), None));
	log.store(Some(llm_info));
	let body = dtrace::TracingBody::maybe_wrap("llm raw response", body, buffer);
	let encoding = parts.headers.typed_get::<ContentEncoding>();
	let (body, decompressed) = http::compression::decompress_body(body, encoding.as_ref())
		.map_err(|e| map_response_compression_error(e, &parts.headers))?;
	if decompressed.is_some() {
		parts.headers.remove(header::CONTENT_ENCODING);
		parts.headers.remove(header::CONTENT_LENGTH);
		parts.headers.remove(header::TRANSFER_ENCODING);
	}
	let usage = AmendOnDrop::new(log, policies, req_snapshot, model_catalog.cloned()).into_llm();
	Ok(OpenStream {
		parts,
		body,
		buffer,
		usage,
	})
}

/// Why an IR attempt answered with an error. The proxy moves it to the access log, where
/// legacy records its own reply failures.
#[derive(Debug, Clone)]
pub struct ReplyFailed(pub String);

/// Sets the head for a whole client reply, and records why it failed if it did.
fn set_head(parts: &mut ::http::response::Parts, reply: &HttpReply) {
	// The writers use only standard statuses.
	parts.status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
	parts.headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/json"),
	);
	if !parts.status.is_success() {
		let error: serde_json::Value = serde_json::from_slice(&reply.body).unwrap_or_default();
		let message = error["error"]["message"]
			.as_str()
			.unwrap_or("unknown failure");
		parts
			.extensions
			.insert(ReplyFailed(format!("llm-ir reply failed: {message}")));
	}
}
