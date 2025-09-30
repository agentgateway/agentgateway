use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::http::{Request, Response};
use crate::*;

pub fn must_traverse<'a, T>(
	value: &'a Value,
	path: &[&str],
	f: impl Fn(&'a Value) -> Option<T>,
) -> anyhow::Result<T> {
	if let Some(res) = traverse(value, path).and_then(f) {
		Ok(res)
	} else {
		Err(anyhow::anyhow!("missing field {}", path.join(".")))
	}
}

pub fn traverse<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
	if path.is_empty() {
		return Some(value);
	}
	path.iter().try_fold(value, |target, token| match target {
		Value::Object(map) => map.get(*token),
		Value::Array(list) => parse_index(token).and_then(|x| list.get(x)),
		_ => None,
	})
}

pub fn traverse_mut<'a>(value: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
	if path.is_empty() {
		return Some(value);
	}
	path.iter().try_fold(value, |target, token| match target {
		Value::Object(map) => map.get_mut(*token),
		Value::Array(list) => parse_index(token).and_then(|x| list.get_mut(x)),
		_ => None,
	})
}

fn parse_index(s: &str) -> Option<usize> {
	if s.starts_with('+') || (s.starts_with('0') && s.len() != 1) {
		return None;
	}
	s.parse().ok()
}

pub async fn from_request_body<T: DeserializeOwned>(req: Request) -> anyhow::Result<T> {
	let lim = http::buffer_limit(&req);
	from_body_with_limit(req.into_body(), lim).await
}

pub async fn from_response_body<T: DeserializeOwned>(resp: Response) -> anyhow::Result<T> {
	let lim = http::response_buffer_limit(&resp);
	from_body_with_limit(resp.into_body(), lim).await
}

pub async fn from_body_with_limit<T: DeserializeOwned>(
	body: http::Body,
	limit: usize,
) -> anyhow::Result<T> {
	let bytes = http::read_body_with_limit(body, limit).await?;
	// Try to parse the response body as JSON
	let t = serde_json::from_slice::<T>(bytes.as_ref())?;
	Ok(t)
}

pub async fn inspect_body<T: DeserializeOwned>(req: &mut http::Request) -> anyhow::Result<T> {
	let buffer = http::buffer_limit(req);
	let body = req.body_mut();
	let orig = std::mem::replace(body, http::Body::empty());
	let bytes = http::read_body_with_limit(orig, buffer).await?;
	// Try to parse the response body as JSON
	let t = serde_json::from_slice::<T>(bytes.as_ref());
	// Regardless of an error or not, we should reset the body back
	*body = http::Body::from(bytes);
	t.map_err(Into::into)
}

pub fn to_body<T: Serialize>(j: T) -> anyhow::Result<http::Body> {
	let bytes = serde_json::to_vec(&j)?;
	Ok(http::Body::from(bytes))
}
