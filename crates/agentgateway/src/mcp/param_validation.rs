//! SEP-2243 custom routing header (`Mcp-Param-*`) deep validation.
//!
//! The transport layer (`streamablehttp::validate_standard_headers`) only shallow-checks that an
//! inbound `Mcp-Param-*` value decodes. Full server-side validation also needs the resolved
//! tool's `inputSchema`: resolve each `x-mcp-header` annotation, then confirm every inbound
//! `Mcp-Param-*` header agrees with the corresponding call argument. That lives here and is driven
//! from the `tools/call` site (which has the schema) and the `tools/list` filter.
//!
//! Values are decoded with rmcp's `decode_header_value` (base64 sentinel unwrap, else verbatim);
//! HTTP OWS is trimmed first, since rmcp does not.

use rmcp::model::RequestId;
use rmcp::transport::common::http_header::HEADER_MCP_PARAM_PREFIX;
use rmcp::transport::common::mcp_headers::decode_header_value;

use crate::mcp::Error;

/// Generic label for the dynamic `Mcp-Param-{name}` family. The typed `Error` variants carry a
/// `&'static str`, and custom-header names are not static, so failures report the family.
pub(crate) const HEADER_MCP_PARAM: &str = "Mcp-Param-*";

/// Pre-authorization snapshot of a `tools/call` needed to validate its `Mcp-Param-*` routing
/// headers. Captured before guardrails mutate the call params: `arguments` is matched against the
/// headers, and `meta` is replayed onto the synthetic schema-fetch `tools/list` (SEP-2575 requires
/// `_meta` on every modern request).
pub(crate) struct RoutingHeaderSnapshot {
	pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
	pub meta: Option<rmcp::model::Meta>,
	/// The client's inbound `Mcp-Param-*` headers. Captured before guardrails run, because a policy
	/// header mutation could otherwise rewrite or drop them and mask a client header/body mismatch —
	/// this check exists to validate what the client actually sent.
	pub param_headers: ::http::HeaderMap,
}

/// 2^53 - 1, the JS safe-integer bound SEP-2243 requires for integer custom-header values.
const JS_MAX_SAFE_INT: i64 = 9_007_199_254_740_991;

fn is_js_safe_int(n: i64) -> bool {
	(-JS_MAX_SAFE_INT..=JS_MAX_SAFE_INT).contains(&n)
}

/// Whether `name` is an `Mcp-Param-*` custom routing header (case-insensitive prefix match).
pub(crate) fn is_mcp_param_header(name: &::http::HeaderName) -> bool {
	name
		.as_str()
		.get(..HEADER_MCP_PARAM_PREFIX.len())
		.is_some_and(|p| p.eq_ignore_ascii_case(HEADER_MCP_PARAM_PREFIX))
		&& name.as_str().len() > HEADER_MCP_PARAM_PREFIX.len()
}

/// Copy the inbound `Mcp-Param-*` headers into a standalone map, preserving duplicates so the
/// ambiguous-repeat check still fires.
pub(crate) fn mcp_param_headers(headers: &::http::HeaderMap) -> ::http::HeaderMap {
	let mut out = ::http::HeaderMap::new();
	for (name, value) in headers {
		if is_mcp_param_header(name) {
			out.append(name.clone(), value.clone());
		}
	}
	out
}

/// OWS-trim then decode a custom routing header value. `None` when a sentinel-wrapped value is
/// malformed because the value is bad base64 or non-UTF-8. A non-sentinel value
/// passes through verbatim.
fn trim_and_decode(value: &str) -> Option<String> {
	decode_header_value(value.trim_matches(|c| c == ' ' || c == '\t'))
}

/// Parse a SEP-2243 integer custom-header value exactly. Accepts a plain decimal (`42`, `-7`) or a
/// decimal with an all-zero fraction (`42.0`), honoring the SEP's numeric comparison
/// (`42` == `42.0`), but rejects any non-integer value. Parsing as `i64` rather than `f64` keeps
/// the confused-deputy check sound: `f64` would round `42.000000000000001`
/// onto the body integer and falsely match. Near 2^53, it can also round
/// fractional values like `9007199254740991.4` onto the body integer.
fn parse_exact_integer(s: &str) -> Option<i64> {
	match s.split_once('.') {
		None => s.parse().ok(),
		Some((int_part, frac)) if frac.bytes().all(|b| b == b'0') => int_part.parse().ok(),
		Some(_) => None,
	}
}

/// SEP-2243 primitive type an `x-mcp-header` parameter may have. `number`/array/object/null are not
/// permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XMcpPrimitive {
	String,
	Integer,
	Boolean,
}

/// A resolved SEP-2243 mapping: a top-level tool parameter mirrored into `Mcp-Param-{name}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XMcpHeaderParam {
	/// The `Mcp-Param-{name}` header (normalized lowercase).
	pub header: ::http::HeaderName,
	/// The top-level argument key whose value is mirrored.
	pub param: String,
	pub ty: XMcpPrimitive,
}

/// A tool's `x-mcp-header` annotations violate SEP-2243.
///
/// The client must exclude the tool from `tools/list`, and the gateway rejects a
/// direct call to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XMcpHeaderError {
	pub param: String,
	pub reason: &'static str,
}

impl XMcpHeaderError {
	fn new(param: &str, reason: &'static str) -> Self {
		Self {
			param: param.to_string(),
			reason,
		}
	}
}

impl std::fmt::Display for XMcpHeaderError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"invalid x-mcp-header on parameter {:?}: {}",
			self.param, self.reason
		)
	}
}

/// Per-value validation outcome. All map to `HEADER_MISMATCH` at the call site; kept distinct for
/// messages/tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XMcpValueError {
	/// Decoded header value disagrees with the body argument.
	Mismatch,
	/// Sentinel-wrapped value could not be decoded.
	Undecodable,
	/// Integer argument value outside the JS safe range.
	OutOfRange,
}

/// RFC 9110 §5.6.2 `tchar`.
fn is_tchar(c: char) -> bool {
	c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

/// Whether `value` contains the `x-mcp-header` key at any depth. Used to reject nested annotations.
fn contains_x_mcp_header(value: &serde_json::Value) -> bool {
	match value {
		serde_json::Value::Object(map) => {
			map.contains_key("x-mcp-header") || map.values().any(contains_x_mcp_header)
		},
		serde_json::Value::Array(items) => items.iter().any(contains_x_mcp_header),
		_ => false,
	}
}

fn primitive_type(
	param: &str,
	def: &serde_json::Map<String, serde_json::Value>,
) -> Result<XMcpPrimitive, XMcpHeaderError> {
	match def.get("type").and_then(serde_json::Value::as_str) {
		Some("string") => Ok(XMcpPrimitive::String),
		Some("integer") => Ok(XMcpPrimitive::Integer),
		Some("boolean") => Ok(XMcpPrimitive::Boolean),
		// `number`/array/object/null and missing/non-string `type` are not permitted.
		_ => Err(XMcpHeaderError::new(
			param,
			"x-mcp-header parameter must be type string, integer, or boolean",
		)),
	}
}

/// Resolve the SEP-2243 `x-mcp-header` mappings from a tool `inputSchema`.
///
/// Top-level `properties` only. SEP prose (line 169) permits nesting, but the conformance edge-case
/// table (line 744) and the conformance suite treat a nested annotation as an invalid tool
/// definition, so we follow the suite. `Err` if any annotation is invalid, nested, duplicated
/// (case-insensitive), or non-primitive. The caller excludes (tools/list) or rejects (tools/call).
pub(crate) fn x_mcp_header_params(
	input_schema: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<XMcpHeaderParam>, XMcpHeaderError> {
	// A valid annotation sits on a top-level property. One anywhere else is the
	// nested case the suite rejects, including root-level `$defs`/`allOf`/`$ref`
	// and annotations below any property. This loop catches root-level siblings;
	// the per-property loop below catches nesting inside a property.
	for (key, value) in input_schema {
		if key != "properties" && contains_x_mcp_header(value) {
			return Err(XMcpHeaderError::new(key, "x-mcp-header must not be nested"));
		}
	}

	let Some(serde_json::Value::Object(props)) = input_schema.get("properties") else {
		return Ok(Vec::new());
	};

	let mut out: Vec<XMcpHeaderParam> = Vec::new();
	let mut seen: Vec<String> = Vec::new();
	for (param, def) in props {
		let serde_json::Value::Object(def) = def else {
			continue;
		};
		// A property's own `x-mcp-header` is top-level and allowed. The same key
		// anywhere in its sub-schema is nested.
		if def
			.iter()
			.any(|(k, v)| k != "x-mcp-header" && contains_x_mcp_header(v))
		{
			return Err(XMcpHeaderError::new(
				param,
				"x-mcp-header must not be nested",
			));
		}
		let Some(annotation) = def.get("x-mcp-header") else {
			continue;
		};
		let name = annotation
			.as_str()
			.ok_or_else(|| XMcpHeaderError::new(param, "x-mcp-header must be a string"))?;
		if name.is_empty() {
			return Err(XMcpHeaderError::new(
				param,
				"x-mcp-header must not be empty",
			));
		}
		if !name.chars().all(is_tchar) {
			return Err(XMcpHeaderError::new(
				param,
				"x-mcp-header must be an RFC 9110 token",
			));
		}
		let lower = name.to_ascii_lowercase();
		if seen.contains(&lower) {
			return Err(XMcpHeaderError::new(
				param,
				"x-mcp-header must be case-insensitively unique",
			));
		}
		let ty = primitive_type(param, def)?;
		let header = ::http::HeaderName::try_from(format!("{HEADER_MCP_PARAM_PREFIX}{lower}"))
			.map_err(|_| XMcpHeaderError::new(param, "x-mcp-header yields an invalid header name"))?;
		seen.push(lower);
		out.push(XMcpHeaderParam {
			header,
			param: param.clone(),
			ty,
		});
	}
	Ok(out)
}

/// Validate one decoded custom-header value against its body argument, per SEP-2243 type rules.
/// Integers compare numerically (`42` == `42.0`) and must be in JS safe range; booleans are
/// lowercase `true`/`false`; strings compare verbatim.
fn validate_param_value(
	header: &str,
	arg: &serde_json::Value,
	ty: XMcpPrimitive,
) -> Result<(), XMcpValueError> {
	let decoded = trim_and_decode(header).ok_or(XMcpValueError::Undecodable)?;
	match ty {
		XMcpPrimitive::Integer => {
			if !arg.is_number() {
				return Err(XMcpValueError::Mismatch);
			}
			// serde_json stores `42.0` as f64, so `as_i64` misses integral floats; recover them
			// (exact for `|n| <= 2^53-1`) to honor the SEP's `42 == 42.0` numeric comparison.
			let body = arg
				.as_i64()
				.or_else(|| arg.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
				.filter(|n| is_js_safe_int(*n))
				.ok_or(XMcpValueError::OutOfRange)?;
			let header_int = parse_exact_integer(&decoded).ok_or(XMcpValueError::Mismatch)?;
			if header_int != body {
				return Err(XMcpValueError::Mismatch);
			}
			Ok(())
		},
		XMcpPrimitive::Boolean => {
			let body = arg.as_bool().ok_or(XMcpValueError::Mismatch)?;
			let expected = if body { "true" } else { "false" };
			(decoded == expected)
				.then_some(())
				.ok_or(XMcpValueError::Mismatch)
		},
		XMcpPrimitive::String => {
			let body = arg.as_str().ok_or(XMcpValueError::Mismatch)?;
			(decoded == body)
				.then_some(())
				.ok_or(XMcpValueError::Mismatch)
		},
	}
}

/// Validate the inbound `Mcp-Param-*` headers for a resolved tool call against its `x-mcp-header`
/// mappings and the call arguments. Caller gates on modern.
///
/// Validate-if-present contract: the gateway validates an `Mcp-Param-*` header when the client
/// sends it, but does not *require* one for an annotated argument. An absent routing header is
/// therefore never an error (the caller also skips the schema fetch entirely when none are present).
/// Rejects as `HEADER_MISMATCH`: an `Mcp-Param-*` with no declared mapping (unexpected), a declared
/// param sent more than once (ambiguous), a present header for an absent/null argument, and any
/// value that fails `validate_param_value`.
pub(crate) fn validate_custom_param_headers(
	params: &[XMcpHeaderParam],
	arguments: Option<&serde_json::Map<String, serde_json::Value>>,
	headers: &::http::HeaderMap,
	id: &Option<RequestId>,
) -> Result<(), Error> {
	let invalid = || Error::InvalidRoutingHeader(id.clone(), HEADER_MCP_PARAM);
	let mismatch = || Error::HeaderBodyMismatch(id.clone(), HEADER_MCP_PARAM);

	// Every inbound Mcp-Param-* must map to a declared annotation.
	for name in headers.keys().filter(|k| is_mcp_param_header(k)) {
		if !params.iter().any(|p| p.header == *name) {
			return Err(invalid());
		}
	}

	let empty = serde_json::Map::new();
	let args = arguments.unwrap_or(&empty);
	for p in params {
		// A routing header sent more than once is ambiguous; reject rather than honor the first.
		let mut values = headers.get_all(&p.header).iter();
		let header = match values.next() {
			None => None,
			Some(v) => {
				if values.next().is_some() {
					return Err(invalid());
				}
				Some(v.to_str().map_err(|_| invalid())?)
			},
		};
		let arg = args.get(&p.param).filter(|v| !v.is_null());
		match (header, arg) {
			(Some(h), Some(a)) => validate_param_value(h, a, p.ty).map_err(|e| match e {
				XMcpValueError::Mismatch => mismatch(),
				XMcpValueError::Undecodable | XMcpValueError::OutOfRange => invalid(),
			})?,
			// Validate-if-present: the client omitted this routing header, so there is nothing to
			// check. A body value without its header is a missing hint, not a mismatch.
			(None, Some(_)) => {},
			// A present header for an absent/null argument asserts a routing value the body does not
			// carry — reject it.
			(Some(_), None) => return Err(mismatch()),
			(None, None) => {},
		}
	}
	Ok(())
}

/// Resolve a resolved tool's `x-mcp-header` mappings from its `inputSchema` and validate the inbound
/// `Mcp-Param-*` headers against the call arguments. A tool whose annotations are themselves invalid
/// is rejected (`InvalidRoutingHeader`); the tools/list filter only gates the list path, not a
/// direct call. Caller gates on modern.
pub(crate) fn validate_tool_call_headers(
	input_schema: &serde_json::Map<String, serde_json::Value>,
	arguments: Option<&serde_json::Map<String, serde_json::Value>>,
	headers: &::http::HeaderMap,
	id: &Option<RequestId>,
) -> Result<(), Error> {
	let params = x_mcp_header_params(input_schema)
		.map_err(|_| Error::InvalidRoutingHeader(id.clone(), HEADER_MCP_PARAM))?;
	validate_custom_param_headers(&params, arguments, headers, id)
}

#[cfg(test)]
#[path = "param_validation_tests.rs"]
mod param_validation_tests;
