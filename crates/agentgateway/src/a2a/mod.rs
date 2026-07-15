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
