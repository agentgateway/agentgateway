//! SEP-2243 `x-mcp-header` annotation validity.
//!
//! A tool may annotate a primitive parameter with `x-mcp-header` so a modern client mirrors that
//! parameter's value into an `Mcp-Param-{name}` request header. A client MUST exclude from
//! `tools/list` any tool whose annotations violate the SEP constraints; the gateway is a client
//! toward its upstream, so `handler::merge_tools` applies the same check when merging upstream tools.
//!
//! This validates only the annotations already present in the tool schema — no upstream fetch. It
//! deliberately does not compare `Mcp-Param-*` headers against a call body: that header/body match is
//! the server's responsibility (SEP-2243 "Server Validation").

/// A tool's `x-mcp-header` annotations violate SEP-2243, so the tool is excluded from `tools/list`.
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

/// SEP-2243 permits `x-mcp-header` only on primitive parameters (`string`/`integer`/`boolean`);
/// `number`/array/object/null and a missing/non-string `type` are rejected.
fn validate_primitive_type(
	param: &str,
	def: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), XMcpHeaderError> {
	match def.get("type").and_then(serde_json::Value::as_str) {
		Some("string" | "integer" | "boolean") => Ok(()),
		_ => Err(XMcpHeaderError::new(
			param,
			"x-mcp-header parameter must be type string, integer, or boolean",
		)),
	}
}

/// Validate the SEP-2243 `x-mcp-header` annotations in a tool `inputSchema`.
///
/// Top-level `properties` only. SEP prose permits nesting, but the conformance edge-case table and
/// suite treat a nested annotation as an invalid tool definition, so we follow the suite. `Err` if
/// any annotation is invalid, nested, empty, a non-token, duplicated (case-insensitive), or on a
/// non-primitive parameter. The caller excludes such a tool from `tools/list`.
pub(crate) fn validate_x_mcp_header_annotations(
	input_schema: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), XMcpHeaderError> {
	// A valid annotation sits on a top-level property. One anywhere else is the nested case the suite
	// rejects, including root-level `$defs`/`allOf`/`$ref`. This loop catches root-level siblings; the
	// per-property loop below catches nesting inside a property.
	for (key, value) in input_schema {
		if key != "properties" && contains_x_mcp_header(value) {
			return Err(XMcpHeaderError::new(key, "x-mcp-header must not be nested"));
		}
	}

	let Some(serde_json::Value::Object(props)) = input_schema.get("properties") else {
		return Ok(());
	};

	let mut seen: Vec<String> = Vec::new();
	for (param, def) in props {
		let serde_json::Value::Object(def) = def else {
			continue;
		};
		// A property's own `x-mcp-header` is top-level and allowed. The same key anywhere in its
		// sub-schema is nested.
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
		validate_primitive_type(param, def)?;
		seen.push(lower);
	}
	Ok(())
}

#[cfg(test)]
#[path = "param_validation_tests.rs"]
mod param_validation_tests;
