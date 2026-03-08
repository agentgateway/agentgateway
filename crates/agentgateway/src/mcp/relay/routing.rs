//! Target routing and routed identifier handling for multiplexed MCP sessions.
//!
//! `TargetRouter` owns target-aware name and URI rewriting. `TargetIds` owns
//! the session-bound codec for downstream-visible routed identifiers.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rmcp::model::{ProgressToken, RequestId};
use serde::{Deserialize, Serialize};

use crate::http::sessionpersistence::Encoder;
use crate::mcp::upstream::UpstreamError;
use crate::mcp::{ClientError, local_session_binding, session_binding_tag};

// Double underscore namespacing (SEP-993) avoids collisions with tool names that include "_".
// Reference: modelcontextprotocol/modelcontextprotocol#94.
pub(crate) const TARGET_NAME_DELIMITER: &str = "__";
pub(crate) const AGW_SCHEME: &str = "agw";
pub(crate) const AGW_URI_QUERY_PARAM: &str = "u";

#[derive(Debug, Clone)]
pub(crate) struct TargetRouter {
	default_target_name: Option<String>,
}

impl TargetRouter {
	pub(crate) fn new(default_target_name: Option<String>) -> Self {
		Self {
			default_target_name,
		}
	}

	pub(crate) fn default_target_name(&self) -> Option<&str> {
		self.default_target_name.as_deref()
	}

	pub(crate) fn default_target_name_owned(&self) -> Option<String> {
		self.default_target_name.clone()
	}

	pub(crate) fn uses_target_routing(&self) -> bool {
		self.default_target_name.is_none()
	}

	pub(crate) fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		if let Some(default) = self.default_target_name() {
			Ok((default, res))
		} else {
			res
				.split_once(TARGET_NAME_DELIMITER)
				.ok_or(UpstreamError::InvalidRequest(
					"invalid resource name".to_string(),
				))
		}
	}

	pub(crate) fn unwrap_resource_uri(&self, uri: &str) -> Option<(String, String)> {
		if let Some(default) = self.default_target_name() {
			return Some((default.to_string(), uri.to_string()));
		}
		let parsed = url::Url::parse(uri).ok()?;
		if parsed.scheme() != AGW_SCHEME {
			return None;
		}
		let target = decode_target_from_uri_host(parsed.host_str()?)?;
		parsed
			.query_pairs()
			.find(|(k, _)| k == AGW_URI_QUERY_PARAM)
			.map(|(_, v)| (target, v.into_owned()))
	}

	pub(crate) fn resource_name<'a>(&self, target: &str, name: Cow<'a, str>) -> Cow<'a, str> {
		if self.default_target_name.is_some() {
			return name;
		}
		Cow::Owned(format!("{target}{TARGET_NAME_DELIMITER}{name}"))
	}

	pub(crate) fn prefix_task_id(&self, target: &str, task_id: &mut String) {
		let old_id = std::mem::take(task_id);
		*task_id = self.resource_name(target, Cow::Owned(old_id)).into_owned();
	}

	pub(crate) fn wrap_resource_uri<'a>(&self, target: &str, uri: &'a str) -> Cow<'a, str> {
		if self.default_target_name.is_some() {
			return Cow::Borrowed(uri);
		}

		let mut encoded = String::with_capacity(uri.len() + target.len() + 32);
		encoded.push_str(AGW_SCHEME);
		encoded.push_str("://");
		encoded.push_str(&encode_target_for_uri_host(target));
		encoded.push_str("/?");
		encoded.push_str(AGW_URI_QUERY_PARAM);
		encoded.push('=');

		let bytes = uri.as_bytes();
		let mut start = 0;

		for (i, &b) in bytes.iter().enumerate() {
			if b == b'{' || b == b'}' {
				if i > start {
					for s in url::form_urlencoded::byte_serialize(&bytes[start..i]) {
						encoded.push_str(s);
					}
				}
				encoded.push(b as char);
				start = i + 1;
			}
		}
		if start < bytes.len() {
			for s in url::form_urlencoded::byte_serialize(&bytes[start..]) {
				encoded.push_str(s);
			}
		}

		Cow::Owned(encoded)
	}
}

fn encode_target_for_uri_host(target: &str) -> String {
	utf8_percent_encode(target, NON_ALPHANUMERIC).to_string()
}

fn decode_target_from_uri_host(target: &str) -> Option<String> {
	percent_decode_str(target)
		.decode_utf8()
		.ok()
		.map(|decoded| decoded.into_owned())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum RoutedRequestIdKind {
	#[serde(rename = "n")]
	Number,
	#[serde(rename = "s")]
	String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutedRequestId {
	#[serde(rename = "t")]
	kind: String,
	#[serde(rename = "v")]
	version: u8,
	#[serde(rename = "b")]
	session_binding: String,
	#[serde(rename = "n")]
	target: String,
	#[serde(rename = "k")]
	original_id_kind: RoutedRequestIdKind,
	#[serde(rename = "i")]
	original_id: String,
}

impl RoutedRequestId {
	const KIND: &str = "agw-request";
	const VERSION: u8 = 1;

	fn new(session_binding: &str, target: &str, original_id: &RequestId) -> Self {
		let (original_id_kind, original_id) = match original_id {
			RequestId::Number(value) => (RoutedRequestIdKind::Number, value.to_string()),
			RequestId::String(value) => (RoutedRequestIdKind::String, value.to_string()),
		};
		Self {
			kind: Self::KIND.to_string(),
			version: Self::VERSION,
			session_binding: session_binding.to_string(),
			target: target.to_string(),
			original_id_kind,
			original_id,
		}
	}

	fn into_parts(self) -> Result<(String, RequestId), UpstreamError> {
		if self.kind != Self::KIND {
			return Err(UpstreamError::InvalidRequest(
				"unknown routed request id kind".to_string(),
			));
		}
		if self.version != Self::VERSION {
			return Err(UpstreamError::InvalidRequest(
				"unsupported routed request id version".to_string(),
			));
		}
		let original_id = match self.original_id_kind {
			RoutedRequestIdKind::Number => {
				RequestId::Number(self.original_id.parse::<i64>().map_err(|_| {
					UpstreamError::InvalidRequest("routed request id number parse failed".to_string())
				})?)
			},
			RoutedRequestIdKind::String => RequestId::String(self.original_id.into()),
		};
		Ok((self.target, original_id))
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ElicitationRouteId {
	#[serde(rename = "t")]
	kind: String,
	#[serde(rename = "v")]
	version: u8,
	#[serde(rename = "b")]
	session_binding: String,
	#[serde(rename = "n")]
	target: String,
	#[serde(rename = "i")]
	original_id: String,
}

impl ElicitationRouteId {
	const KIND: &str = "agw-elicitation";
	const VERSION: u8 = 1;

	fn new(session_binding: &str, target: &str, original_id: &str) -> Self {
		Self {
			kind: Self::KIND.to_string(),
			version: Self::VERSION,
			session_binding: session_binding.to_string(),
			target: target.to_string(),
			original_id: original_id.to_string(),
		}
	}
}

#[derive(Debug, Clone)]
pub(crate) struct TargetIds {
	encoder: Encoder,
	session_binding: Arc<RwLock<String>>,
}

impl TargetIds {
	pub(crate) fn new() -> Self {
		Self {
			encoder: Encoder::base64(),
			session_binding: Arc::new(RwLock::new(local_session_binding())),
		}
	}

	pub(crate) fn with_session_binding(mut self, session_handle: &str, encoder: Encoder) -> Self {
		self.encoder = encoder;
		self.set_session_binding(session_handle);
		self
	}

	pub(crate) fn set_session_binding(&self, session_handle: &str) {
		let mut binding = self.session_binding.write().unwrap_or_else(|e| {
			tracing::error!("target id binding lock poisoned while updating; continuing");
			e.into_inner()
		});
		*binding = session_binding_tag(session_handle);
	}

	pub(crate) fn encode_request_id(
		&self,
		target: &str,
		id: &RequestId,
	) -> Result<RequestId, ClientError> {
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("target id binding lock poisoned while encoding request id; continuing");
			e.into_inner()
		});
		let route_id = RoutedRequestId::new(session_binding.as_str(), target, id);
		let plaintext =
			serde_json::to_string(&route_id).expect("serializing routed request id should not fail");
		self
			.encoder
			.encrypt(&plaintext)
			.map(|encoded| RequestId::String(encoded.into()))
			.map_err(|error| {
				ClientError::new(anyhow::anyhow!(
					"failed to encode routed request id for {target}: {error}"
				))
			})
	}

	pub(crate) fn decode_request_id(
		&self,
		id: &RequestId,
	) -> Result<(String, RequestId), UpstreamError> {
		let RequestId::String(raw) = id else {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id must be a string when multiplexing".to_string(),
			));
		};
		let encoded = self
			.encoder
			.decrypt(raw.as_ref())
			.map_err(|_| UpstreamError::InvalidRequest("invalid routed request id".to_string()))?;
		let route_id = serde_json::from_slice::<RoutedRequestId>(&encoded).map_err(|_| {
			UpstreamError::InvalidRequest("invalid routed request id payload".to_string())
		})?;
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("target id binding lock poisoned while decoding request id; continuing");
			e.into_inner()
		});
		if route_id.session_binding != session_binding.as_str() {
			return Err(UpstreamError::InvalidRequest(
				"routed request id does not match this session".to_string(),
			));
		}
		route_id.into_parts()
	}

	pub(crate) fn encode_progress_token(
		&self,
		target: &str,
		token: &ProgressToken,
	) -> Result<ProgressToken, ClientError> {
		self.encode_request_id(target, &token.0).map(ProgressToken)
	}

	pub(crate) fn decode_progress_token(
		&self,
		token: &ProgressToken,
	) -> Result<(String, ProgressToken), UpstreamError> {
		let (target, original_id) = self.decode_request_id(&token.0)?;
		Ok((target, ProgressToken(original_id)))
	}

	pub(crate) fn encode_elicitation_id(
		&self,
		target: &str,
		elicitation_id: &str,
	) -> Result<String, ClientError> {
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("target id binding lock poisoned while encoding elicitation id; continuing");
			e.into_inner()
		});
		let route_id = ElicitationRouteId::new(session_binding.as_str(), target, elicitation_id);
		let plaintext =
			serde_json::to_string(&route_id).expect("serializing elicitation route id should not fail");
		self.encoder.encrypt(&plaintext).map_err(|error| {
			ClientError::new(anyhow::anyhow!(
				"failed to encode elicitation route id for {target}: {error}"
			))
		})
	}

	pub(crate) fn decode_elicitation_id(
		&self,
		elicitation_id: &str,
	) -> Result<(String, String), UpstreamError> {
		let encoded = self
			.encoder
			.decrypt(elicitation_id)
			.map_err(|_| UpstreamError::InvalidRequest("invalid elicitation route id".to_string()))?;
		let route_id = serde_json::from_slice::<ElicitationRouteId>(&encoded).map_err(|_| {
			UpstreamError::InvalidRequest("invalid elicitation route id payload".to_string())
		})?;
		if route_id.kind != ElicitationRouteId::KIND {
			return Err(UpstreamError::InvalidRequest(
				"unknown elicitation route id kind".to_string(),
			));
		}
		if route_id.version != ElicitationRouteId::VERSION {
			return Err(UpstreamError::InvalidRequest(
				"unsupported elicitation route id version".to_string(),
			));
		}
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("target id binding lock poisoned while decoding elicitation id; continuing");
			e.into_inner()
		});
		if route_id.session_binding != session_binding.as_str() {
			return Err(UpstreamError::InvalidRequest(
				"elicitation route id does not match this session".to_string(),
			));
		}
		Ok((route_id.target, route_id.original_id))
	}
}
