mod client;
mod openapi;
mod sse;
mod stdio;
mod streamablehttp;

use std::collections::HashMap;
use std::io;

use agent_core::prelude::AssertSize;
pub(crate) use client::McpHttpClient;
use itertools::Itertools;
pub use openapi::ParseError as OpenAPIParseError;
use rmcp::model::{
	ClientNotification, ClientRequest, ExtensionCapabilities, GetMeta, JsonObject, JsonRpcRequest,
};
use rmcp::transport::TokioChildProcess;
use rmcp::transport::common::http_header::HEADER_SESSION_ID;
use thiserror::Error;
use tokio::process::Command;

use crate::mcp::mergestream::Messages;
use crate::mcp::router::{McpBackendGroup, McpTarget};
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
use crate::mcp::{FailureMode, mergestream, upstream};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{McpPrefixMode, McpTargetSpec};
use crate::*;

#[derive(Debug, Clone)]
pub struct IncomingRequestContext {
	method: ::http::Method,
	uri: ::http::Uri,
	headers: http::HeaderMap,
	ext: ::http::Extensions,
	authority: Option<::http::uri::Authority>,
}

impl IncomingRequestContext {
	#[cfg(test)]
	pub fn empty() -> Self {
		Self {
			method: ::http::Method::GET,
			uri: ::http::Uri::from_static("/"),
			headers: http::HeaderMap::new(),
			ext: ::http::Extensions::new(),
			authority: None,
		}
	}
	pub fn new(parts: &::http::request::Parts) -> Self {
		Self {
			method: parts.method.clone(),
			uri: parts.uri.clone(),
			headers: parts.headers.clone(),
			ext: parts.extensions.clone(),
			authority: parts.uri.authority().cloned(),
		}
	}
	pub fn headers_mut(&mut self) -> &mut http::HeaderMap {
		&mut self.headers
	}
	pub fn extensions(&self) -> &::http::Extensions {
		&self.ext
	}
	pub fn extensions_mut(&mut self) -> &mut ::http::Extensions {
		&mut self.ext
	}
	pub fn apply(&self, req: &mut http::Request) -> anyhow::Result<()> {
		req.extensions_mut().extend(self.ext.clone());
		let explicit_auto_hostname = req
			.extensions()
			.get::<crate::http::filters::AutoHostname>()
			.is_some_and(|auto| auto.explicit);
		if explicit_auto_hostname {
			let authority = req.uri().authority().map(|a| strng::new(a.as_str()));
			if let Some(authority) = authority
				&& let Some(auto) = req
					.extensions_mut()
					.get_mut::<crate::http::filters::AutoHostname>()
				&& auto.target.is_none()
			{
				auto.target = Some(authority);
			}
		}
		for (k, v) in &self.headers {
			// Remove headers we do not want to propagate to the backend
			if k == http::header::CONTENT_ENCODING
				|| k == http::header::CONTENT_LENGTH
				|| k.as_str().eq_ignore_ascii_case(HEADER_SESSION_ID)
			{
				continue;
			}
			if !req.headers().contains_key(k) {
				req.headers_mut().insert(k.clone(), v.clone());
			}
		}
		let Some(authority) = self.authority.clone() else {
			return Ok(());
		};
		http::modify_req_uri(req, |uri| {
			uri.authority = Some(authority);
			Ok(())
		})
	}
	// SEP-414: copy W3C trace context into the message's `_meta` (un-prefixed keys, per spec).
	// The only trace carrier for stdio upstreams, which have no request headers.
	fn stamp_trace_context<T: GetMeta>(&self, msg: &mut T) {
		for key in ["traceparent", "tracestate", "baggage"] {
			let Some(value) = self.headers.get(key).and_then(|v| v.to_str().ok()) else {
				continue;
			};
			msg.get_meta_mut().0.insert(
				key.to_string(),
				serde_json::Value::String(value.to_string()),
			);
		}
	}
	// Empty-bodied Request mirroring the incoming headers/extensions, for CEL input.
	pub fn as_request(&self) -> crate::http::Request {
		let mut req = ::http::Request::new(crate::http::Body::empty());
		*req.method_mut() = self.method.clone();
		*req.uri_mut() = self.uri.clone();
		*req.headers_mut() = self.headers.clone();
		*req.extensions_mut() = self.ext.clone();
		req
	}
}

#[derive(Debug, Error)]
pub enum UpstreamError {
	#[error("unknown {resource_type}: {resource_name}")]
	Authorization {
		resource_type: String,
		resource_name: String,
	},
	#[error("mcpGuardrails rejected: {}", .0.message)]
	McpGuardrails(rmcp::ErrorData),
	#[error("invalid request: {0}")]
	InvalidRequest(String),
	/// A server-side availability/capability gap. Distinct from `InvalidRequest`,
	/// so client-visible errors do not blame the request for a backend condition.
	#[error("{0}")]
	Unavailable(String),
	#[error("unsupported method: {0}")]
	InvalidMethod(String),
	#[error("stdio upstream error: {0}")]
	ServiceError(#[from] rmcp::ServiceError),
	#[error("http upstream error: {0}")]
	Http(#[from] mcp::ClientError),
	#[error("openapi upstream error: {0}")]
	OpenAPIError(#[from] anyhow::Error),
	#[error("{0}")]
	Proxy(#[from] ProxyError),
	#[error("stdio upstream error: {0}")]
	Stdio(#[from] io::Error),
	#[error("stdio server exited")]
	StdioShutdown,
	#[error("upstream closed on send")]
	Send,
	#[error("upstream closed on receive")]
	Recv,
}

// UpstreamTarget defines a source for MCP information.
#[derive(Debug)]
pub(crate) enum Upstream {
	McpStreamable(streamablehttp::Client),
	McpSSE(sse::Client),
	McpStdio(stdio::Process),
	OpenAPI(Box<openapi::Handler>),
}

impl Upstream {
	pub fn get_session_state(&self) -> Option<http::sessionpersistence::MCPSession> {
		match self {
			Upstream::McpStreamable(c) => Some(c.get_session_state()),
			Upstream::McpSSE(c) => Some(c.get_session_state()),
			Upstream::OpenAPI(c) => Some(c.get_session_state()),
			_ => None,
		}
	}

	pub fn set_session_id(&self, id: Option<&str>, pinned: Option<SocketAddr>) {
		match self {
			Upstream::McpStreamable(c) => c.set_session_id(id, pinned),
			Upstream::McpSSE(c) => c.set_session_id(id, pinned),
			Upstream::McpStdio(_) => {},
			Upstream::OpenAPI(c) => c.set_session_id(id, pinned),
		}
	}

	pub(crate) async fn delete(&self, ctx: &IncomingRequestContext) -> Result<(), UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => {
				c.stop().await?;
			},
			Upstream::McpStreamable(c) => {
				if c.has_session_id() {
					c.send_delete(ctx).await?;
				}
			},
			Upstream::McpSSE(c) => {
				c.stop().await?;
			},
			Upstream::OpenAPI(_) => {
				// No need to do anything here
			},
		}
		Ok(())
	}
	pub(crate) async fn get_event_stream(
		&self,
		ctx: &IncomingRequestContext,
	) -> Result<mergestream::Messages, UpstreamError> {
		match &self {
			Upstream::McpStdio(c) => Ok(c.get_event_stream().await?),
			Upstream::McpSSE(c) => c.connect_to_event_stream(ctx).await,
			Upstream::McpStreamable(c) => c
				.get_event_stream(ctx)
				.await?
				.try_into()
				.map_err(Into::into),
			Upstream::OpenAPI(_m) => Ok(Messages::pending()),
		}
	}
	pub(crate) async fn generic_stream(
		&self,
		mut request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<mergestream::Messages, UpstreamError> {
		// stdio/SSE route server-initiated notifications through `get_event_stream`,
		// which `subscriptions/listen` never merges. Reject them before sending
		// an ack over a silent stream. OpenAPI has no notifications to lose.
		if matches!(&self, Upstream::McpStdio(_) | Upstream::McpSSE(_))
			&& matches!(
				&request.request,
				ClientRequest::SubscriptionsListenRequest(_)
			) {
			return Err(UpstreamError::Unavailable(
				"subscriptions/listen is not supported for stdio/SSE upstreams".to_string(),
			));
		}
		ctx.stamp_trace_context(&mut request.request);
		match &self {
			Upstream::McpStdio(c) => Ok(mergestream::Messages::from(
				Box::pin(c.send_message(request, ctx).assert_size::<{ 6 * 1024 }>()).await?,
			)),
			Upstream::McpSSE(c) => Ok(mergestream::Messages::from(
				Box::pin(c.send_message(request, ctx).assert_size::<{ 6 * 1024 }>()).await?,
			)),
			Upstream::McpStreamable(c) => {
				let is_init = matches!(&request.request, &ClientRequest::InitializeRequest(_));
				let res = Box::pin(c.send_request(request, ctx).assert_size::<{ 6 * 1024 }>()).await?;
				if is_init {
					let sid = match &res {
						StreamableHttpPostResponse::Accepted => None,
						StreamableHttpPostResponse::Json(_, sid) => sid.as_ref(),
						StreamableHttpPostResponse::Sse(_, sid) => sid.as_ref(),
					};
					c.set_session_id(sid.map(|s| s.as_str()), None);
				}
				res.try_into().map_err(Into::into)
			},
			Upstream::OpenAPI(c) => {
				Ok(Box::pin(c.send_message(request, ctx).assert_size::<{ 6 * 1024 }>()).await?)
			},
		}
	}

	pub(crate) async fn generic_notification(
		&self,
		mut request: ClientNotification,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		ctx.stamp_trace_context(&mut request);
		match &self {
			Upstream::McpStdio(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::McpSSE(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::McpStreamable(c) => {
				c.send_notification(request, ctx).await?;
			},
			Upstream::OpenAPI(_) => {},
		}
		Ok(())
	}
}

#[derive(Debug)]
pub(crate) struct UpstreamGroup {
	backend: McpBackendGroup,
	client: PolicyClient,
	by_name: IndexMap<Strng, Arc<upstream::Upstream>>,

	// per-target set of capabilities; we record the capabilities from a legacy
	// target's initialize response so a modern client can see them in discover.
	extensions: RwLock<HashMap<Strng, ExtensionCapabilities>>,

	// If we have one target and prefixMode is not Always, names and URIs pass
	// through unchanged and all calls route to this target.
	pub default_target_name: Option<String>,
	pub prefix_mode: McpPrefixMode,
	pub is_multiplexing: bool,
	pub failure_mode: FailureMode,
}

impl UpstreamGroup {
	pub fn size(&self) -> usize {
		self.by_name.len()
	}

	pub(crate) fn new(client: PolicyClient, backend: McpBackendGroup) -> Result<Self, mcp::Error> {
		let is_multiplexing = backend.targets.len() != 1;
		let default_target_name = (!is_multiplexing && backend.prefix_mode != McpPrefixMode::Always)
			.then(|| backend.targets[0].name.to_string());
		let mut s = Self {
			failure_mode: backend.failure_mode,
			prefix_mode: backend.prefix_mode,
			backend,
			client,
			by_name: IndexMap::new(),
			extensions: RwLock::new(HashMap::new()),
			default_target_name,
			is_multiplexing,
		};
		s.setup_connections()?;
		if s.by_name.is_empty() {
			if s.backend.targets.is_empty() && s.failure_mode == FailureMode::FailOpen {
				warn!(
					"MCP backend configured with zero targets and failure_mode=failOpen; allowing startup to avoid downstream retry loops"
				);
				return Ok(s);
			}
			return Err(mcp::Error::NoBackends);
		}
		Ok(s)
	}

	pub(crate) fn setup_connections(&mut self) -> Result<(), mcp::Error> {
		for tgt in &self.backend.targets {
			debug!("initializing target: {}", tgt.name);
			match self.setup_upstream(tgt.as_ref()) {
				Ok(transport) => {
					self.by_name.insert(tgt.name.clone(), Arc::new(transport));
				},
				Err(e) => {
					if self.failure_mode == FailureMode::FailOpen {
						warn!(
							"failed to initialize target '{}', skipping (failure_mode=FailOpen): {}",
							tgt.name, e
						);
					} else {
						return Err(e);
					}
				},
			}
		}
		Ok(())
	}

	pub(crate) fn iter_named(&self) -> impl Iterator<Item = (Strng, Arc<upstream::Upstream>)> {
		self.by_name.iter().map(|(k, v)| (k.clone(), v.clone()))
	}
	pub(crate) fn get(&self, name: &str) -> anyhow::Result<&upstream::Upstream> {
		self
			.by_name
			.get(name)
			.map(|v| v.as_ref())
			.ok_or_else(|| anyhow::anyhow!("requested target {name} is not initialized",))
	}
	/// Returns the stored name key if it exists in the upstream map.
	/// Used by `parse_resource_uri` to get a stable `&str` reference.
	pub(crate) fn get_name(&self, name: &str) -> Option<&str> {
		self.by_name.get_key_value(name).map(|(k, _)| k.as_str())
	}

	pub(crate) fn stateful(&self) -> bool {
		self.backend.stateful
	}

	/// True when some target's `delete` does teardown work even without an upstream
	/// session id: stdio processes and SSE streams hold per-connection state.
	pub(crate) fn has_connection_teardown(&self) -> bool {
		self
			.by_name
			.values()
			.any(|u| matches!(u.as_ref(), Upstream::McpStdio(_) | Upstream::McpSSE(_)))
	}

	pub(crate) fn record_extensions(&self, target: &str, extensions: Option<&ExtensionCapabilities>) {
		let Some(ext) = extensions else {
			return;
		};
		if ext.is_empty() {
			return;
		}
		let mut store = self.extensions.write().expect("write lock");
		store.insert(strng::new(target), ext.clone());
	}

	/// merged view of all target's per-extension capabilities, combining the
	/// results in hand from the current fanout with those recorded at initialize
	pub(crate) fn merged_extensions(
		&self,
		fresh: &HashMap<Strng, ExtensionCapabilities>,
	) -> Option<ExtensionCapabilities> {
		let store = self.extensions.read().expect("read lock");
		merge_extension_capabilities(self.by_name.keys().filter_map(|name| {
			fresh
				.get(name)
				.or_else(|| store.get(name))
				.map(|ext| (name.as_str(), ext))
		}))
	}

	fn setup_upstream(&self, target: &McpTarget) -> Result<upstream::Upstream, mcp::Error> {
		trace!("connecting to target: {}", target.name);
		let target = match &target.spec {
			McpTargetSpec::Sse(sse) => {
				debug!("starting sse transport for target: {}", target.name);
				let path = match sse.path.as_str() {
					"" => "/sse",
					_ => sse.path.as_str(),
				};

				let upstream_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for SSE"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				let client = sse::Client::new(upstream_client, path.into());

				upstream::Upstream::McpSSE(client)
			},
			McpTargetSpec::Mcp(mcp) => {
				debug!(
					"starting streamable http transport for target: {}",
					target.name
				);
				let path = match mcp.path.as_str() {
					"" => "/mcp",
					_ => mcp.path.as_str(),
				};

				let http_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for MCP"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				let client = streamablehttp::Client::new(http_client, path.into())
					.map_err(|_| mcp::Error::InvalidSessionIdHeader)?;

				upstream::Upstream::McpStreamable(client)
			},
			McpTargetSpec::Stdio {
				cmd,
				args,
				env,
				clear_env,
			} => {
				debug!("starting stdio transport for target: {}", target.name);
				#[cfg(target_os = "windows")]
				// Command has some weird behavior on Windows where it expects the executable extension to be
				// .exe. The which create will resolve the actual command for us.
				// See https://github.com/rust-lang/rust/issues/37519#issuecomment-1694507663
				// for more context.
				let cmd = which::which(cmd).map_err(|e| mcp::Error::Stdio(io::Error::other(e)))?;
				#[cfg(target_family = "unix")]
				let mut c = Command::new(cmd);
				#[cfg(target_os = "windows")]
				let mut c = Command::new(&cmd);
				c.args(args);
				if *clear_env {
					c.env_clear();
				}
				for (k, v) in env {
					c.env(k, v);
				}
				let proc = TokioChildProcess::new(c).map_err(mcp::Error::Stdio)?;
				upstream::Upstream::McpStdio(upstream::stdio::Process::new(proc))
			},
			McpTargetSpec::OpenAPI(open) => {
				// Renamed for clarity
				debug!("starting OpenAPI transport for target: {}", target.name);

				let tools = openapi::parse_openapi_schema(&open.schema).map_err(mcp::Error::OpenAPI)?;
				let prefix = openapi::get_server_prefix(&open.schema).map_err(mcp::Error::OpenAPI)?;

				let http_client = McpHttpClient::new(
					self.client.clone(),
					target
						.backend
						.clone()
						.expect("there must be a backend for OpenAPI"),
					target.backend_policies.clone(),
					self.backend.stateful,
					target.name.to_string(),
				);
				upstream::Upstream::OpenAPI(Box::new(openapi::Handler::new(
					http_client,
					tools,  // From parse_openapi_schema
					prefix, // From get_server_prefix
				)))
			},
		};

		Ok(target)
	}
}

/// Extension names are unioned across all targets, but we only keep settings when all targets agree
/// on the same settings object. If any target has a different settings object for the same
/// extension, we log a warning and advertise the extension with empty settings.
fn merge_extension_capabilities<'a>(
	per_target: impl Iterator<Item = (&'a str, &'a ExtensionCapabilities)>,
) -> Option<ExtensionCapabilities> {
	let merged: ExtensionCapabilities = per_target
		.flat_map(|(target, ext)| ext.iter().map(move |(k, v)| (k.as_str(), (target, v))))
		.into_group_map()
		.into_iter()
		.map(|(k, advertisers)| {
			let settings = if advertisers.iter().map(|(_, v)| v).all_equal() {
				advertisers[0].1.clone()
			} else {
				warn!(
					extension = %k,
					targets = ?advertisers.iter().map(|(t, _)| t).collect::<Vec<_>>(),
					"targets advertise divergent extension settings, advertising support without settings"
				);
				JsonObject::default()
			};
			(k.to_string(), settings)
		})
		.collect();
	(!merged.is_empty()).then_some(merged)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ext(id: &str, settings: serde_json::Value) -> ExtensionCapabilities {
		let mut e = ExtensionCapabilities::new();
		e.insert(id.to_string(), settings.as_object().cloned().unwrap());
		e
	}

	#[test]
	fn merge_extensions_agreeing_settings_pass_through() {
		let a = ext("io.modelcontextprotocol/ui", serde_json::json!({"x": 1}));
		let b = ext("io.modelcontextprotocol/ui", serde_json::json!({"x": 1}));
		let merged = merge_extension_capabilities([("a", &a), ("b", &b)].into_iter()).unwrap();
		assert_eq!(
			merged.get("io.modelcontextprotocol/ui"),
			serde_json::json!({"x": 1}).as_object()
		);
	}

	#[test]
	fn merge_extensions_divergent_settings_advertise_empty() {
		let a = ext("io.modelcontextprotocol/ui", serde_json::json!({"x": 1}));
		let b = ext("io.modelcontextprotocol/ui", serde_json::json!({"x": 2}));
		let merged = merge_extension_capabilities([("a", &a), ("b", &b)].into_iter()).unwrap();
		assert_eq!(
			merged.get("io.modelcontextprotocol/ui"),
			serde_json::json!({}).as_object()
		);
	}

	#[test]
	fn merge_extensions_unions_distinct_extensions() {
		let a = ext("io.modelcontextprotocol/ui", serde_json::json!({}));
		let b = ext("example/other", serde_json::json!({"y": true}));
		let merged = merge_extension_capabilities([("a", &a), ("b", &b)].into_iter()).unwrap();
		assert_eq!(merged.len(), 2);
		assert_eq!(
			merged.get("example/other"),
			serde_json::json!({"y": true}).as_object()
		);
		assert!(merge_extension_capabilities(std::iter::empty()).is_none());
	}

	#[test]
	fn incoming_request_context_applies_original_authority() {
		let parts = ::http::Request::builder()
			.uri("http://original.example/mcp")
			.body(())
			.unwrap()
			.into_parts()
			.0;
		let ctx = IncomingRequestContext::new(&parts);
		let mut req = ::http::Request::builder()
			.uri("http://svc.default.svc.cluster.local:80/mcp")
			.body(crate::http::Body::empty())
			.unwrap();

		ctx.apply(&mut req).unwrap();

		assert_eq!(
			req.uri().authority().map(|a| a.as_str()),
			Some("original.example")
		);
		assert_eq!(req.headers().get(http::header::HOST), None);
		assert_eq!(req.uri().path(), "/mcp");
	}

	fn ctx_with_headers(headers: &[(&str, &str)]) -> IncomingRequestContext {
		let mut builder = ::http::Request::builder()
			.uri("http://example/")
			.method("GET");
		for (k, v) in headers {
			builder = builder.header(*k, *v);
		}
		let parts = builder.body(()).unwrap().into_parts().0;
		IncomingRequestContext::new(&parts)
	}

	fn empty_upstream_req() -> http::Request {
		::http::Request::builder()
			.uri("http://upstream/")
			.body(http::Body::empty())
			.unwrap()
	}

	#[test]
	fn apply_strips_inbound_mcp_session_id() {
		let ctx = ctx_with_headers(&[(HEADER_SESSION_ID, "client-sid"), ("x-trace", "abc")]);
		let mut req = empty_upstream_req();
		ctx.apply(&mut req).unwrap();
		assert!(req.headers().get(HEADER_SESSION_ID).is_none());
		assert_eq!(req.headers().get("x-trace").unwrap(), "abc");
	}

	#[test]
	fn apply_preserves_upstream_session_id_when_already_set() {
		let ctx = ctx_with_headers(&[(HEADER_SESSION_ID, "client-sid")]);
		let mut req = empty_upstream_req();
		req.headers_mut().insert(
			::http::HeaderName::from_static("mcp-session-id"),
			::http::HeaderValue::from_static("upstream-sid"),
		);
		ctx.apply(&mut req).unwrap();
		assert_eq!(
			req.headers().get(HEADER_SESSION_ID).unwrap(),
			"upstream-sid"
		);
	}

	#[test]
	fn apply_strips_content_encoding_and_length() {
		let ctx = ctx_with_headers(&[
			(http::header::CONTENT_ENCODING.as_str(), "gzip"),
			(http::header::CONTENT_LENGTH.as_str(), "42"),
		]);
		let mut req = empty_upstream_req();
		ctx.apply(&mut req).unwrap();
		assert!(req.headers().get(http::header::CONTENT_ENCODING).is_none());
		assert!(req.headers().get(http::header::CONTENT_LENGTH).is_none());
	}

	#[test]
	fn apply_propagates_other_headers() {
		let ctx = ctx_with_headers(&[("authorization", "Bearer token"), ("x-request-id", "req-1")]);
		let mut req = empty_upstream_req();
		ctx.apply(&mut req).unwrap();
		assert_eq!(req.headers().get("authorization").unwrap(), "Bearer token");
		assert_eq!(req.headers().get("x-request-id").unwrap(), "req-1");
	}

	fn ping_request() -> ClientRequest {
		serde_json::from_value(serde_json::json!({"method": "ping"})).unwrap()
	}

	#[test]
	fn stamp_trace_context_copies_w3c_keys_into_meta_unprefixed() {
		let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
		let ctx = ctx_with_headers(&[
			("traceparent", traceparent),
			("tracestate", "vendor=abc"),
			("baggage", "userId=42"),
			("authorization", "Bearer token"),
		]);
		let mut req = ping_request();
		ctx.stamp_trace_context(&mut req);

		let meta = &req.get_meta().0;
		// SEP-414: un-prefixed keys, verbatim W3C values, matching the forwarded headers.
		assert_eq!(meta.get("traceparent").unwrap(), traceparent);
		assert_eq!(meta.get("tracestate").unwrap(), "vendor=abc");
		assert_eq!(meta.get("baggage").unwrap(), "userId=42");
		// Non-trace headers are not smuggled into `_meta`.
		assert!(meta.get("authorization").is_none());
	}

	#[test]
	fn stamp_trace_context_noop_without_trace_headers() {
		let ctx = ctx_with_headers(&[("authorization", "Bearer token")]);
		let mut req = ping_request();
		ctx.stamp_trace_context(&mut req);
		assert!(req.get_meta().0.get("traceparent").is_none());
	}
}
