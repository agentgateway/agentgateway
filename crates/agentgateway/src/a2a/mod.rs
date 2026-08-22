use agent_core::strng::Strng;
use http::{Request, Uri, header};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use crate::http::{Body, BodyInspection, Response, filters};
use crate::json;
use crate::types::agent::A2aPolicy;

pub async fn apply_to_request(_: &A2aPolicy, req: &mut Request<Body>) -> RequestType {
	// Possible options are POST a JSON-RPC message or GET /.well-known/agent.json
	// For agent card, we will process only on the response
	classify_request(req).await
}

async fn classify_request(req: &mut Request<Body>) -> RequestType {
	// Possible options are POST a JSON-RPC message or GET /.well-known/agent.json
	// For agent card, we will process only on the response
	match (req.method(), req.uri().path()) {
		// agent-card.json: v0.3.0+
		// agent.json: older versions
		(m, path)
			if m == http::Method::GET
				&& (path.ends_with("/.well-known/agent.json")
					|| path.ends_with("/.well-known/agent-card.json")) =>
		{
			// In case of rewrite, use the original so we know where to send them back to
			let uri = req
				.extensions()
				.get::<filters::OriginalUrl>()
				.map(|u| u.0.clone())
				.unwrap_or_else(|| req.uri().clone());
			let uri = crate::http::x_headers::apply_forwarded_scheme(uri, req.headers());
			// Also record the (possibly rewritten) backend path so we can compute
			// relative interface URLs on the response side.
			let backend_path = req.uri().path().to_string();
			let rewrite = req
				.extensions()
				.get::<filters::AppliedUrlRewrite>()
				.cloned();
			RequestType::AgentCard(uri, backend_path, rewrite)
		},
		(m, _) if m == http::Method::POST => {
			let method = match crate::http::classify_content_type(req.headers()) {
				crate::http::WellKnownContentTypes::Json => match inspect_method(req).await {
					Ok(method) => method,
					Err(e) => {
						warn!("failed to read a2a request: {e}");
						Strng::from("unknown")
					},
				},
				_ => {
					warn!("unknown content type from A2A");
					Strng::from("unknown")
				},
			};
			RequestType::Call(method)
		},
		_ => RequestType::Unknown,
	}
}

#[derive(Debug, Clone, Default)]
pub enum RequestType {
	#[default]
	Unknown,
	AgentCard(http::Uri, String, Option<filters::AppliedUrlRewrite>),
	Call(Strng),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseInfo {
	pub outcome: ResponseOutcome,
	pub error_code: Option<i64>,
	pub result_kind: Option<Strng>,
	pub task_state: Option<Strng>,
	pub context_id: Option<Strng>,
}

impl ResponseInfo {
	fn from_json(value: &Value) -> Self {
		let error = value.get("error").filter(|e| !e.is_null());
		let result = value.get("result").filter(|r| !r.is_null());
		let outcome = if error.is_some() {
			ResponseOutcome::Error
		} else if result.is_some() {
			ResponseOutcome::Success
		} else {
			ResponseOutcome::Unknown
		};
		let error_code = error
			.and_then(|e| e.get("code"))
			.and_then(serde_json::Value::as_i64);
		// A2A v0.3 puts the Task/Message fields directly on `result`, discriminated by
		// `kind`. A2A v1.0 models the response as `oneof payload { Task task = 1;
		// Message message = 2; }`, so the payload is nested under `result.task` or
		// `result.message` and carries no `kind` field. Prefer the flat v0.3 shape and
		// fall back to the nested v1.0 shape so both spec generations populate.
		let payload = |name: &str| result.and_then(|r| r.get(name)).filter(|v| !v.is_null());
		let task = payload("task");
		let message = payload("message");
		let result_kind = result
			.and_then(|r| r.get("kind"))
			.and_then(serde_json::Value::as_str)
			.map(Strng::from)
			// v1.0: the populated `oneof` arm names the kind.
			.or_else(|| task.map(|_| Strng::from("task")))
			.or_else(|| message.map(|_| Strng::from("message")));
		let task_state = result
			.and_then(|r| r.get("status"))
			.and_then(|status| status.get("state"))
			.and_then(serde_json::Value::as_str)
			.or_else(|| {
				// v1.0: result.task.status.state. A Message has no status.
				task
					.and_then(|t| t.get("status"))
					.and_then(|status| status.get("state"))
					.and_then(serde_json::Value::as_str)
			})
			.map(Strng::from);
		// context_id ties multiple turns of a conversation together. Both Task and
		// Message carry it, so check the flat v0.3 location and then either v1.0 arm.
		let context_id = result
			.and_then(|r| r.get("contextId"))
			.or_else(|| task.and_then(|t| t.get("contextId")))
			.or_else(|| message.and_then(|m| m.get("contextId")))
			.and_then(serde_json::Value::as_str)
			.map(Strng::from);
		Self {
			outcome,
			error_code,
			result_kind,
			task_state,
			context_id,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseOutcome {
	Success,
	Error,
	Unknown,
}

impl ResponseOutcome {
	pub fn as_str(self) -> &'static str {
		match self {
			ResponseOutcome::Success => "success",
			ResponseOutcome::Error => "error",
			ResponseOutcome::Unknown => "unknown",
		}
	}
}

pub async fn apply_to_response(
	pol: Option<&A2aPolicy>,
	a2a_type: RequestType,
	resp: &mut Response,
) -> anyhow::Result<Option<ResponseInfo>> {
	if pol.is_none() {
		return Ok(None);
	};
	match a2a_type {
		RequestType::AgentCard(uri, backend_path, rewrite) => {
			// For agent card, we need to mutate the request to insert the proper URL to reach it
			// through the gateway.
			let buffer_limit = crate::http::response_buffer_limit(resp);
			let body = std::mem::replace(resp.body_mut(), Body::empty());
			let Ok(mut agent_card) = json::from_body_with_limit::<Value>(body, buffer_limit).await else {
				anyhow::bail!("agent card invalid JSON");
			};
			let gateway_base = build_agent_path(uri);

			// Compute the backend agent base by stripping the agent-card suffix from the
			// (possibly rewritten) backend request path. This lets us compute the *relative*
			// part of interface URLs so they are anchored at the gateway path instead of
			// being naively appended.
			let backend_agent_path = strip_agent_card_suffix(&backend_path);

			if let Some(interfaces) = agent_card.get_mut("supportedInterfaces") {
				// A2A v1.0: rewrite url inside each AgentInterface entry.
				let arr = interfaces
					.as_array_mut()
					.ok_or_else(|| anyhow::anyhow!("agent card supportedInterfaces is not an array"))?;
				for iface in arr.iter_mut() {
					if let Some(url_val) = iface.get_mut("url")
						&& let Some(s) = url_val.as_str()
						&& let Ok(iface_uri) = s.parse::<Uri>()
					{
						let iface_path = iface_uri
							.path_and_query()
							.map(|pq| pq.as_str())
							.unwrap_or_else(|| iface_uri.path());
						// Strip the backend agent base from the interface path so the
						// result is relative to the agent card location. Then anchor
						// that relative path at the gateway base.
						// Only match complete path segments to avoid partial matches
						// (e.g., /internal/weather should not match /internal/weather-v2).
						let url = public_interface_url(
							&gateway_base,
							iface_path,
							backend_agent_path,
							rewrite.as_ref(),
						);
						*url_val = Value::String(url);
					}
				}
			} else if let Some(url_field) = json::traverse_mut(&mut agent_card, &["url"]) {
				// A2A v0.3: rewrite the single top-level url.
				*url_field = Value::String(gateway_base);
			} else {
				anyhow::bail!("agent card missing URL (no 'url' or 'supportedInterfaces' field)");
			}

			resp.headers_mut().remove(header::CONTENT_LENGTH);
			*resp.body_mut() = json::to_body(agent_card)?;
			Ok(None)
		},
		RequestType::Call(_) => Ok(inspect_call_response(resp).await),
		RequestType::Unknown => Ok(None),
	}
}

async fn inspect_call_response(resp: &mut Response) -> Option<ResponseInfo> {
	if !matches!(
		crate::http::classify_content_type(resp.headers()),
		crate::http::WellKnownContentTypes::Json
	) {
		return None;
	}

	let body = match crate::http::inspect_response_body(resp).await {
		Ok(BodyInspection::Complete(body)) => body,
		Ok(BodyInspection::Partial(_)) => return None,
		Err(err) => {
			debug!("failed to inspect a2a response: {err}");
			return None;
		},
	};
	match serde_json::from_slice::<Value>(&body) {
		Ok(value) => Some(ResponseInfo::from_json(&value)),
		Err(err) => {
			debug!("failed to parse a2a response JSON: {err}");
			None
		},
	}
}

#[derive(Deserialize)]
struct JsonRpcMethod {
	method: Strng,
}

async fn inspect_method(req: &mut Request<Body>) -> anyhow::Result<Strng> {
	Ok(json::inspect_body::<JsonRpcMethod>(req).await?.method)
}

fn build_agent_path(uri: Uri) -> String {
	// Keep the original URL the found the agent at, but strip the agent card suffix.
	// Note: this won't work in the case they are hosting their agent in other locations.
	let path = strip_agent_card_suffix(uri.path());
	uri.to_string().replace(uri.path(), path)
}

/// Strip the agent-card well-known suffix from a path, returning the base path
/// where the agent is hosted.
fn strip_agent_card_suffix(path: &str) -> &str {
	let path = path.strip_suffix("/.well-known/agent.json").unwrap_or(path);
	path
		.strip_suffix("/.well-known/agent-card.json")
		.unwrap_or(path)
}

fn public_interface_url(
	gateway_base: &str,
	iface_path: &str,
	backend_agent_path: &str,
	rewrite: Option<&filters::AppliedUrlRewrite>,
) -> String {
	if let Some(path) = rewrite.and_then(|rewrite| {
		let crate::types::agent::PathRedirect::Prefix(replacement) = rewrite.path.as_ref()? else {
			return None;
		};
		let crate::types::agent::PathMatch::PathPrefix(matched) = &rewrite.path_match else {
			return None;
		};
		let rest = strip_complete_path_prefix(iface_path, replacement)?;
		Some(join_path_prefix(matched, rest))
	}) {
		return replace_path(gateway_base, &path);
	}

	let relative = strip_complete_path_prefix(iface_path, backend_agent_path).unwrap_or(iface_path);
	format!("{gateway_base}{relative}")
}

fn replace_path(uri: &str, path: &str) -> String {
	let Ok(uri) = uri.parse::<Uri>() else {
		return format!("{uri}{path}");
	};
	let original = uri.to_string();
	let query = uri.query().map(str::to_string);
	let mut path_and_query = path.to_string();
	if let Some(query) = query {
		path_and_query.push('?');
		path_and_query.push_str(&query);
	}
	let Ok(path_and_query) = path_and_query.parse() else {
		return original;
	};
	let mut parts = uri.into_parts();
	parts.path_and_query = Some(path_and_query);
	Uri::from_parts(parts).map_or(original, |uri| uri.to_string())
}

/// Strip `prefix` only when it ends on a path-segment boundary.
fn strip_complete_path_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
	if prefix.is_empty() {
		return None;
	}
	let prefix = prefix.trim_end_matches('/');
	if prefix.is_empty() {
		return Some(path);
	}
	if path == prefix {
		return Some("");
	}
	let stripped = path.strip_prefix(prefix)?;
	stripped.starts_with('/').then_some(stripped)
}

fn join_path_prefix(prefix: &str, rest: &str) -> String {
	let prefix = prefix.trim_end_matches('/');
	let rest = rest.trim_start_matches('/');
	match (prefix, rest) {
		("", "") => "/".to_string(),
		("", rest) => format!("/{rest}"),
		(prefix, "") => prefix.to_string(),
		(prefix, rest) => format!("{prefix}/{rest}"),
	}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
