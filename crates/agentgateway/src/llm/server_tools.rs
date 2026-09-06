//! Fulfilment of client-declared server tools through MCP.
//!
//! The request and response manipulation lives in `agent_llm::server_tools`. This module wires it
//! into the proxy: it resolves the MCP backend from the store, rewrites the request before it is
//! translated, and after each model response decides whether to execute a tool and continue the
//! same turn or hand the finished message to the client.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use ::http::request::Parts;
use agent_llm::server_tools as st;
use agent_llm::server_tools::{InterceptedTool, ToolDefinition, ToolUse, UsageTotals};
use http_body_util::BodyExt;
use serde_json::Value;
use tokio::sync::mpsc;

use super::policy::{ServerToolFailureMode, ServerToolsConfig};
use super::*;
use crate::mcp::{MCPInfo, ToolRuntime};
use crate::proxy::ProxyError;

#[cfg(test)]
#[path = "server_tools_tests.rs"]
mod tests;

/// Per-request interception state, built when the request is parsed.
pub struct Interception {
	config: Arc<ServerToolsConfig>,
	policy: Policy,
	tools: Vec<InterceptedTool>,
	/// The client's request after the rewrite. Follow-up turns are appended to a copy of it.
	request: types::messages::Request,
	/// The client request headers, used when rendering follow-up requests.
	headers: HeaderMap,
	/// Tool runtimes keyed by mapping index.
	runtimes: HashMap<usize, Arc<ToolRuntime>>,
	max_iterations: u32,
}

impl std::fmt::Debug for Interception {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Interception")
			.field("tools", &self.tools)
			.field("max_iterations", &self.max_iterations)
			.finish()
	}
}

impl Interception {
	fn tool(&self, name: &str) -> Option<&InterceptedTool> {
		self.tools.iter().find(|t| t.name == name)
	}

	fn names(&self) -> HashSet<&str> {
		self.tools.iter().map(|t| t.name.as_str()).collect()
	}

	fn runtime(&self, tool: &InterceptedTool) -> Option<&Arc<ToolRuntime>> {
		self.runtimes.get(&tool.mapping)
	}
}

/// Rewrite the client's server tools when the policy maps them. Returns `None`, leaving the request
/// untouched, when nothing applies or the MCP tool definitions cannot be resolved.
pub async fn intercept(
	config: &Arc<ServerToolsConfig>,
	policy: &Policy,
	req: &mut types::messages::Request,
	inputs: &Arc<ProxyInputs>,
	parts: &Parts,
) -> Option<Arc<Interception>> {
	let found = st::find_server_tools(req, &config.matchers());
	if found.is_empty() {
		return None;
	}
	let mut runtimes: HashMap<usize, Arc<ToolRuntime>> = HashMap::new();
	let mut tools = Vec::with_capacity(found.len());
	let mut definitions = Vec::with_capacity(found.len());
	for tool in found {
		let mapping = &config.tools[tool.mapping];
		let runtime = match runtimes.get(&tool.mapping) {
			Some(rt) => rt.clone(),
			None => match ToolRuntime::new(
				inputs,
				&mapping.mcp.backend,
				mapping.mcp.target.as_deref(),
				parts,
			) {
				Ok(rt) => {
					let rt = Arc::new(rt);
					runtimes.insert(tool.mapping, rt.clone());
					rt
				},
				Err(e) => {
					warn!(
						tool = %tool.name,
						backend = %mapping.mcp.backend,
						"server tool not intercepted: {e}"
					);
					continue;
				},
			},
		};
		let definition = match &mapping.input_schema {
			Some(schema) => ToolDefinition {
				description: mapping.description.clone(),
				input_schema: schema.clone(),
			},
			None => match runtime.tool_definition(&mapping.mcp.tool).await {
				Ok(Some(mut def)) => {
					if mapping.description.is_some() {
						def.description = mapping.description.clone();
					}
					def
				},
				Ok(None) => {
					warn!(
						tool = %tool.name,
						backend = %mapping.mcp.backend,
						mcp_tool = %mapping.mcp.tool,
						"server tool not intercepted: the MCP backend does not serve that tool"
					);
					continue;
				},
				Err(e) => {
					warn!(
						tool = %tool.name,
						backend = %mapping.mcp.backend,
						"server tool not intercepted: {e}"
					);
					continue;
				},
			},
		};
		tools.push(tool);
		definitions.push(definition);
	}
	if tools.is_empty() {
		return None;
	}
	st::rewrite_server_tools(req, &tools, &definitions);
	let max_iterations = tools
		.iter()
		.filter_map(|t| t.max_uses)
		.filter(|m| *m > 0)
		.min()
		.map(|m| m.min(u64::from(config.max_iterations)) as u32)
		.unwrap_or(config.max_iterations)
		.max(1);
	debug!(tools = ?tools, max_iterations, "intercepting server tools");
	Some(Arc::new(Interception {
		config: config.clone(),
		policy: policy.clone(),
		tools,
		request: req.clone(),
		headers: parts.headers.clone(),
		runtimes,
		max_iterations,
	}))
}

/// What the proxy needs to send a follow-up request to the same upstream.
pub struct ResendContext {
	pub parts: Parts,
	pub backend_auth: Option<BackendAuth>,
	pub upstream: client::Client,
	pub target: Target,
	pub connection: client::ConnectionConfig,
	pub mcp_log: Option<AsyncLog<MCPInfo>>,
}

impl ResendContext {
	async fn send(&self, body: Vec<u8>) -> Result<Response, ProxyError> {
		let mut parts = self.parts.clone();
		parts.headers.remove(header::CONTENT_LENGTH);
		let mut req = Request::from_parts(parts, Body::from(body));
		crate::http::auth::apply_late_backend_auth(self.backend_auth.as_ref(), &mut req).await?;
		self
			.upstream
			.call(client::Call {
				req,
				target: self.target.clone(),
				connection: self.connection.clone(),
			})
			.await
	}
}

fn server_tool_error(msg: impl std::fmt::Display) -> AIError {
	AIError::ServerTool(strng::format!("{msg}"))
}

/// One model response, in the client's format.
struct Turn {
	message: Value,
	/// The translated streaming events, when the client asked for a stream.
	events: Vec<st::SseEvent>,
	parts: ::http::response::Parts,
	raw: Bytes,
}

enum TurnOutcome {
	Message(Turn),
	/// The upstream answered with a non-success status.
	Failed(BufferedResponse),
}

struct LoopState {
	conversation: types::messages::Request,
	totals: UsageTotals,
	iteration: u32,
	last_fingerprint: Option<String>,
}

enum Next {
	/// Return the message to the client; `strip` removes the gateway's tool calls first.
	Finish {
		strip: bool,
	},
	Execute(Vec<ToolUse>),
}

impl AIProvider {
	async fn read_turn(
		&self,
		req: &LLMRequest,
		catalog: Option<&Arc<catalog::ModelCatalog>>,
		resp: Response,
	) -> Result<TurnOutcome, AIError> {
		let buffered = Self::buffer_response(resp).await?;
		if !buffered.parts.status.is_success() {
			return Ok(TurnOutcome::Failed(buffered));
		}
		let BufferedResponse { parts, bytes } = buffered;
		let (message, events) = if req.streaming {
			let translation = self.chat_translation(
				req.input_format,
				Some(&req.request_model),
				catalog.map(|c| c.as_handle()),
			)?;
			let translated = translation.stream(
				Response::from_parts(parts.clone(), Body::from(bytes.clone())),
				ChatStreamContext {
					buffer_limit: usize::MAX,
					logger: agent_llm::StreamingUsageGuard::default(),
					model: req.request_model.to_string(),
					log_content: LogContentFields::default(),
					tool_name_map: bedrock_tool_name_map(req).cloned(),
				},
			);
			let body = translated
				.into_body()
				.collect()
				.await
				.map_err(AIError::ResponseDecoding)?
				.to_bytes();
			let events = st::parse_sse(&body);
			let mut acc = st::MessageAccumulator::default();
			acc.feed_all(&events);
			let message = acc
				.finish()
				.ok_or_else(|| server_tool_error("upstream stream carried no message"))?;
			(message, events)
		} else {
			let translated =
				self.translate_chat_or_detect_response(req, &bytes, catalog.map(|c| c.as_handle()))?;
			let body = translated.serialize().map_err(AIError::ResponseParsing)?;
			let message = serde_json::from_slice::<Value>(&body).map_err(AIError::ResponseParsing)?;
			(message, Vec::new())
		};
		Ok(TurnOutcome::Message(Turn {
			message,
			events,
			parts,
			raw: bytes,
		}))
	}

	fn render_follow_up(
		&self,
		interception: &Interception,
		conversation: &types::messages::Request,
		req: &LLMRequest,
		catalog: Option<&Arc<catalog::ModelCatalog>>,
	) -> Result<Vec<u8>, AIError> {
		let translation = self.chat_translation(
			req.input_format,
			Some(&req.request_model),
			catalog.map(|c| c.as_handle()),
		)?;
		let rendered = translation.render_request(
			types::ChatRequest::Messages(conversation.clone()),
			&ChatRequestContext {
				provider: self,
				headers: &interception.headers,
				prompt_caching: interception.policy.prompt_caching.as_ref(),
				catalog: catalog.map(|c| c.as_handle()),
			},
		)?;
		interception
			.policy
			.apply_final_transformations(rendered.body, &mut None)
	}
}

fn decide(interception: &Interception, state: &LoopState, message: &Value) -> Next {
	let calls = st::tool_uses(message);
	let ours: Vec<ToolUse> = calls
		.iter()
		.filter(|c| interception.tool(&c.name).is_some())
		.cloned()
		.collect();
	if ours.is_empty() {
		return Next::Finish { strip: false };
	}
	if ours.len() != calls.len() {
		debug!("model mixed server tools with client tools; returning the client tools");
		return Next::Finish { strip: true };
	}
	if state.iteration >= interception.max_iterations {
		debug!(
			iterations = state.iteration,
			"server tool iteration cap reached; ending the turn"
		);
		return Next::Finish { strip: true };
	}
	if state.last_fingerprint.as_deref() == Some(st::fingerprint(&ours).as_str()) {
		debug!("model repeated the same server tool call; ending the turn");
		return Next::Finish { strip: true };
	}
	Next::Execute(ours)
}

async fn execute(
	interception: &Interception,
	calls: &[ToolUse],
	mcp_log: Option<&AsyncLog<MCPInfo>>,
) -> Result<Vec<Value>, AIError> {
	let mut results = Vec::with_capacity(calls.len());
	for call in calls {
		let tool = interception
			.tool(&call.name)
			.ok_or_else(|| server_tool_error(format!("unmapped tool {}", call.name)))?;
		let runtime = interception
			.runtime(tool)
			.ok_or_else(|| server_tool_error(format!("no runtime for tool {}", call.name)))?;
		let mapping = &interception.config.tools[tool.mapping];
		let outcome = runtime.call(&mapping.mcp.tool, &call.input, mcp_log).await;
		let block = match outcome {
			Ok(outcome) => st::tool_result_block(
				&call.id,
				st::mcp_content_to_tool_result(&outcome.content, interception.config.max_result_bytes),
				outcome.is_error,
			),
			Err(e) => match interception.config.failure_mode {
				ServerToolFailureMode::FailClosed => {
					return Err(server_tool_error(format!("tool {} failed: {e}", call.name)));
				},
				ServerToolFailureMode::FailOpen => {
					warn!(tool = %call.name, "server tool failed; reporting the error to the model: {e}");
					st::tool_result_block(
						&call.id,
						vec![serde_json::json!({"type": "text", "text": format!("tool call failed: {e}")})],
						true,
					)
				},
			},
		};
		results.push(block);
	}
	Ok(results)
}

async fn close_runtimes(interception: &Interception) {
	for runtime in interception.runtimes.values() {
		runtime.close().await;
	}
}

fn llm_response_for(message: &Value, log_content: LogContentFields) -> LLMResponse {
	serde_json::from_value::<types::messages::Response>(message.clone())
		.map(|r| r.to_llm_response(log_content))
		.unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
impl AIProvider {
	/// Run the tool loop for a request whose server tools were intercepted. The first upstream
	/// response has a success status.
	pub async fn run_server_tools(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		logging: LLMLogging,
		model_catalog: Option<&Arc<catalog::ModelCatalog>>,
		first: Response,
		interception: Arc<Interception>,
		resend: Box<ResendContext>,
	) -> Result<Response, AIError> {
		if req.streaming {
			Box::pin(self.run_server_tools_streaming(
				client,
				req,
				rate_limit,
				req_snapshot,
				logging,
				model_catalog.cloned(),
				first,
				interception,
				resend,
			))
			.await
		} else {
			Box::pin(self.run_server_tools_buffered(
				client,
				req,
				rate_limit,
				req_snapshot,
				logging,
				model_catalog,
				first,
				interception,
				resend,
			))
			.await
		}
	}

	async fn run_server_tools_buffered(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		logging: LLMLogging,
		model_catalog: Option<&Arc<catalog::ModelCatalog>>,
		first: Response,
		interception: Arc<Interception>,
		resend: Box<ResendContext>,
	) -> Result<Response, AIError> {
		let mut state = LoopState {
			conversation: interception.request.clone(),
			totals: UsageTotals::default(),
			iteration: 0,
			last_fingerprint: None,
		};
		let mut resp = first;
		let result = loop {
			let turn = match self.read_turn(&req, model_catalog, resp).await? {
				TurnOutcome::Message(turn) => turn,
				TurnOutcome::Failed(buffered) => {
					break self
						.process_chat_or_detect_buffered_response(
							client,
							req,
							rate_limit,
							req_snapshot,
							logging,
							model_catalog.map(Arc::as_ref),
							buffered,
						)
						.await;
				},
			};
			match decide(&interception, &state, &turn.message) {
				Next::Finish { strip } => {
					if !strip && state.iteration == 0 {
						break self
							.process_chat_or_detect_buffered_response(
								client,
								req,
								rate_limit,
								req_snapshot,
								logging,
								model_catalog.map(Arc::as_ref),
								BufferedResponse {
									parts: turn.parts,
									bytes: turn.raw,
								},
							)
							.await;
					}
					let mut message = turn.message;
					if strip {
						st::strip_tool_uses(&mut message, &interception.names());
					}
					state.totals.add(UsageTotals::of(&message));
					st::set_usage(&mut message, state.totals);
					let translated: Box<dyn ResponseType> = Box::new(
						serde_json::from_value::<types::messages::Response>(message)
							.map_err(AIError::ResponseParsing)?,
					);
					break self
						.finish_translated_response(
							client,
							req,
							rate_limit,
							req_snapshot,
							logging,
							model_catalog.map(Arc::as_ref),
							turn.parts,
							translated,
						)
						.await;
				},
				Next::Execute(calls) => {
					let results = execute(&interception, &calls, resend.mcp_log.as_ref()).await?;
					state.last_fingerprint = Some(st::fingerprint(&calls));
					state.totals.add(UsageTotals::of(&turn.message));
					state.iteration += 1;
					let assistant = turn
						.message
						.get("content")
						.and_then(Value::as_array)
						.cloned()
						.unwrap_or_default();
					st::append_tool_turn(&mut state.conversation, assistant, results);
					let body =
						self.render_follow_up(&interception, &state.conversation, &req, model_catalog)?;
					resp = resend
						.send(body)
						.await
						.map_err(|e| server_tool_error(format!("follow-up request failed: {e}")))?;
				},
			}
		};
		close_runtimes(&interception).await;
		result
	}

	async fn run_server_tools_streaming(
		&self,
		client: PolicyClient,
		req: LLMRequest,
		rate_limit: LLMResponsePolicies,
		req_snapshot: Option<Arc<RequestSnapshot>>,
		logging: LLMLogging,
		model_catalog: Option<Arc<catalog::ModelCatalog>>,
		first: Response,
		interception: Arc<Interception>,
		resend: Box<ResendContext>,
	) -> Result<Response, AIError> {
		let LLMLogging {
			response: log,
			guardrails: guardrail_log,
			content: log_content,
		} = logging;
		log.store(Some(llm::LLMInfo {
			request: req.clone(),
			response: LLMResponse::default(),
		}));
		let buffer = http::response_buffer_limit(&first);

		// The client gets the first response's headers now; everything else arrives as events.
		let (parts, body) = first.into_parts();
		let mut client_parts = parts.clone();
		client_parts.headers.remove(header::CONTENT_LENGTH);
		client_parts.headers.remove(header::CONTENT_ENCODING);
		client_parts.headers.remove(header::TRANSFER_ENCODING);
		client_parts.extensions.remove::<DeferredResponseEncoding>();
		let client_parts =
			normalize_sse_response_headers(Response::from_parts(client_parts, Body::empty()))
				.into_parts()
				.0;

		let prompt_guard_headers = response_prompt_guard_headers(
			&client_parts.headers,
			rate_limit.request_traceparent.as_ref(),
		);
		let evaluators =
			if rate_limit.streaming_prompt_guard_enabled && !rate_limit.prompt_guard.is_empty() {
				let temp_guard = policy::PromptGuard {
					streaming: policy::PromptGuardStreamingMode::Enabled,
					request: vec![],
					response: rate_limit.prompt_guard.clone(),
				};
				temp_guard.begin_streaming_response_guard(
					&client,
					&prompt_guard_headers,
					req_snapshot.clone(),
					guardrail_log,
				)
			} else {
				vec![]
			};
		let amend = AmendOnDrop::new(log, rate_limit, req_snapshot, model_catalog.clone());

		let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);
		let provider = self.clone();
		let keepalive = interception.config.keepalive_interval;
		let task = tokio::spawn(async move {
			let outcome = provider
				.server_tools_stream_loop(
					&req,
					model_catalog.as_ref(),
					Response::from_parts(parts, body),
					&interception,
					&resend,
					&tx,
					keepalive,
				)
				.await;
			close_runtimes(&interception).await;
			match outcome {
				Ok(Some((events, message))) => {
					amend.non_atomic_mutate(|r| r.response = llm_response_for(&message, log_content));
					let _ = tx.send(Ok(st::encode_sse(&events))).await;
				},
				Ok(None) => {},
				Err(e) => {
					warn!("server tool turn failed: {e}");
					let _ = tx.send(Ok(st::error_event(&e.to_string()))).await;
				},
			}
			// Report usage once the final events are queued.
			drop(amend);
		});
		let abort = AbortOnDrop(task.abort_handle());
		let stream = futures_util::stream::unfold((rx, abort), |(mut rx, abort)| async move {
			rx.recv().await.map(|item| (item, (rx, abort)))
		});
		let body = Body::from_stream(stream);
		let body = if evaluators.is_empty() {
			body
		} else {
			GuardedSseBody::new(body, evaluators, buffer, None)
		};
		Ok(Response::from_parts(client_parts, body))
	}

	/// Drive the loop while keeping the client alive. Returns the final events and message, or
	/// `None` when the client went away.
	async fn server_tools_stream_loop(
		&self,
		req: &LLMRequest,
		catalog: Option<&Arc<catalog::ModelCatalog>>,
		first: Response,
		interception: &Interception,
		resend: &ResendContext,
		tx: &mpsc::Sender<Result<Bytes, std::io::Error>>,
		keepalive: Duration,
	) -> Result<Option<(Vec<st::SseEvent>, Value)>, AIError> {
		let mut state = LoopState {
			conversation: interception.request.clone(),
			totals: UsageTotals::default(),
			iteration: 0,
			last_fingerprint: None,
		};
		let mut resp = first;
		loop {
			let Some(turn) = with_keepalive(tx, keepalive, self.read_turn(req, catalog, resp)).await
			else {
				return Ok(None);
			};
			let turn = match turn? {
				TurnOutcome::Message(turn) => turn,
				TurnOutcome::Failed(buffered) => {
					return Err(server_tool_error(format!(
						"follow-up request failed with status {}",
						buffered.parts.status
					)));
				},
			};
			match decide(interception, &state, &turn.message) {
				Next::Finish { strip } => {
					let mut message = turn.message;
					if !strip && state.iteration == 0 {
						return Ok(Some((turn.events, message)));
					}
					state.totals.add(UsageTotals::of(&message));
					st::set_usage(&mut message, state.totals);
					let events = if strip {
						st::strip_tool_uses(&mut message, &interception.names());
						st::synthesize_sse(&message)
					} else {
						let mut events = turn.events;
						st::patch_usage(&mut events, state.totals);
						events
					};
					return Ok(Some((events, message)));
				},
				Next::Execute(calls) => {
					let Some(results) = with_keepalive(
						tx,
						keepalive,
						execute(interception, &calls, resend.mcp_log.as_ref()),
					)
					.await
					else {
						return Ok(None);
					};
					let results = results?;
					state.last_fingerprint = Some(st::fingerprint(&calls));
					state.totals.add(UsageTotals::of(&turn.message));
					state.iteration += 1;
					let assistant = turn
						.message
						.get("content")
						.and_then(Value::as_array)
						.cloned()
						.unwrap_or_default();
					st::append_tool_turn(&mut state.conversation, assistant, results);
					let body = self.render_follow_up(interception, &state.conversation, req, catalog)?;
					let Some(sent) = with_keepalive(tx, keepalive, resend.send(body)).await else {
						return Ok(None);
					};
					resp = sent.map_err(|e| server_tool_error(format!("follow-up request failed: {e}")))?;
				},
			}
		}
	}
}

/// Poll `fut` while sending a `ping` event every `interval`. Returns `None` if the client is gone.
async fn with_keepalive<T>(
	tx: &mpsc::Sender<Result<Bytes, std::io::Error>>,
	interval: Duration,
	fut: impl Future<Output = T>,
) -> Option<T> {
	tokio::pin!(fut);
	let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(100)));
	ticker.tick().await;
	loop {
		tokio::select! {
			out = &mut fut => return Some(out),
			_ = ticker.tick() => {
				if tx.send(Ok(st::ping_event())).await.is_err() {
					return None;
				}
			},
		}
	}
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
	fn drop(&mut self) {
		self.0.abort();
	}
}
