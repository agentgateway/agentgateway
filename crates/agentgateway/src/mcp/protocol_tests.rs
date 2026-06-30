//! Unit tests for the per-request protocol model.
//!
//! These tests feed raw JSON shapes into the gateway parser. rmcp caps its known
//! versions at `2025-11-25`, so real `2026-07-28` interop waits for modern
//! stateless transport support.

use rmcp::model::{ClientJsonRpcMessage, RequestId};
use serde_json::{Value, json};

use super::*;
use crate::mcp::Error;

fn headers(pairs: &[(&str, &str)]) -> ::http::HeaderMap {
	let mut h = ::http::HeaderMap::new();
	for (k, v) in pairs {
		h.insert(
			::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
			::http::HeaderValue::from_str(v).unwrap(),
		);
	}
	h
}

fn message(body: Value) -> ClientJsonRpcMessage {
	serde_json::from_value(body).expect("valid client message")
}

fn initialize_body(version: &str) -> Value {
	json!({
		"jsonrpc": "2.0",
		"id": 1,
		"method": "initialize",
		"params": {
			"protocolVersion": version,
			"capabilities": {},
			"clientInfo": {"name": "test client", "version": "0.0.1"}
		}
	})
}

fn tools_call_body(name: &str) -> Value {
	json!({
		"jsonrpc": "2.0",
		"id": 7,
		"method": "tools/call",
		"params": {"name": name, "arguments": {}}
	})
}

#[test]
fn parse_round_trips_every_known_version() {
	for v in ProtocolVersion::KNOWN_VERSIONS {
		assert_eq!(ProtocolVersion::parse(v.as_str()), Some(*v));
		assert_eq!(v.as_str().parse::<ProtocolVersion>(), Ok(*v));
	}
	assert_eq!(ProtocolVersion::parse("1900-01-01"), None);
	assert!("1900-01-01".parse::<ProtocolVersion>().is_err());
}

#[test]
fn rc_version_is_known_but_not_advertised_downstream() {
	// The gateway parses the RC now, but it must not advertise modern support
	// until stateless transport is implemented.
	assert!(ProtocolVersion::KNOWN_VERSIONS.contains(&ProtocolVersion::V_2026_07_28));
	assert!(!ProtocolVersion::SUPPORTED_DOWNSTREAM_VERSIONS.contains(&ProtocolVersion::V_2026_07_28));
	assert!(!ProtocolVersion::V_2026_07_28.is_supported_downstream());
	for v in ProtocolVersion::SUPPORTED_DOWNSTREAM_VERSIONS {
		assert!(v.is_supported_downstream());
		assert!(!v.is_modern(), "{} must be legacy", v.as_str());
	}
}

#[test]
fn modern_versions_do_not_use_sessions() {
	assert!(ProtocolVersion::V_2026_07_28.is_modern());
	assert!(!ProtocolVersion::V_2025_11_25.is_modern());
}

#[test]
fn reject_if_unsupported_downstream_gates_modern_only() {
	// Parsed-but-not-advertised modern is rejected (carrying the request id);
	// legacy and version-less contexts pass.
	let legacy = RequestProtocolContext {
		version: Some(ProtocolVersion::V_2025_06_18),
		request_id: Some(RequestId::Number(7)),
	};
	assert!(legacy.reject_if_unsupported_downstream().is_ok());
	assert!(
		RequestProtocolContext::default()
			.reject_if_unsupported_downstream()
			.is_ok()
	);

	let modern = RequestProtocolContext {
		version: Some(ProtocolVersion::V_2026_07_28),
		request_id: Some(RequestId::Number(7)),
	};
	assert!(matches!(
		modern.reject_if_unsupported_downstream(),
		Err(Error::UnsupportedVersion(Some(RequestId::Number(7)), v)) if v == "2026-07-28"
	));
}

#[test]
fn reject_modern_session_request_gates_modern_only() {
	// Bodyless GET/DELETE gate: absent and legacy pass; modern rejected with no id.
	assert!(reject_modern_session_request(&headers(&[])).is_ok());
	assert!(
		reject_modern_session_request(&headers(&[("mcp-protocol-version", "2025-06-18")])).is_ok()
	);
	assert!(matches!(
		reject_modern_session_request(&headers(&[("mcp-protocol-version", "2026-07-28")])),
		Err(Error::UnsupportedVersion(None, v)) if v == "2026-07-28"
	));
}

#[test]
fn extract_reads_legacy_header_version() {
	let msg = message(tools_call_body("echo"));
	let ctx =
		RequestProtocolContext::extract(&headers(&[("mcp-protocol-version", "2025-06-18")]), &msg)
			.unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2025_06_18));
	assert!(!ctx.version.unwrap().is_modern());
}

#[test]
fn extract_treats_initialize_body_as_authoritative() {
	let msg = message(initialize_body("2025-06-18"));
	let ctx = RequestProtocolContext::extract(&::http::HeaderMap::new(), &msg).unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2025_06_18));
}

#[test]
fn extract_rejects_unsupported_header_version_with_recovered_id() {
	let msg = message(tools_call_body("echo"));
	let err =
		RequestProtocolContext::extract(&headers(&[("mcp-protocol-version", "1900-01-01")]), &msg)
			.unwrap_err();
	match err {
		Error::UnsupportedVersion(Some(RequestId::Number(7)), v) => assert_eq!(v, "1900-01-01"),
		other => panic!("expected UnsupportedVersion carrying the request id, got {other:?}"),
	}
}

#[test]
fn extract_rejects_header_body_version_disagreement() {
	// initialize body says 2025-06-18, header says 2025-11-25.
	let msg = message(initialize_body("2025-06-18"));
	let err =
		RequestProtocolContext::extract(&headers(&[("mcp-protocol-version", "2025-11-25")]), &msg)
			.unwrap_err();
	assert!(matches!(
		err,
		Error::VersionMismatch(Some(RequestId::Number(1)))
	));
}

#[test]
fn extract_reads_per_request_meta_version() {
	// SEP-2575 carries the version in `_meta` for non-initialize requests.
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let ctx = RequestProtocolContext::extract(&::http::HeaderMap::new(), &msg).unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2026_07_28));
	assert!(ctx.version.unwrap().is_modern());
}

#[test]
fn extract_rejects_header_meta_version_disagreement() {
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let err =
		RequestProtocolContext::extract(&headers(&[("mcp-protocol-version", "2025-06-18")]), &msg)
			.unwrap_err();
	assert!(matches!(
		err,
		Error::VersionMismatch(Some(RequestId::Number(7)))
	));
}

#[test]
fn extract_validates_standard_method_header_on_modern() {
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);

	let ctx = RequestProtocolContext::extract(
		&headers(&[(HEADER_MCP_METHOD, "tools/call"), (HEADER_MCP_NAME, "echo")]),
		&msg,
	)
	.unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2026_07_28));

	// A standard `Mcp-Method` header that disagrees with the body is the header's fault.
	let msg = {
		let mut body = tools_call_body("echo");
		body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
		message(body)
	};
	let err = RequestProtocolContext::extract(&headers(&[(HEADER_MCP_METHOD, "tools/list")]), &msg)
		.unwrap_err();
	assert!(matches!(
		err,
		Error::HeaderBodyMismatch(Some(RequestId::Number(7)), header) if header == HEADER_MCP_METHOD
	));
}

#[test]
fn extract_rejects_mismatched_standard_name_on_modern() {
	// `Mcp-Name` claims `safe` while the body calls `dangerous`: the standard `Mcp-Name`
	// header must track the body resource, so this is rejected (confused-deputy guard).
	let mut body = tools_call_body("dangerous");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let err = RequestProtocolContext::extract(
		&headers(&[(HEADER_MCP_METHOD, "tools/call"), (HEADER_MCP_NAME, "safe")]),
		&msg,
	)
	.unwrap_err();
	assert!(matches!(
		err,
		Error::HeaderBodyMismatch(Some(RequestId::Number(7)), header) if header == HEADER_MCP_NAME
	));

	// A matching name is accepted.
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let ctx = RequestProtocolContext::extract(
		&headers(&[(HEADER_MCP_METHOD, "tools/call"), (HEADER_MCP_NAME, "echo")]),
		&msg,
	)
	.unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2026_07_28));
}

#[test]
fn extract_rejects_malformed_standard_header_on_modern() {
	// A present but undecodable routing header must fail closed. If the parser
	// turns it into `None`, the body/header check never runs.
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let mut h = ::http::HeaderMap::new();
	h.insert(
		::http::HeaderName::from_static("mcp-method"),
		::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
	);
	let err = RequestProtocolContext::extract(&h, &msg).unwrap_err();
	assert!(matches!(
		err,
		Error::InvalidRoutingHeader(Some(RequestId::Number(7)), header)
			if header == HEADER_MCP_METHOD
	));
}

#[test]
fn extract_rejects_malformed_standard_name_on_modern() {
	// `Mcp-Name` uses the same fail-closed rule. If it is present but undecodable,
	// the gateway rejects it instead of treating it as absent.
	let mut body = tools_call_body("echo");
	body["params"]["_meta"] = json!({ META_PROTOCOL_VERSION_KEY: "2026-07-28" });
	let msg = message(body);
	let mut h = ::http::HeaderMap::new();
	h.insert(
		::http::HeaderName::from_static("mcp-method"),
		::http::HeaderValue::from_static("tools/call"),
	);
	h.insert(
		::http::HeaderName::from_static("mcp-name"),
		::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
	);
	let err = RequestProtocolContext::extract(&h, &msg).unwrap_err();
	assert!(matches!(
		err,
		Error::InvalidRoutingHeader(Some(RequestId::Number(7)), header)
			if header == HEADER_MCP_NAME
	));
}

#[test]
fn extract_rejects_malformed_standard_header_on_legacy() {
	// Validation is presence-gated, not version-gated: an undecodable `Mcp-Name`
	// fails closed even on a legacy request, so the confused-deputy guard can't be
	// bypassed by claiming a legacy version.
	let msg = message(tools_call_body("echo"));
	let mut h = ::http::HeaderMap::new();
	h.insert(
		::http::HeaderName::from_static("mcp-protocol-version"),
		::http::HeaderValue::from_static("2025-06-18"),
	);
	h.insert(
		::http::HeaderName::from_static("mcp-name"),
		::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
	);
	let err = RequestProtocolContext::extract(&h, &msg).unwrap_err();
	assert!(matches!(
		err,
		Error::InvalidRoutingHeader(Some(RequestId::Number(7)), header)
			if header == HEADER_MCP_NAME
	));
}

#[test]
fn extract_validates_standard_headers_on_legacy() {
	// Presence-gated: a mismatched `Mcp-Method` is rejected on a legacy request too,
	// not just modern.
	let msg = message(tools_call_body("echo"));
	let err = RequestProtocolContext::extract(
		&headers(&[
			("mcp-protocol-version", "2025-06-18"),
			(HEADER_MCP_METHOD, "tools/list"),
		]),
		&msg,
	)
	.unwrap_err();
	assert!(matches!(
		err,
		Error::HeaderBodyMismatch(Some(RequestId::Number(7)), header)
			if header == HEADER_MCP_METHOD
	));
}

#[test]
fn extract_validates_standard_method_header_on_modern_notifications() {
	let notification = json!({
		"jsonrpc": "2.0",
		"method": "notifications/initialized",
		"params": {}
	});
	let msg = message(notification);
	let ctx = RequestProtocolContext::extract(
		&headers(&[
			("mcp-protocol-version", "2026-07-28"),
			(HEADER_MCP_METHOD, "notifications/initialized"),
		]),
		&msg,
	)
	.unwrap();
	assert_eq!(ctx.version, Some(ProtocolVersion::V_2026_07_28));

	let err = RequestProtocolContext::extract(
		&headers(&[
			("mcp-protocol-version", "2026-07-28"),
			(HEADER_MCP_METHOD, "notifications/progress"),
		]),
		&msg,
	)
	.unwrap_err();
	assert!(matches!(err, Error::HeaderBodyMismatch(None, header) if header == HEADER_MCP_METHOD));
}

#[test]
fn validate_version_header_covers_bodyless_requests() {
	assert_eq!(
		validate_version_header(&headers(&[("mcp-protocol-version", "2026-07-28")])).unwrap(),
		Some(ProtocolVersion::V_2026_07_28)
	);
	assert_eq!(
		validate_version_header(&::http::HeaderMap::new()).unwrap(),
		None
	);
	// Bodyless rejection carries no id, so the response stays HTTP-status-only.
	assert!(matches!(
		validate_version_header(&headers(&[("mcp-protocol-version", "1900-01-01")])),
		Err(Error::UnsupportedVersion(None, _))
	));
}

/// Keep the missing downgrade/upgrade cases visible until the gateway can run
/// against a real RC peer.
mod compatibility_gaps {
	#[test]
	#[ignore = "RC-client -> legacy-server: needs version downgrade negotiation"]
	fn rc_client_to_legacy_server() {}

	#[test]
	#[ignore = "legacy-client -> RC-server: needs modern stateless transport"]
	fn legacy_client_to_rc_server() {}
}
