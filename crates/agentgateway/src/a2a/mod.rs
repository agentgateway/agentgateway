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
	// Possible options are POST a JSON-RPC or HTTP+JSON A2A message, or GET /.well-known/agent.json
	// For agent card, we will process only on the response
	let method = req.method().clone();
	let path = req.uri().path().to_string();
	match (method, path.as_str()) {
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
			RequestType::AgentCard(uri)
		},
		(http::Method::POST, path) => {
			let method = match crate::http::classify_content_type(req.headers()) {
				crate::http::WellKnownContentTypes::Json => match inspect_method(req, path).await {
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
		// The remaining REST operations are reads and deletes. They carry no body to
		// inspect, so the operation is fully determined by the method and path.
		(m, path) if m == http::Method::GET || m == http::Method::DELETE => {
			match rest_method_from_path(&m, path) {
				Some(method) => RequestType::Call(Strng::from(method)),
				// Not an A2A operation we recognize; leave it unclassified rather than
				// tagging unrelated traffic as A2A.
				None => RequestType::Unknown,
			}
		},
		_ => RequestType::Unknown,
	}
}

#[derive(Debug, Clone, Default)]
pub enum RequestType {
	#[default]
	Unknown,
	AgentCard(http::Uri),
	Call(Strng),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseInfo {
	pub outcome: ResponseOutcome,
	pub error_code: Option<i64>,
	pub result_kind: Option<Strng>,
	pub task_state: Option<Strng>,
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
		let result_kind = result
			.and_then(|r| r.get("kind"))
			.and_then(serde_json::Value::as_str)
			.map(Strng::from);
		let task_state = result
			.and_then(|r| r.get("status"))
			.and_then(|status| status.get("state"))
			.and_then(serde_json::Value::as_str)
			.map(Strng::from);
		Self {
			outcome,
			error_code,
			result_kind,
			task_state,
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
		RequestType::AgentCard(uri) => {
			// For agent card, we need to mutate the request to insert the proper URL to reach it
			// through the gateway.
			let buffer_limit = crate::http::response_buffer_limit(resp);
			let body = std::mem::replace(resp.body_mut(), Body::empty());
			let Ok(mut agent_card) = json::from_body_with_limit::<Value>(body, buffer_limit).await else {
				anyhow::bail!("agent card invalid JSON");
			};
			let gateway_base = build_agent_path(uri);

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
						let path_and_query = iface_uri
							.path_and_query()
							.map(|pq| pq.as_str())
							.unwrap_or_else(|| iface_uri.path());
						*url_val = Value::String(format!("{gateway_base}{path_and_query}"));
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
	method: Option<Strng>,
}

/// Determine the A2A method for a POST request.
///
/// A2A defines multiple wire formats for the same operations:
///   - JSON-RPC: `{"jsonrpc": "2.0", "method": "SendMessage", "params": {...}}`
///   - HTTP+JSON (REST): `{"message": {...}, "configuration": {...}}` POSTed to
///     a method-specific path such as `/message:send` or `/tasks/{id}:cancel`.
///
/// The REST binding has no `method` field in the body — the method is
/// conveyed by the URL path instead — so JSON-RPC's plain `body.method`
/// extraction alone misclassifies every REST call as `unknown`.
async fn inspect_method(req: &mut Request<Body>, path: &str) -> anyhow::Result<Strng> {
	let body = json::inspect_body::<JsonRpcMethod>(req).await?;
	// JSON-RPC carries the method verbatim. This is deliberately passed through
	// unchanged so both the v0.3 (`message/send`) and v1.0 (`SendMessage`) spellings
	// are reported as the client sent them.
	if let Some(method) = body.method {
		return Ok(method);
	}
	if let Some(method) = rest_method_from_path(&http::Method::POST, path) {
		return Ok(Strng::from(method));
	}
	Ok(Strng::from("unknown"))
}

/// Map an A2A HTTP+JSON (REST) request to its canonical A2A method name.
///
/// The REST binding conveys the operation in the request line rather than the
/// body (A2A v1.0 §11.3 "URL Patterns and HTTP Methods"), so the method is
/// derived from the HTTP method plus the trailing path segments:
///
/// | Request                                                  | Method                            |
/// |----------------------------------------------------------|-----------------------------------|
/// | `POST   /message:send`                                     | `SendMessage`                     |
/// | `POST   /message:stream`                                   | `SendStreamingMessage`            |
/// | `GET    /tasks/{id}`                                       | `GetTask`                         |
/// | `GET    /tasks`                                            | `ListTasks`                       |
/// | `POST   /tasks/{id}:cancel`                                | `CancelTask`                      |
/// | `POST   /tasks/{id}:subscribe`                             | `SubscribeToTask`                 |
/// | `POST   /tasks/{id}/pushNotificationConfigs`               | `CreateTaskPushNotificationConfig`|
/// | `GET    /tasks/{id}/pushNotificationConfigs/{configId}`    | `GetTaskPushNotificationConfig`   |
/// | `GET    /tasks/{id}/pushNotificationConfigs`               | `ListTaskPushNotificationConfigs` |
/// | `DELETE /tasks/{id}/pushNotificationConfigs/{configId}`    | `DeleteTaskPushNotificationConfig`|
/// | `GET    /extendedAgentCard`                                | `GetExtendedAgentCard`            |
///
/// The returned names are the canonical method names from the spec's method
/// mapping reference (§5.3), which are shared with the JSON-RPC and gRPC
/// bindings. Reporting those keeps `a2a.method` comparable across bindings
/// rather than inventing a REST-only spelling.
///
/// Matching is done on trailing segments because the gateway may host the agent
/// under an arbitrary prefix (e.g. `/a2a/{agent}/tasks/{id}`), and `{id}` /
/// `{configId}` are opaque and unbounded.
fn rest_method_from_path(method: &http::Method, path: &str) -> Option<&'static str> {
	let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
	// Segment counted from the end; 0 is the last segment.
	let seg = |n: usize| -> Option<&str> {
		segments
			.len()
			.checked_sub(n + 1)
			.and_then(|i| segments.get(i).copied())
	};
	let last = seg(0)?;

	if *method == http::Method::POST {
		// The custom verb binds to the final segment with a `:` suffix. `{id}` is
		// opaque, so match on the suffix rather than the whole segment.
		match last.split_once(':') {
			Some(("message", "send")) => Some("SendMessage"),
			Some(("message", "stream")) => Some("SendStreamingMessage"),
			Some((_, "cancel")) if seg(1) == Some("tasks") => Some("CancelTask"),
			Some((_, "subscribe")) if seg(1) == Some("tasks") => Some("SubscribeToTask"),
			Some(_) => None,
			// Slash-style streaming path, accepted for compatibility with clients that
			// predate the `:stream` custom-verb spelling.
			None if last == "stream" && seg(1) == Some("message") => Some("SendStreamingMessage"),
			None if last == "pushNotificationConfigs" && seg(1).is_some() && seg(2) == Some("tasks") => {
				Some("CreateTaskPushNotificationConfig")
			},
			None => None,
		}
	} else if *method == http::Method::GET {
		if last == "extendedAgentCard" {
			Some("GetExtendedAgentCard")
		} else if last == "pushNotificationConfigs" && seg(1).is_some() && seg(2) == Some("tasks") {
			Some("ListTaskPushNotificationConfigs")
		} else if seg(1) == Some("pushNotificationConfigs") && seg(3) == Some("tasks") {
			Some("GetTaskPushNotificationConfig")
		} else if seg(1) == Some("tasks") {
			Some("GetTask")
		} else if last == "tasks" {
			Some("ListTasks")
		} else {
			None
		}
	} else if *method == http::Method::DELETE
		&& seg(1) == Some("pushNotificationConfigs")
		&& seg(3) == Some("tasks")
	{
		Some("DeleteTaskPushNotificationConfig")
	} else {
		None
	}
}

fn build_agent_path(uri: Uri) -> String {
	// Keep the original URL the found the agent at, but strip the agent card suffix.
	// Note: this won't work in the case they are hosting their agent in other locations.
	let path = uri.path();
	let path = path.strip_suffix("/.well-known/agent.json").unwrap_or(path);
	let path = path
		.strip_suffix("/.well-known/agent-card.json")
		.unwrap_or(path);

	uri.to_string().replace(uri.path(), path)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
