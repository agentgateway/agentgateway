//! Per-request MCP protocol model.
//!
//! In the 2026-07-28 RC (SEP-2575), every request is self-describing: the
//! protocol version and client metadata travel on the request, not in a
//! negotiated session. This module owns the gateway's version parser and the
//! per-request context the downstream-support checks read.
//!
//! The parser stays gateway-local because rmcp caps its known versions at
//! `2025-11-25` and exposes no `Mcp-Method`/`Mcp-Name` constants, so it cannot
//! represent the RC fields. rmcp's `ProtocolVersion` is a newtype that accepts
//! any string, so the gateway parses versions from `&str` and compares against
//! the body rmcp deserialized via `.as_str()`.

use std::str::FromStr;

use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, ErrorCode, GetMeta,
	RequestId,
};

use crate::mcp::Error;

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

/// SEP-2243 standard HTTP headers. `Mcp-Method` carries the JSON-RPC method.
/// `Mcp-Name` carries the resolved tool/prompt/resource name. Intermediaries can
/// route without parsing the JSON-RPC body. Modern-only (`2026-07-28`+).
pub const HEADER_MCP_METHOD: &str = "Mcp-Method";
pub const HEADER_MCP_NAME: &str = "Mcp-Name";

/// SEP-2575 per-request protocol version carried in `_meta`. The RC draft
/// defines this wire key. Recheck the final SEP text before `2026-07-28` is
/// advertised as supported.
pub const META_PROTOCOL_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

/// JSON-RPC error code for an unsupported/unrecognized protocol version.
/// Final SEP-2575 value from the MCP reserved protocol-error range.
pub const UNSUPPORTED_PROTOCOL_VERSION: ErrorCode = ErrorCode(-32022);
/// JSON-RPC error code for a standard header that disagrees with the body.
/// Final SEP-2243 value from the MCP reserved protocol-error range.
pub const HEADER_MISMATCH: ErrorCode = ErrorCode(-32020);

/// MCP protocol versions the gateway recognizes on the wire.
///
/// "Legacy" versions (≤ `2025-11-25`) use session-based transport; "modern"
/// (`2026-07-28`+, SEP-2567) is stateless, with no protocol-level session or GET
/// stream. `KNOWN_VERSIONS` lists what the parser accepts;
/// `SUPPORTED_DOWNSTREAM_VERSIONS` lists what the gateway accepts from downstream
/// clients. The gateway parses `2026-07-28` but does not advertise it until
/// stateless transport is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
// Variant names mirror rmcp's `ProtocolVersion::V_YYYY_MM_DD` associated consts so the
// two line up by name. rmcp models versions as a newtype over `Cow<str>`, not an enum, so
// converging later means rewriting these `match` sites — not a drop-in re-export swap.
#[allow(non_camel_case_types)]
pub enum ProtocolVersion {
	V_2024_11_05,
	V_2025_03_26,
	V_2025_06_18,
	V_2025_11_25,
	/// 2026-07-28 RC. Recognized/parsed; not advertised downstream yet.
	V_2026_07_28,
}

impl ProtocolVersion {
	/// Every version the gateway can parse, including the not-yet-supported RC.
	pub const KNOWN_VERSIONS: &'static [Self] = &[
		Self::V_2024_11_05,
		Self::V_2025_03_26,
		Self::V_2025_06_18,
		Self::V_2025_11_25,
		Self::V_2026_07_28,
	];

	/// Versions the gateway advertises/accepts downstream. Today that is the
	/// legacy set. `2026-07-28` stays out until stateless transport,
	/// `server/discover`, and standard headers are implemented.
	// Keep this in sync with the downstream gate until modern stateless support is implemented.
	pub const SUPPORTED_DOWNSTREAM_VERSIONS: &'static [Self] = &[
		Self::V_2024_11_05,
		Self::V_2025_03_26,
		Self::V_2025_06_18,
		Self::V_2025_11_25,
	];

	pub fn as_str(&self) -> &'static str {
		match self {
			Self::V_2024_11_05 => "2024-11-05",
			Self::V_2025_03_26 => "2025-03-26",
			Self::V_2025_06_18 => "2025-06-18",
			Self::V_2025_11_25 => "2025-11-25",
			Self::V_2026_07_28 => "2026-07-28",
		}
	}

	/// Match the version rmcp deserialized from the initialize body. rmcp's own
	/// table stops at `2025-11-25`, but it round-trips the raw string, so we match
	/// on `.as_str()` rather than depend on rmcp's `KNOWN_VERSIONS`.
	pub fn from_rmcp(v: &rmcp::model::ProtocolVersion) -> Option<Self> {
		Self::parse(v.as_str())
	}

	pub fn parse(s: &str) -> Option<Self> {
		Self::KNOWN_VERSIONS
			.iter()
			.find(|v| v.as_str() == s)
			.copied()
	}

	/// Whether this version uses modern (SEP-2567 stateless) transport.
	pub fn is_modern(&self) -> bool {
		*self >= Self::V_2026_07_28
	}

	/// Whether this version is supported for downstream requests.
	pub fn is_supported_downstream(&self) -> bool {
		Self::SUPPORTED_DOWNSTREAM_VERSIONS.contains(self)
	}
}

impl FromStr for ProtocolVersion {
	type Err = ();
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::parse(s).ok_or(())
	}
}

/// Protocol context extracted once per request, before any gateway rewrite.
///
/// SEP-2575 requires each request to be self-describing, so this is the single
/// place the request version is resolved rather than reconstructing it from a
/// session.
///
/// `version` is `None` for legacy subsequent requests that carry no
/// `MCP-Protocol-Version` header; their version lives in the session, not the
/// request.
#[derive(Debug, Clone, Default)]
pub struct RequestProtocolContext {
	pub version: Option<ProtocolVersion>,
	pub request_id: Option<RequestId>,
}

impl RequestProtocolContext {
	/// Extract and validate the protocol context from request headers + body.
	///
	/// The version may arrive in the initialize body, the `MCP-Protocol-Version`
	/// header, and `_meta`; all present sources must agree, and any disagreement is a
	/// symmetric `VersionMismatch`. The `Mcp-Method`/`Mcp-Name` routing headers, when
	/// present, are validated against the body and reported as wrong on conflict.
	pub fn extract(
		headers: &::http::HeaderMap,
		message: &ClientJsonRpcMessage,
	) -> Result<Self, Error> {
		let request_id = get_id_from_request(message);

		let header_version = parse_version_header(headers, request_id.clone())?;
		let meta_version = parse_meta_version(message_meta(message), request_id.clone())?;

		// Initialize carries the negotiated version in its params; it is
		// authoritative for its own request.
		let init_version = match initialize_params(message) {
			Some(p) => {
				let v = ProtocolVersion::from_rmcp(&p.protocol_version).ok_or_else(|| {
					Error::UnsupportedVersion(request_id.clone(), p.protocol_version.as_str().to_string())
				})?;
				Some(v)
			},
			None => None,
		};

		let mut version = None;
		for v in [init_version, header_version, meta_version]
			.into_iter()
			.flatten()
		{
			match version {
				Some(prev) if prev != v => return Err(Error::VersionMismatch(request_id.clone())),
				None => version = Some(v),
				Some(_) => {},
			}
		}

		// SEP-2243 standard headers, when present, must agree with the body: `Mcp-Method`
		// with the request method, `Mcp-Name` with the resolved tool/prompt/resource name.
		// Presence-gated, not version-gated: an intermediary routing on a header that
		// disagrees with the executed body is a confused-deputy gap regardless of the
		// claimed version, so a legacy version must not bypass the check. An undecodable
		// value fails closed (never treated as absent). Conforming legacy clients omit
		// these headers, so valid legacy traffic is never rejected.
		let mcp_method = routing_header(headers, HEADER_MCP_METHOD, &request_id)?;
		let mcp_name = routing_header(headers, HEADER_MCP_NAME, &request_id)?;
		if let Some(method) = mcp_method
			&& let Some(body_method) = message_method(message)
			&& method != body_method
		{
			return Err(Error::HeaderBodyMismatch(
				request_id.clone(),
				HEADER_MCP_METHOD,
			));
		}
		if let Some(name) = mcp_name
			&& let Some(body_name) = request_resource_name(message)
			&& name != body_name.as_str()
		{
			return Err(Error::HeaderBodyMismatch(
				request_id.clone(),
				HEADER_MCP_NAME,
			));
		}

		Ok(Self {
			version,
			request_id,
		})
	}

	/// Reject a request whose negotiated version the gateway does not accept
	/// downstream (modern is parsed but not advertised yet). `Ok` when the version
	/// is absent (legacy subsequent request) or supported.
	pub(crate) fn reject_if_unsupported_downstream(&self) -> Result<(), Error> {
		if let Some(version) = self.version
			&& !version.is_supported_downstream()
		{
			return Err(Error::UnsupportedVersion(
				self.request_id.clone(),
				version.as_str().to_string(),
			));
		}
		Ok(())
	}
}

fn message_method(message: &ClientJsonRpcMessage) -> Option<&str> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.request.method()),
		ClientJsonRpcMessage::Notification(notification) => {
			Some(notification_method(&notification.notification))
		},
		_ => None,
	}
}

fn notification_method(notification: &ClientNotification) -> &str {
	match notification {
		ClientNotification::CancelledNotification(n) => n.method.as_str(),
		ClientNotification::ProgressNotification(n) => n.method.as_str(),
		ClientNotification::InitializedNotification(n) => n.method.as_str(),
		ClientNotification::RootsListChangedNotification(n) => n.method.as_str(),
		ClientNotification::CustomNotification(n) => n.method.as_str(),
	}
}

/// Gateway-facing resource name for the SEP-2243 `Mcp-Name` standard header, where the
/// request targets a named resource (tool/prompt/resource).
fn request_resource_name(message: &ClientJsonRpcMessage) -> Option<String> {
	let ClientJsonRpcMessage::Request(req) = message else {
		return None;
	};
	match &req.request {
		ClientRequest::CallToolRequest(r) => Some(r.params.name.to_string()),
		ClientRequest::GetPromptRequest(r) => Some(r.params.name.to_string()),
		ClientRequest::ReadResourceRequest(r) => Some(r.params.uri.to_string()),
		_ => None,
	}
}

fn initialize_params(
	message: &ClientJsonRpcMessage,
) -> Option<&rmcp::model::InitializeRequestParams> {
	match message {
		ClientJsonRpcMessage::Request(req) => match &req.request {
			ClientRequest::InitializeRequest(init) => Some(&init.params),
			_ => None,
		},
		_ => None,
	}
}

fn get_id_from_request(message: &ClientJsonRpcMessage) -> Option<RequestId> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.id.clone()),
		_ => None,
	}
}

fn message_meta(message: &ClientJsonRpcMessage) -> Option<&rmcp::model::Meta> {
	match message {
		ClientJsonRpcMessage::Request(req) => Some(req.request.get_meta()),
		ClientJsonRpcMessage::Notification(notification) => Some(notification.notification.get_meta()),
		_ => None,
	}
}

/// Validate the `MCP-Protocol-Version` header alone, for bodyless requests
/// (GET/DELETE). An unknown version is rejected; the error carries no request id,
/// so the response is HTTP-status-only (no JSON-RPC body to recover an id for).
pub fn validate_version_header(
	headers: &::http::HeaderMap,
) -> Result<Option<ProtocolVersion>, Error> {
	parse_version_header(headers, None)
}

/// Reject a bodyless modern request on a legacy-only session operation (GET/DELETE).
/// SEP-2567 removed protocol-level sessions and the GET stream for modern; the
/// value-less version header is all there is to gate on. `Ok` when the version is
/// absent or legacy.
pub(crate) fn reject_modern_session_request(headers: &::http::HeaderMap) -> Result<(), Error> {
	if let Some(version) = validate_version_header(headers)?
		&& version.is_modern()
	{
		return Err(Error::UnsupportedVersion(
			None,
			version.as_str().to_string(),
		));
	}
	Ok(())
}

/// Decode a SEP-2243 routing header. `None` if absent. `Err` if present but
/// undecodable. A malformed routing header fails closed instead of disappearing
/// into `None`. Validated whenever present, regardless of negotiated version.
fn routing_header<'a>(
	headers: &'a ::http::HeaderMap,
	name: &str,
	id: &Option<RequestId>,
) -> Result<Option<&'a str>, Error> {
	match headers.get(name) {
		None => Ok(None),
		Some(v) => v
			.to_str()
			.map(Some)
			.map_err(|_| Error::InvalidRoutingHeader(id.clone(), name.to_string())),
	}
}

fn parse_version_header(
	headers: &::http::HeaderMap,
	id: Option<RequestId>,
) -> Result<Option<ProtocolVersion>, Error> {
	let Some(value) = headers.get(rmcp::transport::common::http_header::HEADER_MCP_PROTOCOL_VERSION)
	else {
		return Ok(None);
	};
	let value = value
		.to_str()
		.map_err(|_| Error::UnsupportedVersion(id.clone(), "<non-utf8>".to_string()))?;
	ProtocolVersion::parse(value)
		.map(Some)
		.ok_or_else(|| Error::UnsupportedVersion(id, value.to_string()))
}

fn parse_meta_version(
	meta: Option<&rmcp::model::Meta>,
	id: Option<RequestId>,
) -> Result<Option<ProtocolVersion>, Error> {
	let Some(value) = meta.and_then(|m| m.0.get(META_PROTOCOL_VERSION_KEY)) else {
		return Ok(None);
	};
	let Some(s) = value.as_str() else {
		return Err(Error::UnsupportedVersion(id, value.to_string()));
	};
	ProtocolVersion::parse(s)
		.map(Some)
		.ok_or_else(|| Error::UnsupportedVersion(id, s.to_string()))
}
