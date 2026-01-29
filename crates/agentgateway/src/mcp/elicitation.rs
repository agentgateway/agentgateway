use rmcp::model::{JsonObject, Meta, ServerRequest};
use serde::{Deserialize, Serialize};

// Temporary local URL-elicitation shape support until rust-sdk PR #605 lands.
// See: https://github.com/modelcontextprotocol/rust-sdk/pull/605
/// Error code for URL-based elicitation required (SEP-1036).
pub const URL_ELICITATION_REQUIRED_CODE: i32 = -32042;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationMode {
	Form,
	Url,
}

/// URL-based elicitation parameters (SEP-1036).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlElicitationParams {
	/// Protocol-level metadata for this request (SEP-1319)
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<Meta>,
	/// The elicitation mode.
	pub mode: ElicitationMode,
	/// The message to present to the user explaining why the interaction is needed.
	pub message: String,
	/// The ID of the elicitation, which must be unique within the context of the server.
	pub elicitation_id: String,
	/// The URL that the user should navigate to.
	pub url: String,
	/// Task metadata for async task management (SEP-1319).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub task: Option<JsonObject>,
}

pub fn extract_url_elicitation(params: &ServerRequest) -> Option<UrlElicitationParams> {
	let ServerRequest::CustomRequest(custom) = params else {
		return None;
	};
	if custom.method != "elicitation/create" {
		return None;
	}
	custom
		.params_as::<UrlElicitationParams>()
		.ok()
		.flatten()
		.and_then(|params| {
			if params.mode == ElicitationMode::Url {
				Some(params)
			} else {
				None
			}
		})
}
