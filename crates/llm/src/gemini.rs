use agent_core::strng;
use agent_core::strng::Strng;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use crate::{RouteType, apply};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "GeminiProvider"))]
pub struct Provider {
	/// Model ID to send to Gemini, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("gcp.gemini");
}
pub const DEFAULT_HOST_STR: &str = "generativelanguage.googleapis.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);
/// Google API keys are uniformly `AIza`-prefixed. Used to distinguish API keys (which the
/// native endpoints authenticate via `x-goog-api-key`) from OAuth access tokens (which stay
/// in `Authorization: Bearer`).
pub const API_KEY_PREFIX: &str = "AIza";

pub fn path(route: RouteType) -> &'static str {
	match route {
		RouteType::Embeddings => "/v1beta/openai/embeddings",
		RouteType::Rerank => "/rerank",
		_ => "/v1beta/openai/chat/completions",
	}
}

/// Native `{model}:{method}` path, used instead of [`path`] when the request was rendered in the
/// native Gemini format. `?alt=sse` is required on the streaming endpoint; without it Google
/// returns a JSON array rather than an SSE stream.
pub fn native_gemini_path(route: RouteType, request_model: &str, streaming: bool) -> Strng {
	let model = model_resource_name(request_model);
	match route {
		RouteType::GeminiCountTokens => strng::format!("/v1beta/{model}:countTokens"),
		_ if streaming => strng::format!("/v1beta/{model}:streamGenerateContent?alt=sse"),
		_ => strng::format!("/v1beta/{model}:generateContent"),
	}
}

/// `{collection}/{id}` for the `/v1beta/{model}:{method}` URL bindings, which accept `models/*`
/// and `tunedModels/*`. Configs and clients write either the bare id or the full resource name,
/// and the router percent-encodes a target's `/` to keep it in one segment, so both arrive here.
/// The id is escaped rather than interpolated: it can never open a path segment of its own.
fn model_resource_name(request_model: &str) -> String {
	const MODEL_SEGMENT: &AsciiSet = &CONTROLS.add(b'/').add(b'%');
	let (collection, id) = match request_model.split_once('/') {
		Some(("models", id)) => ("models", id),
		Some(("tunedModels", id)) => ("tunedModels", id),
		_ => ("models", request_model),
	};
	format!("{collection}/{}", utf8_percent_encode(id, MODEL_SEGMENT))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[rstest::rstest]
	#[case::bare("gemini-2.5-flash", "/v1beta/models/gemini-2.5-flash:generateContent")]
	#[case::resource_name(
		"models/gemini-2.5-flash",
		"/v1beta/models/gemini-2.5-flash:generateContent"
	)]
	#[case::tuned_model("tunedModels/abc", "/v1beta/tunedModels/abc:generateContent")]
	#[case::traversal(
		"gemini-2.5-flash/../../v1beta/models/other",
		"/v1beta/models/gemini-2.5-flash%2F..%2F..%2Fv1beta%2Fmodels%2Fother:generateContent"
	)]
	#[case::absolute(
		"/v1beta/models/other",
		"/v1beta/models/%2Fv1beta%2Fmodels%2Fother:generateContent"
	)]
	#[case::empty("", "/v1beta/models/:generateContent")]
	fn native_gemini_path_keeps_the_model_in_one_segment(
		#[case] request_model: &str,
		#[case] expected: &str,
	) {
		let path = native_gemini_path(RouteType::GenerateContent, request_model, false);
		assert_eq!(path.as_str(), expected);
	}

	#[test]
	fn native_gemini_path_covers_the_other_two_methods() {
		assert_eq!(
			native_gemini_path(RouteType::GeminiCountTokens, "tunedModels/abc", false).as_str(),
			"/v1beta/tunedModels/abc:countTokens"
		);
		assert_eq!(
			native_gemini_path(RouteType::GenerateContent, "tunedModels/abc", true).as_str(),
			"/v1beta/tunedModels/abc:streamGenerateContent?alt=sse"
		);
	}
}
