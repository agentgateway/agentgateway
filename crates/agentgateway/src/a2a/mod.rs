use agent_core::strng::Strng;
use http::{Request, Uri, header};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::http::{Body, Response, filters};
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
		(m, "/.well-known/agent.json" | "/.well-known/agent-card.json") if m == http::Method::GET => {
			// In case of rewrite, use the original so we know where to send them back to
			let uri = req
				.extensions()
				.get::<filters::OriginalUrl>()
				.map(|u| u.0.clone())
				.unwrap_or_else(|| req.uri().clone());
			RequestType::AgentCard(uri)
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
	AgentCard(http::Uri),
	Call(Strng),
}

pub async fn apply_to_response(
	pol: Option<&A2aPolicy>,
	a2a_type: RequestType,
	resp: &mut Response,
) -> anyhow::Result<()> {
	if pol.is_none() {
		return Ok(());
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
			let Some(url_field) = json::traverse_mut(&mut agent_card, &["url"]) else {
				anyhow::bail!("agent card missing URL");
			};
			let new_uri = build_agent_path(uri);

			*url_field = Value::String(new_uri);

			resp.headers_mut().remove(header::CONTENT_LENGTH);
			*resp.body_mut() = json::to_body(agent_card)?;
			Ok(())
		},
		RequestType::Call(_) => {
			// We don't currently inspect A2A responses.
			Ok(())
		},
		RequestType::Unknown => Ok(()),
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
