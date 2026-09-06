//! Fulfilment of client-declared server tools through MCP.
//!
//! The request and response manipulation lives in `agent_llm::server_tools` (Anthropic Messages),
//! `agent_llm::responses_tools` (OpenAI Responses) and `agent_llm::completions_tools` (OpenAI Chat
//! Completions `web_search_options`). This module wires them into the proxy: it
//! resolves the MCP backends from the store, rewrites the request before it is translated, and
//! after each model response decides whether to execute a tool and continue the same turn or hand
//! the finished message to the client.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use ::http::request::Parts;
use agent_llm::server_tools::{InterceptedTool, SseEvent, ToolDefinition, ToolUse};
use agent_llm::{completions_tools as ct, responses_tools as rt, server_tools as st};
use http_body_util::BodyExt;
use serde_json::Value;
use tokio::sync::mpsc;

use super::policy::{
	ServerToolFailureMode, ServerToolMapping, ServerToolMcpServer, ServerToolsConfig,
};
use super::*;
use crate::mcp::{MCPInfo, ToolRuntime};
use crate::proxy::ProxyError;

#[cfg(test)]
#[path = "server_tools_tests.rs"]
mod tests;

/// The client's wire format, which decides how a turn is read, continued, and returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
	Messages,
	Responses,
	Completions,
}

impl Wire {
	fn tool_uses(self, message: &Value) -> Vec<ToolUse> {
		match self {
			Wire::Messages => st::tool_uses(message),
			Wire::Responses => rt::function_calls(message),
			Wire::Completions => ct::tool_calls(message),
		}
	}

	fn strip(self, message: &mut Value, names: &HashSet<&str>) -> usize {
		match self {
			Wire::Messages => st::strip_tool_uses(message, names),
			Wire::Responses => rt::strip_function_calls(message, names),
			Wire::Completions => ct::strip_tool_calls(message, names),
		}
	}

	/// Rebuild the complete message from the translated stream.
	fn accumulate(self, events: &[SseEvent]) -> Option<Value> {
		match self {
			Wire::Messages => {
				let mut acc = st::MessageAccumulator::default();
				acc.feed_all(events);
				acc.finish()
			},
			Wire::Responses => rt::final_response(events),
			Wire::Completions => {
				let mut acc = ct::ChunkAccumulator::default();
				acc.feed_all(events);
				acc.finish()
			},
		}
	}

	fn synthesize(self, message: &Value) -> Vec<SseEvent> {
		match self {
			Wire::Messages => st::synthesize_sse(message),
			Wire::Responses => rt::synthesize_sse(message),
			Wire::Completions => ct::synthesize_sse(message),
		}
	}

	fn keepalive(self) -> Bytes {
		match self {
			Wire::Messages => st::ping_event(),
			Wire::Responses => rt::keepalive(),
			Wire::Completions => ct::keepalive(),
		}
	}

	fn error_event(self, message: &str) -> Bytes {
		match self {
			Wire::Messages => st::error_event(message),
			Wire::Responses => rt::error_event(message),
			Wire::Completions => ct::error_event(message),
		}
	}

	/// The tool result item the model reads on the next turn.
	fn tool_result(
		self,
		call: &ToolUse,
		content: &[Value],
		is_error: bool,
		max_bytes: usize,
	) -> Value {
		match self {
			Wire::Messages => st::tool_result_block(
				&call.id,
				st::mcp_content_to_tool_result(content, max_bytes),
				is_error,
			),
			Wire::Responses => {
				let mut output = rt::mcp_content_to_output(content, max_bytes);
				if is_error {
					output = format!("error: {output}");
				}
				rt::function_call_output(&call.id, output)
			},
			Wire::Completions => {
				let mut output = rt::mcp_content_to_output(content, max_bytes);
				if is_error {
					output = format!("error: {output}");
				}
				ct::tool_message(&call.id, output)
			},
		}
	}

	/// The tool result that reports a failed call to the model.
	fn error_result(self, call: &ToolUse, message: &str) -> Value {
		match self {
			Wire::Messages => st::tool_result_block(
				&call.id,
				vec![serde_json::json!({"type": "text", "text": format!("tool call failed: {message}")})],
				true,
			),
			Wire::Responses => rt::function_call_output(&call.id, format!("tool call failed: {message}")),
			Wire::Completions => ct::tool_message(&call.id, format!("tool call failed: {message}")),
		}
	}

	fn parse_final(self, message: Value) -> Result<Box<dyn ResponseType>, AIError> {
		Ok(match self {
			Wire::Messages => Box::new(
				serde_json::from_value::<types::messages::Response>(message)
					.map_err(AIError::ResponseParsing)?,
			),
			Wire::Responses => Box::new(
				serde_json::from_value::<types::responses::Response>(message)
					.map_err(AIError::ResponseParsing)?,
			),
			Wire::Completions => Box::new(
				serde_json::from_value::<types::completions::Response>(message)
					.map_err(AIError::ResponseParsing)?,
			),
		})
	}

	fn llm_response_for(self, message: &Value, log_content: LogContentFields) -> LLMResponse {
		match self {
			Wire::Messages => serde_json::from_value::<types::messages::Response>(message.clone())
				.map(|r| r.to_llm_response(log_content))
				.unwrap_or_default(),
			Wire::Responses => serde_json::from_value::<types::responses::Response>(message.clone())
				.map(|r| r.to_llm_response(log_content))
				.unwrap_or_default(),
			Wire::Completions => serde_json::from_value::<types::completions::Response>(message.clone())
				.map(|r| r.to_llm_response(log_content))
				.unwrap_or_default(),
		}
	}
}

/// The conversation being continued, in the client's format.
#[derive(Clone)]
enum Conversation {
	Messages(types::messages::Request),
	Responses(types::responses::Request),
	Completions(types::completions::Request),
}

impl Conversation {
	fn wire(&self) -> Wire {
		match self {
			Conversation::Messages(_) => Wire::Messages,
			Conversation::Responses(_) => Wire::Responses,
			Conversation::Completions(_) => Wire::Completions,
		}
	}

	fn chat_request(&self) -> types::ChatRequest {
		match self {
			Conversation::Messages(req) => types::ChatRequest::Messages(req.clone()),
			Conversation::Responses(req) => types::ChatRequest::Responses(req.clone()),
			Conversation::Completions(req) => types::ChatRequest::Completions(req.clone()),
		}
	}

	/// Append the model's turn and the tool results, ready to be sent again.
	fn append(&mut self, message: &Value, results: Vec<Value>) {
		match self {
			Conversation::Messages(req) => {
				let assistant = message
					.get("content")
					.and_then(Value::as_array)
					.cloned()
					.unwrap_or_default();
				st::append_tool_turn(req, assistant, results);
			},
			Conversation::Responses(req) => {
				let output = message
					.get("output")
					.and_then(Value::as_array)
					.cloned()
					.unwrap_or_default();
				rt::append_tool_turn(req, output, results);
			},
			Conversation::Completions(req) => {
				let assistant = message
					.get("choices")
					.and_then(Value::as_array)
					.and_then(|c| c.first())
					.and_then(|c| c.get("message"))
					.cloned()
					.unwrap_or(Value::Null);
				ct::append_tool_turn(req, assistant, results);
			},
		}
	}
}

/// Token usage summed across the model calls of one client turn.
#[derive(Debug, Clone, Copy)]
enum Totals {
	Messages(st::UsageTotals),
	Responses(rt::UsageTotals),
	Completions(ct::UsageTotals),
}

impl Totals {
	fn new(wire: Wire) -> Self {
		match wire {
			Wire::Messages => Totals::Messages(Default::default()),
			Wire::Responses => Totals::Responses(Default::default()),
			Wire::Completions => Totals::Completions(Default::default()),
		}
	}

	fn add(&mut self, message: &Value) {
		match self {
			Totals::Messages(t) => t.add(st::UsageTotals::of(message)),
			Totals::Responses(t) => t.add(rt::UsageTotals::of(message)),
			Totals::Completions(t) => t.add(ct::UsageTotals::of(message)),
		}
	}

	fn set(&self, message: &mut Value) {
		match self {
			Totals::Messages(t) => st::set_usage(message, *t),
			Totals::Responses(t) => rt::set_usage(message, *t),
			Totals::Completions(t) => ct::set_usage(message, *t),
		}
	}

	fn patch(&self, events: &mut Vec<SseEvent>) {
		match self {
			Totals::Messages(t) => st::patch_usage(events, *t),
			Totals::Responses(t) => rt::patch_usage(events, *t),
			Totals::Completions(t) => ct::patch_usage(events, *t),
		}
	}
}

/// A tool the model can call, bound to the MCP tool that fulfils it.
#[derive(Debug, Clone)]
struct BoundTool {
	/// The name the model calls.
	name: String,
	/// The tool on the MCP backend.
	mcp_tool: String,
	/// Index into `Interception::runtimes`.
	runtime: usize,
}

/// Per-request interception state, built when the request is parsed.
pub struct Interception {
	config: Arc<ServerToolsConfig>,
	policy: Policy,
	tools: Vec<BoundTool>,
	/// The client's request after the rewrite. Follow-up turns are appended to a copy of it.
	conversation: Conversation,
	/// The client request headers, used when rendering follow-up requests.
	headers: HeaderMap,
	runtimes: Vec<Arc<ToolRuntime>>,
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
	fn wire(&self) -> Wire {
		self.conversation.wire()
	}

	fn tool(&self, name: &str) -> Option<&BoundTool> {
		self.tools.iter().find(|t| t.name == name)
	}

	fn names(&self) -> HashSet<&str> {
		self.tools.iter().map(|t| t.name.as_str()).collect()
	}
}

/// Resolves MCP runtimes and tool definitions while a request's tools are being bound.
struct Binder<'a> {
	inputs: &'a Arc<ProxyInputs>,
	parts: &'a Parts,
	runtimes: Vec<Arc<ToolRuntime>>,
	by_target: HashMap<(Strng, Option<Strng>), usize>,
	bound: Vec<BoundTool>,
}

impl<'a> Binder<'a> {
	fn new(inputs: &'a Arc<ProxyInputs>, parts: &'a Parts) -> Self {
		Self {
			inputs,
			parts,
			runtimes: Vec::new(),
			by_target: HashMap::new(),
			bound: Vec::new(),
		}
	}

	fn runtime(&mut self, backend: &Strng, target: Option<&Strng>) -> anyhow::Result<usize> {
		let key = (backend.clone(), target.cloned());
		if let Some(idx) = self.by_target.get(&key) {
			return Ok(*idx);
		}
		let runtime = ToolRuntime::new(self.inputs, backend, target.map(|t| t.as_str()), self.parts)?;
		self.runtimes.push(Arc::new(runtime));
		let idx = self.runtimes.len() - 1;
		self.by_target.insert(key, idx);
		Ok(idx)
	}

	/// Bind one declared tool to its operator mapping. Returns the definition the model sees, or
	/// `None` when the mapping cannot be resolved, in which case the tool is left alone.
	async fn bind_mapping(
		&mut self,
		name: &str,
		mapping: &ServerToolMapping,
	) -> Option<ToolDefinition> {
		let runtime = match self.runtime(&mapping.mcp.backend, mapping.mcp.target.as_ref()) {
			Ok(idx) => idx,
			Err(e) => {
				warn!(tool = %name, backend = %mapping.mcp.backend, "server tool not intercepted: {e}");
				return None;
			},
		};
		let definition = match &mapping.input_schema {
			Some(schema) => ToolDefinition {
				description: mapping.description.clone(),
				input_schema: schema.clone(),
			},
			None => match self.runtimes[runtime]
				.tool_definition(&mapping.mcp.tool)
				.await
			{
				Ok(Some(mut def)) => {
					if mapping.description.is_some() {
						def.description = mapping.description.clone();
					}
					def
				},
				Ok(None) => {
					warn!(
						tool = %name,
						backend = %mapping.mcp.backend,
						mcp_tool = %mapping.mcp.tool,
						"server tool not intercepted: the MCP backend does not serve that tool"
					);
					return None;
				},
				Err(e) => {
					warn!(tool = %name, backend = %mapping.mcp.backend, "server tool not intercepted: {e}");
					return None;
				},
			},
		};
		self.bound.push(BoundTool {
			name: name.to_string(),
			mcp_tool: mapping.mcp.tool.to_string(),
			runtime,
		});
		Some(definition)
	}

	/// Bind the tools of a remote MCP server the client declared. Returns the function tools the
	/// model sees in place of the descriptor.
	async fn bind_server(
		&mut self,
		descriptor: &rt::McpDescriptor,
		server: &ServerToolMcpServer,
		taken: &HashSet<String>,
	) -> Result<Vec<(String, ToolDefinition)>, AIError> {
		let runtime = match self.runtime(&server.backend, server.target.as_ref()) {
			Ok(idx) => idx,
			Err(e) => {
				warn!(server = %descriptor.server_label, backend = %server.backend, "mcp server not intercepted: {e}");
				return Ok(Vec::new());
			},
		};
		let definitions = match self.runtimes[runtime].tool_definitions().await {
			Ok(defs) => defs,
			Err(e) => {
				warn!(server = %descriptor.server_label, backend = %server.backend, "mcp server not intercepted: {e}");
				return Ok(Vec::new());
			},
		};
		let mut out = Vec::new();
		for (name, def) in definitions {
			if !descriptor.allows(&name) {
				continue;
			}
			if taken.contains(&name) || self.bound.iter().any(|b| b.name == name) {
				warn!(server = %descriptor.server_label, tool = %name, "mcp tool skipped: the name is already taken");
				continue;
			}
			if descriptor.requires_approval(&name) && !server.skip_approval {
				return Err(AIError::ServerToolRequest(strng::format!(
					"mcp server {} requires approval for tool {name}; set require_approval to \"never\" or enable skipApproval on the gateway",
					descriptor.server_label
				)));
			}
			self.bound.push(BoundTool {
				name: name.clone(),
				mcp_tool: name.clone(),
				runtime,
			});
			out.push((name, def));
		}
		Ok(out)
	}

	fn finish(
		self,
		config: &Arc<ServerToolsConfig>,
		policy: &Policy,
		conversation: Conversation,
		headers: HeaderMap,
		max_iterations: u32,
	) -> Option<Arc<Interception>> {
		if self.bound.is_empty() {
			return None;
		}
		debug!(tools = ?self.bound, max_iterations, "intercepting server tools");
		Some(Arc::new(Interception {
			config: config.clone(),
			policy: policy.clone(),
			tools: self.bound,
			conversation,
			headers,
			runtimes: self.runtimes,
			max_iterations,
		}))
	}
}

/// Rewrite a Messages client's server tools when the policy maps them. Returns `None`, leaving
/// the request untouched, when nothing applies or the MCP tool definitions cannot be resolved.
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
	let mut binder = Binder::new(inputs, parts);
	let mut tools: Vec<InterceptedTool> = Vec::with_capacity(found.len());
	let mut definitions = Vec::with_capacity(found.len());
	for tool in found {
		let mapping = &config.tools[tool.mapping];
		if let Some(def) = binder.bind_mapping(&tool.name, mapping).await {
			tools.push(tool);
			definitions.push(def);
		}
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
	binder.finish(
		config,
		policy,
		Conversation::Messages(req.clone()),
		parts.headers.clone(),
		max_iterations,
	)
}

/// Rewrite a Responses client's built-in tools and `mcp` servers when the policy maps them.
/// Returns `None`, leaving the request untouched, when nothing applies. Fails when the client
/// requires approval for an MCP tool the gateway would run.
pub async fn intercept_responses(
	config: &Arc<ServerToolsConfig>,
	policy: &Policy,
	req: &mut types::responses::Request,
	inputs: &Arc<ProxyInputs>,
	parts: &Parts,
) -> Result<Option<Arc<Interception>>, AIError> {
	let builtins = rt::find_builtin_tools(req, &config.matchers());
	let descriptors = if config.mcp_servers.is_empty() {
		Vec::new()
	} else {
		rt::find_mcp_descriptors(req)
	};
	if builtins.is_empty() && descriptors.is_empty() {
		return Ok(None);
	}
	let taken = rt::client_tool_names(req);
	let mut binder = Binder::new(inputs, parts);

	let mut tools: Vec<InterceptedTool> = Vec::with_capacity(builtins.len());
	let mut definitions = Vec::with_capacity(builtins.len());
	for tool in builtins {
		let mapping = &config.tools[tool.mapping];
		if let Some(def) = binder.bind_mapping(&tool.name, mapping).await {
			tools.push(tool);
			definitions.push(def);
		}
	}
	// Built-ins are replaced in place, so the descriptor positions below stay valid.
	rt::rewrite_builtin_tools(req, &tools, &definitions);

	let mut edits = Vec::new();
	for descriptor in &descriptors {
		let Some(server) = config
			.mcp_servers
			.iter()
			.find(|s| s.matches(&descriptor.server_label, descriptor.server_url.as_deref()))
		else {
			debug!(server = %descriptor.server_label, "mcp server not mapped; leaving it to the provider");
			continue;
		};
		let exposed = binder.bind_server(descriptor, server, &taken).await?;
		if exposed.is_empty() {
			continue;
		}
		edits.push((
			descriptor.index,
			exposed
				.iter()
				.map(|(name, def)| rt::function_tool(name, def))
				.collect(),
		));
	}
	rt::replace_tools(req, edits);

	Ok(binder.finish(
		config,
		policy,
		Conversation::Responses(req.clone()),
		parts.headers.clone(),
		config.max_iterations.max(1),
	))
}

/// Rewrite a Chat Completions client's `web_search_options` into a function tool when a
/// `web_search*` mapping exists. Returns `None`, leaving the request untouched, otherwise.
pub async fn intercept_completions(
	config: &Arc<ServerToolsConfig>,
	policy: &Policy,
	req: &mut types::completions::Request,
	inputs: &Arc<ProxyInputs>,
	parts: &Parts,
) -> Option<Arc<Interception>> {
	let tool = ct::find_web_search(req, &config.matchers())?;
	let mut binder = Binder::new(inputs, parts);
	let def = binder
		.bind_mapping(&tool.name, &config.tools[tool.mapping])
		.await?;
	ct::rewrite_web_search(req, &tool, &def);
	binder.finish(
		config,
		policy,
		Conversation::Completions(req.clone()),
		parts.headers.clone(),
		config.max_iterations.max(1),
	)
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
	events: Vec<SseEvent>,
	parts: ::http::response::Parts,
	raw: Bytes,
}

enum TurnOutcome {
	Message(Turn),
	/// The upstream answered with a non-success status.
	Failed(BufferedResponse),
}

struct LoopState {
	conversation: Conversation,
	totals: Totals,
	iteration: u32,
	last_fingerprint: Option<String>,
}

impl LoopState {
	fn new(interception: &Interception) -> Self {
		Self {
			conversation: interception.conversation.clone(),
			totals: Totals::new(interception.wire()),
			iteration: 0,
			last_fingerprint: None,
		}
	}
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
		wire: Wire,
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
			let message = wire
				.accumulate(&events)
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
		conversation: &Conversation,
		req: &LLMRequest,
		catalog: Option<&Arc<catalog::ModelCatalog>>,
	) -> Result<Vec<u8>, AIError> {
		let translation = self.chat_translation(
			req.input_format,
			Some(&req.request_model),
			catalog.map(|c| c.as_handle()),
		)?;
		let rendered = translation.render_request(
			conversation.chat_request(),
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
	let calls = interception.wire().tool_uses(message);
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
	let wire = interception.wire();
	let mut results = Vec::with_capacity(calls.len());
	for call in calls {
		let tool = interception
			.tool(&call.name)
			.ok_or_else(|| server_tool_error(format!("unmapped tool {}", call.name)))?;
		let runtime = interception
			.runtimes
			.get(tool.runtime)
			.ok_or_else(|| server_tool_error(format!("no runtime for tool {}", call.name)))?;
		let outcome = runtime.call(&tool.mcp_tool, &call.input, mcp_log).await;
		let item = match outcome {
			Ok(outcome) => wire.tool_result(
				call,
				&outcome.content,
				outcome.is_error,
				interception.config.max_result_bytes,
			),
			Err(e) => match interception.config.failure_mode {
				ServerToolFailureMode::FailClosed => {
					return Err(server_tool_error(format!("tool {} failed: {e}", call.name)));
				},
				ServerToolFailureMode::FailOpen => {
					warn!(tool = %call.name, "server tool failed; reporting the error to the model: {e}");
					wire.error_result(call, &e.to_string())
				},
			},
		};
		results.push(item);
	}
	Ok(results)
}

async fn close_runtimes(interception: &Interception) {
	for runtime in &interception.runtimes {
		runtime.close().await;
	}
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
		let wire = interception.wire();
		let mut state = LoopState::new(&interception);
		let mut resp = first;
		let result = loop {
			let turn = match self.read_turn(wire, &req, model_catalog, resp).await? {
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
						wire.strip(&mut message, &interception.names());
					}
					state.totals.add(&message);
					state.totals.set(&mut message);
					let translated = wire.parse_final(message)?;
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
					state.totals.add(&turn.message);
					state.iteration += 1;
					state.conversation.append(&turn.message, results);
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
		let wire = interception.wire();

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
					amend.non_atomic_mutate(|r| r.response = wire.llm_response_for(&message, log_content));
					let _ = tx.send(Ok(st::encode_sse(&events))).await;
				},
				Ok(None) => {},
				Err(e) => {
					warn!("server tool turn failed: {e}");
					let _ = tx.send(Ok(wire.error_event(&e.to_string()))).await;
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
	) -> Result<Option<(Vec<SseEvent>, Value)>, AIError> {
		let wire = interception.wire();
		let mut state = LoopState::new(interception);
		let mut resp = first;
		loop {
			let Some(turn) = with_keepalive(
				tx,
				keepalive,
				wire,
				self.read_turn(wire, req, catalog, resp),
			)
			.await
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
					state.totals.add(&message);
					state.totals.set(&mut message);
					let events = if strip {
						wire.strip(&mut message, &interception.names());
						wire.synthesize(&message)
					} else {
						let mut events = turn.events;
						state.totals.patch(&mut events);
						events
					};
					return Ok(Some((events, message)));
				},
				Next::Execute(calls) => {
					let Some(results) = with_keepalive(
						tx,
						keepalive,
						wire,
						execute(interception, &calls, resend.mcp_log.as_ref()),
					)
					.await
					else {
						return Ok(None);
					};
					let results = results?;
					state.last_fingerprint = Some(st::fingerprint(&calls));
					state.totals.add(&turn.message);
					state.iteration += 1;
					state.conversation.append(&turn.message, results);
					let body = self.render_follow_up(interception, &state.conversation, req, catalog)?;
					let Some(sent) = with_keepalive(tx, keepalive, wire, resend.send(body)).await else {
						return Ok(None);
					};
					resp = sent.map_err(|e| server_tool_error(format!("follow-up request failed: {e}")))?;
				},
			}
		}
	}
}

/// Poll `fut` while sending a keepalive every `interval`. Returns `None` if the client is gone.
async fn with_keepalive<T>(
	tx: &mpsc::Sender<Result<Bytes, std::io::Error>>,
	interval: Duration,
	wire: Wire,
	fut: impl Future<Output = T>,
) -> Option<T> {
	tokio::pin!(fut);
	let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(100)));
	ticker.tick().await;
	loop {
		tokio::select! {
			out = &mut fut => return Some(out),
			_ = ticker.tick() => {
				if tx.send(Ok(wire.keepalive())).await.is_err() {
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
