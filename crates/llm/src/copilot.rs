use agent_core::strng;
use agent_core::strng::Strng;
use http::{HeaderMap, HeaderValue, header};
use serde_json::{Map, Value};

use crate::{AIError, ChatFormat, RouteType, apply, logged_response_parsing, types};

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "CopilotProvider"))]
pub struct Provider {
	/// Model ID to send to GitHub Copilot, overriding the model in the client request.
	#[serde(default, rename = "model", skip_serializing_if = "Option::is_none")]
	pub model_override: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("copilot");
}

impl Provider {
	pub fn is_anthropic_model(request_model: &str) -> bool {
		request_model.to_ascii_lowercase().starts_with("claude-")
	}
	pub fn set_required_headers(headers: &mut HeaderMap) {
		headers.insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static("application/json"),
		);
		headers.insert(
			"editor-version",
			HeaderValue::from_static(concat!("agentgateway/", env!("CARGO_PKG_VERSION"))),
		);
		headers.insert(
			"openai-intent",
			HeaderValue::from_static("conversation-agent"),
		);
		headers.insert(
			"x-github-api-version",
			HeaderValue::from_static("2025-10-01"),
		);
		headers.insert("x-initiator", HeaderValue::from_static("agent"));
		headers.insert(
			"x-interaction-type",
			HeaderValue::from_static("conversation-agent"),
		);
	}
	pub fn supported_formats_for_model(
		request_model: &str,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Vec<ChatFormat> {
		let m = request_model;
		let normalized_model = m.to_ascii_lowercase();
		// TODO: also support endpoint parsing from copilot models and add a tool to grab specific setups in agctl
		if let Some(tags) = catalog.and_then(|c| c.get_model_tags(&normalized_model)) {
			let formats: Vec<ChatFormat> = [
				ChatFormat::OpenAICompletions,
				ChatFormat::OpenAIResponses,
				ChatFormat::AnthropicMessages,
				ChatFormat::BedrockConverse,
				ChatFormat::VertexGemini,
			]
			.into_iter()
			.filter(|f| tags.contains(f.tag()))
			.collect();
			if !formats.is_empty() {
				tracing::debug!(model = %m, ?formats, "copilot formats from modelcatalog tags");
				return formats;
			}
		}
		// Truth table from `curl https://api.githubcopilot.com/models -H "Authorization: Bearer ghu_..." | '.data[] | {id,supported_endpoints}'`
		match normalized_model.as_str() {
			m if m.starts_with("claude-") => {
				// Copilot supports Completions even for Anthropic
				// This is enabled so we can do Responses --> Completions [--> Anthropic, within copilot, presumably].
				// If we add native Responses --> Anthropic we should drop this
				vec![ChatFormat::AnthropicMessages, ChatFormat::OpenAICompletions]
			},
			m if m.starts_with("grok-") || m.starts_with("mai-") => {
				vec![ChatFormat::OpenAIResponses]
			},
			m if m.starts_with("gemini-") => {
				vec![ChatFormat::OpenAICompletions]
			},
			m if m.starts_with("gpt-3") || m.starts_with("gpt-4") => {
				vec![ChatFormat::OpenAICompletions]
			},
			"gpt-5.4" | "gpt-5-mini" => {
				vec![ChatFormat::OpenAICompletions, ChatFormat::OpenAIResponses]
			},
			m if m.starts_with("gpt-") => {
				vec![ChatFormat::OpenAIResponses]
			},
			m if m.starts_with("microsoft-decision-") || m.starts_with("text-embedding-") => vec![],
			_ => vec![ChatFormat::OpenAICompletions],
		}
	}

	pub fn translate_embeddings_request(
		req: &types::embeddings::Request,
	) -> Result<Vec<u8>, AIError> {
		if let Value::String(s) = &req.input {
			let mut normalized = req.clone();
			normalized.input = Value::Array(vec![Value::String(s.clone())]);
			serde_json::to_vec(&normalized).map_err(AIError::RequestMarshal)
		} else {
			serde_json::to_vec(req).map_err(AIError::RequestMarshal)
		}
	}

	pub fn translate_embeddings_response(
		bytes: &[u8],
		request_model: &str,
	) -> Result<(types::embeddings::Response, Vec<u8>), AIError> {
		let mut resp: Map<String, Value> =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		resp
			.entry("object".to_string())
			.or_insert_with(|| Value::String("list".to_string()));
		resp
			.entry("model".to_string())
			.or_insert_with(|| Value::String(request_model.to_string()));
		let normalized = serde_json::to_vec(&resp).map_err(AIError::ResponseParsing)?;
		let resp: types::embeddings::Response =
			serde_json::from_slice(&normalized).map_err(logged_response_parsing(&normalized))?;
		Ok((resp, normalized))
	}
}

pub const DEFAULT_HOST_STR: &str = "api.githubcopilot.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path_suffix(route: RouteType) -> &'static str {
	match route {
		RouteType::Embeddings => "/embeddings",
		RouteType::Messages => "/v1/messages",
		RouteType::Models => "/models",
		RouteType::Rerank => "/rerank",
		RouteType::Responses => "/responses",
		RouteType::SystemOne => "/v1/systemone",
		_ => "/chat/completions",
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;
	use crate::LogContentFields;
	use crate::model_catalog::{Catalog, TestCatalog, tags};
	use crate::types::ResponseType;

	#[test]
	fn catalog_format_tags_are_case_insensitive() {
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("Grok-2", catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn catalog_format_tags_override_builtins() {
		// grok-* defaults to Responses; a catalog tag forces Completions instead.
		let cat = TestCatalog::new([("grok-2", &[tags::OPENAI_COMPLETIONS][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("grok-2", catalog),
			vec![ChatFormat::OpenAICompletions]
		);
	}

	#[test]
	fn decision_model_has_no_chat_formats_and_uses_systemone_path() {
		assert!(Provider::supported_formats_for_model("microsoft-decision-1", None).is_empty());
		assert_eq!(path_suffix(RouteType::SystemOne), "/v1/systemone");
	}

	#[test]
	fn untagged_model_falls_back_to_builtins() {
		let cat = TestCatalog::new([("grok-2", &[][..])]);
		let catalog: Catalog = Some(&cat);
		assert_eq!(
			Provider::supported_formats_for_model("grok-2", catalog),
			vec![ChatFormat::OpenAIResponses]
		);
	}

	#[test]
	fn embedding_model_has_no_chat_formats_and_uses_embeddings_path() {
		assert!(Provider::supported_formats_for_model("text-embedding-3-small", None).is_empty());
		assert_eq!(path_suffix(RouteType::Embeddings), "/embeddings");
	}

	#[test]
	fn translate_embeddings_request_normalizes_string_input() {
		let req: types::embeddings::Request = serde_json::from_value(json!({
			"model": "text-embedding-3-small",
			"input": "hello"
		}))
		.unwrap();

		let body = Provider::translate_embeddings_request(&req).unwrap();
		let body: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(body["model"], json!("text-embedding-3-small"));
		assert_eq!(body["input"], json!(["hello"]));
	}

	#[test]
	fn translate_embeddings_request_preserves_array_input_and_params() {
		let req: types::embeddings::Request = serde_json::from_value(json!({
			"model": "text-embedding-3-small",
			"input": ["hello", "world"],
			"dimensions": 256,
			"encoding_format": "float",
			"user": "alice"
		}))
		.unwrap();

		let body = Provider::translate_embeddings_request(&req).unwrap();
		let body: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(body["model"], json!("text-embedding-3-small"));
		assert_eq!(body["input"], json!(["hello", "world"]));
		assert_eq!(body["dimensions"], json!(256));
		assert_eq!(body["encoding_format"], json!("float"));
		assert_eq!(body["user"], json!("alice"));
	}

	#[test]
	fn translate_embeddings_response_adds_missing_openai_fields() {
		let response =
			br#"{"data":[{"embedding":[0.5,-0.25],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":2,"total_tokens":2}}"#;

		let (resp, body) = Provider::translate_embeddings_response(response, "text-embedding-3-small")
			.expect("Copilot embeddings response should normalize");
		let llm_response = resp.to_llm_response(LogContentFields::default());
		let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

		assert_eq!(body["object"], json!("list"));
		assert_eq!(body["model"], json!("text-embedding-3-small"));
		assert_eq!(body["data"][0]["embedding"], json!([0.5, -0.25]));
		assert_eq!(llm_response.input_tokens, Some(2));
		assert_eq!(llm_response.total_tokens, Some(2));
	}

	#[test]
	fn translate_embeddings_response_preserves_missing_usage() {
		let response = br#"{"data":[{"embedding":[0.5],"index":0,"object":"embedding"}]}"#;

		let (_, body) = Provider::translate_embeddings_response(response, "text-embedding-3-small")
			.expect("Copilot embeddings response should normalize");
		let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

		assert!(body.get("usage").is_none());
	}

	#[test]
	fn translate_embeddings_response_preserves_explicit_openai_fields() {
		let response =
			br#"{"object":"upstream-list","model":"upstream-model","data":[{"embedding":[0.5],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#;

		let (_, body) = Provider::translate_embeddings_response(response, "requested-model")
			.expect("Copilot embeddings response should preserve explicit fields");
		let body: Value = serde_json::from_slice(&body).expect("normalized response should be JSON");

		assert_eq!(body["object"], json!("upstream-list"));
		assert_eq!(body["model"], json!("upstream-model"));
	}

	#[test]
	fn translate_embeddings_response_parse_error_logs_normalized_response() {
		#[derive(Clone)]
		struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

		impl std::io::Write for LogWriter {
			fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
				self.0.lock().unwrap().extend_from_slice(buf);
				Ok(buf.len())
			}

			fn flush(&mut self) -> std::io::Result<()> {
				Ok(())
			}
		}

		let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
		let writer = LogWriter(logs.clone());
		let subscriber = tracing_subscriber::fmt()
			.with_ansi(false)
			.without_time()
			.with_writer(move || writer.clone())
			.finish();

		tracing::subscriber::with_default(subscriber, || {
			let response = br#"{"usage":"invalid"}"#;
			assert!(Provider::translate_embeddings_response(response, "text-embedding-3-small").is_err());
		});

		let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
		assert!(logs.contains(r#""object":"list""#), "{logs}");
		assert!(
			logs.contains(r#""model":"text-embedding-3-small""#),
			"{logs}"
		);
	}

	#[test]
	fn raw_embeddings_response_requires_openai_fields() {
		let response =
			br#"{"data":[{"embedding":[0.5],"index":0,"object":"embedding"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#;

		assert!(serde_json::from_slice::<types::embeddings::Response>(response).is_err());
	}
}
