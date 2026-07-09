//! Support for the MCP Apps extension (`io.modelcontextprotocol/ui`).
//!
//! Apps UI resources use `ui://` URIs. When multiplexing, the routing target is
//! carried inside the URI authority (`ui://{target}+{rest}`) so the client
//! still sees a valid `ui://` URI, unlike other schemes which are prefixed as
//! `{target}+{scheme}://rest`.

use rmcp::model::{ServerJsonRpcMessage, ServerResult};
use tracing::debug;

const UI_META_KEY: &str = "ui";
const UI_RESOURCE_URI_KEY: &str = "resourceUri";
// Deprecated flat form of `_meta.ui.resourceUri`. Current SDKs emit both keys
// and hosts fall back to this one, so it is rewritten and stripped in tandem.
const UI_FLAT_RESOURCE_URI_KEY: &str = "ui/resourceUri";

const UI_SCHEME: &str = "ui://";

/// Returns the part after the `ui://` scheme. Scheme matching is
/// case-insensitive per RFC 3986.
fn strip_ui_scheme(uri: &str) -> Option<&str> {
	uri
		.get(..UI_SCHEME.len())
		.is_some_and(|scheme| scheme.eq_ignore_ascii_case(UI_SCHEME))
		.then(|| &uri[UI_SCHEME.len()..])
}

pub(crate) fn is_ui_uri(uri: &str) -> bool {
	strip_ui_scheme(uri).is_some()
}

/// Encode an upstream `ui://` URI into the client-visible multiplexed form
/// `ui://{target}+{rest}`. Returns None for non-`ui://` URIs.
pub(crate) fn encode_ui_uri(target: &str, uri: &str) -> Option<String> {
	let rest = strip_ui_scheme(uri)?;
	Some(format!("{UI_SCHEME}{target}+{rest}"))
}

/// Reverse of `encode_ui_uri`: split `ui://{target}+{rest}` into the target and
/// the original `ui://{rest}` URI. The delimiter must sit in the authority;
/// target names never contain `+`, so the first `+` is always ours.
pub(crate) fn decode_ui_uri(uri: &str) -> Option<(&str, String)> {
	let rest = strip_ui_scheme(uri)?;
	// The target+ delimiter must sit in the authority (before any /?#).
	let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
	let plus = rest[..authority_end].find('+')?;
	let target = &rest[..plus];
	if target.is_empty() {
		return None;
	}
	Some((target, format!("{UI_SCHEME}{}", &rest[plus + 1..])))
}

/// Rewrite or strip the UI resource URI to include our federation target, in the `_meta.ui.resourceUri`
/// (and deprecated `_meta.ui/resourceUri`) field of each tool in the `tools/list` result.
/// https://github.com/modelcontextprotocol/ext-apps/blob/cf87f2a2c2581b2bc45ff4848aac9fa7e106a576/specification/draft/apps.mdx#L406-L412
///
/// Apply RBAC on the ui resource to avoid advertising it if it would be denied later on
/// resources/read.
///
/// Although the URI is present in other result messages, we only care about tools/list:
/// https://github.com/modelcontextprotocol/ext-apps/blob/cf87f2a2c2581b2bc45ff4848aac9fa7e106a576/specification/draft/apps.mdx#L2370-L2373
pub(crate) fn rewrite_tool_list_ui_meta(
	multiplexing: bool,
	target: &str,
	resource_allowed: &mut impl FnMut(&str) -> bool,
	mut message: ServerJsonRpcMessage,
) -> ServerJsonRpcMessage {
	if let ServerJsonRpcMessage::Response(resp) = &mut message
		&& let ServerResult::ListToolsResult(r) = &mut resp.result
	{
		for t in &mut r.tools {
			let Some(m) = t.meta.as_mut() else { continue };
			if !m.0.contains_key(UI_META_KEY) && !m.0.contains_key(UI_FLAT_RESOURCE_URI_KEY) {
				continue;
			}
			if let Some(obj) = m.0.get_mut(UI_META_KEY).and_then(|ui| ui.as_object_mut()) {
				if !validate_and_rewrite_uri_value(
					obj.get_mut(UI_RESOURCE_URI_KEY),
					multiplexing,
					target,
					&mut *resource_allowed,
				) {
					// do not strip the whole `_meta.ui` object, so we preserve the visibility of the tool
					// e.g. visibility: ["app"] should not go to the model
					debug!(%target, tool = %t.name, "stripping denied _meta.ui.resourceUri");
					obj.remove(UI_RESOURCE_URI_KEY);
				}
				if obj.is_empty() {
					m.0.remove(UI_META_KEY);
				}
			}
			if !validate_and_rewrite_uri_value(
				m.0.get_mut(UI_FLAT_RESOURCE_URI_KEY),
				multiplexing,
				target,
				&mut *resource_allowed,
			) {
				debug!(%target, tool = %t.name, "stripping denied ui/resourceUri");
				m.0.remove(UI_FLAT_RESOURCE_URI_KEY);
			}
			if m.0.is_empty() {
				t.meta = None;
			}
		}
	}
	message
}

/// validates that the original URI + target is allowed by the RBAC policy,
/// and possibly rewrites `ui://` URIs to include federation target in place
fn validate_and_rewrite_uri_value(
	value: Option<&mut serde_json::Value>,
	multiplexing: bool,
	target: &str,
	resource_allowed: &mut impl FnMut(&str) -> bool,
) -> bool {
	let Some(value) = value else { return true };
	let Some(uri) = value.as_str() else {
		return true;
	};
	if !resource_allowed(uri) {
		return false;
	}
	if multiplexing && let Some(rewritten) = encode_ui_uri(target, uri) {
		*value = serde_json::Value::String(rewritten);
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn encode_prefixes_target_in_authority() {
		assert_eq!(
			encode_ui_uri("a", "ui://weather/dashboard.html").as_deref(),
			Some("ui://a+weather/dashboard.html")
		);
		assert_eq!(
			encode_ui_uri("a", "UI://weather/dashboard.html").as_deref(),
			Some("ui://a+weather/dashboard.html")
		);
		assert_eq!(
			encode_ui_uri("a", "ui://weather+extra/x?q=1#f").as_deref(),
			Some("ui://a+weather+extra/x?q=1#f")
		);
		assert_eq!(encode_ui_uri("a", "memo://insights"), None);
	}

	#[test]
	fn decode_roundtrip() {
		let encoded = encode_ui_uri("a", "ui://weather/dashboard.html").unwrap();
		let (target, original) = decode_ui_uri(&encoded).unwrap();
		assert_eq!(target, "a");
		assert_eq!(original, "ui://weather/dashboard.html");

		// '+' later in the original authority stays with the upstream URI
		let encoded = encode_ui_uri("a", "ui://weather+extra/x?q=1#f").unwrap();
		let (target, original) = decode_ui_uri(&encoded).unwrap();
		assert_eq!(target, "a");
		assert_eq!(original, "ui://weather+extra/x?q=1#f");
	}

	#[test]
	fn decode_rejects_malformed_uris() {
		assert_eq!(decode_ui_uri("ui://no-target/dashboard.html"), None);
		assert_eq!(decode_ui_uri("ui://+empty-target/x"), None);
		// '+' only after the authority does not carry a target
		assert_eq!(decode_ui_uri("ui://host/a+b"), None);
		assert_eq!(decode_ui_uri("memo://a+b"), None);
	}
}
