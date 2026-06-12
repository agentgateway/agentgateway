use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::llm::types::RequestType;
use crate::llm::{AIError, InputFormat, LLMRequest, LLMRequestParams, SimpleChatCompletionMessage};

/// Canonical rerank request, modeled on the Cohere `/v2/rerank` API.
/// Unknown fields are preserved via `rest` for passthrough to compatible providers.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	pub query: String,
	pub documents: Vec<Document>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_n: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub return_documents: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_tokens_per_doc: Option<u32>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

/// A document is either a plain string or a structured object (e.g. `{ "text": ... }`).
/// String semantics are guaranteed across all providers; object/structured-field
/// semantics are best-effort and provider-dependent.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum Document {
	Text(String),
	Object(serde_json::Map<String, serde_json::Value>),
}

impl Document {
	/// Extract the text content. For objects, reads the `text` field (Cohere convention).
	/// Used by the Bedrock/Vertex rerank translation.
	pub fn as_text(&self) -> String {
		match self {
			Document::Text(s) => s.clone(),
			Document::Object(m) => m
				.get("text")
				.and_then(|v| v.as_str())
				.map(|s| s.to_string())
				.unwrap_or_default(),
		}
	}
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub id: Option<String>,
	pub results: Vec<RerankResult>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub meta: Option<Meta>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct RerankResult {
	pub index: u32,
	pub relevance_score: f64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub document: Option<Document>,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct Meta {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub billed_units: Option<BilledUnits>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tokens: Option<Tokens>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

/// Cohere reports token counts as floats.
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct Tokens {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens: Option<f64>,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct BilledUnits {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub search_units: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub total_tokens: Option<u32>,
}

impl RequestType for Request {
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		Ok(LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Rerank,
			native_format: Some(crate::llm::custom::ProviderFormat::Rerank),
			request_model: model,
			provider,
			streaming: false,
			params: LLMRequestParams::default(),
			prompt: Default::default(),
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disabled for rerank.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disabled for rerank.")
	}

	fn to_openai(&self) -> Result<Vec<u8>, AIError> {
		// Passthrough to Cohere-compatible backends (Cohere, Jina, vLLM, Azure Foundry).
		serde_json::to_vec(&self).map_err(AIError::RequestMarshal)
	}

	fn to_bedrock(
		&self,
		provider: &crate::llm::bedrock::Provider,
		_headers: Option<&::http::HeaderMap>,
		_prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		crate::llm::conversion::bedrock::from_rerank::translate(self, provider)
	}

	fn to_vertex(&self, provider: &crate::llm::vertex::Provider) -> Result<Vec<u8>, AIError> {
		crate::llm::conversion::vertex::from_rerank::translate(self, provider)
	}
}

impl crate::llm::types::ResponseType for Response {
	fn to_llm_response(&self, _include_completion_in_log: bool) -> crate::llm::LLMResponse {
		// Cohere reports counts in `meta.tokens`; fall back to `billed_units.total_tokens`
		// (e.g. Voyage's usage normalized into meta).
		let input_tokens = self.meta.as_ref().and_then(|m| {
			m.tokens
				.as_ref()
				.and_then(|t| t.input_tokens)
				.map(|t| t as u64)
				.or_else(|| {
					m.billed_units
						.as_ref()
						.and_then(|b| b.total_tokens)
						.map(|t| t as u64)
				})
		});
		crate::llm::LLMResponse {
			input_tokens,
			total_tokens: input_tokens,
			..Default::default()
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::llm::policy::webhook::ResponseChoice> {
		vec![]
	}

	fn set_webhook_choices(
		&mut self,
		_resp: Vec<crate::llm::policy::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(self)
	}
}

/// Parse a rerank response, accepting either Cohere's `results` or Voyage's `data` key.
pub fn parse_response_lenient(bytes: &[u8]) -> Result<Response, serde_json::Error> {
	// Fast path: the canonical Cohere shape (`results` present) parses in a single pass. This is the
	// common case, so avoid the intermediate `Value` round-trip unless the strict parse fails.
	if let Ok(resp) = serde_json::from_slice::<Response>(bytes) {
		return Ok(resp);
	}
	// Fallback: Voyage returns `data` instead of `results`; rewrite the key and retry.
	let mut v: serde_json::Value = serde_json::from_slice(bytes)?;
	if let Some(obj) = v.as_object_mut()
		&& !obj.contains_key("results")
		&& let Some(data) = obj.remove("data")
	{
		obj.insert("results".to_string(), data);
	}
	serde_json::from_value(v)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cohere_request_roundtrips_and_documents_are_string_or_object() {
		let raw = r#"{
			"model": "rerank-v3.5",
			"query": "capital of the US?",
			"documents": ["Carson City is the capital of Nevada.", {"text": "Washington, D.C."}],
			"top_n": 2,
			"return_documents": false
		}"#;
		let req: Request = serde_json::from_str(raw).unwrap();
		assert_eq!(req.query, "capital of the US?");
		assert_eq!(req.documents.len(), 2);
		assert_eq!(
			req.documents[0].as_text(),
			"Carson City is the capital of Nevada."
		);
		assert_eq!(req.documents[1].as_text(), "Washington, D.C.");
		assert_eq!(req.top_n, Some(2));

		let resp_raw = r#"{"id":"x","results":[{"index":1,"relevance_score":0.99},{"index":0,"relevance_score":0.08}],"meta":{"billed_units":{"search_units":1}}}"#;
		let resp: Response = serde_json::from_str(resp_raw).unwrap();
		assert_eq!(resp.results[0].index, 1);
		assert_eq!(resp.results[0].relevance_score, 0.99);
		// Round-trip serialize keeps the shape
		let back = serde_json::to_string(&resp).unwrap();
		assert!(back.contains("relevance_score"));
	}

	#[test]
	fn passthrough_request_serializes_unknown_fields() {
		// Voyage-style extra field must survive passthrough via `rest`.
		let raw = r#"{"model":"rerank-2","query":"q","documents":["a","b"],"truncation":true}"#;
		let req: Request = serde_json::from_str(raw).unwrap();
		let out = String::from_utf8(req.to_openai().unwrap()).unwrap();
		assert!(out.contains("\"truncation\":true"));
		assert!(out.contains("\"query\":\"q\""));
	}

	#[test]
	fn voyage_response_data_field_parses() {
		// Voyage returns `data`, not `results`.
		let raw =
			r#"{"object":"list","data":[{"index":0,"relevance_score":0.4}],"usage":{"total_tokens":26}}"#;
		let resp = parse_response_lenient(raw.as_bytes()).unwrap();
		assert_eq!(resp.results.len(), 1);
		assert_eq!(resp.results[0].index, 0);
	}

	#[test]
	fn cohere_meta_tokens_populate_usage() {
		use crate::llm::types::ResponseType;
		let raw = r#"{"results":[{"index":0,"relevance_score":0.9}],"meta":{"billed_units":{"search_units":1},"tokens":{"input_tokens":214.0,"output_tokens":2.0}}}"#;
		let resp: Response = serde_json::from_str(raw).unwrap();
		assert_eq!(resp.to_llm_response(false).input_tokens, Some(214));
		// Round-trip keeps the meta shape.
		let back = serde_json::to_string(&resp).unwrap();
		assert!(back.contains("\"input_tokens\":214"));
	}

	#[test]
	fn lenient_parse_prefers_existing_results_over_data() {
		// When both `results` and `data` are present, the existing `results` must win
		// (the `data`->`results` rewrite only fires when `results` is absent).
		let raw = r#"{"results":[{"index":5,"relevance_score":0.5}],"data":[{"index":0,"relevance_score":0.1}]}"#;
		let resp = parse_response_lenient(raw.as_bytes()).unwrap();
		assert_eq!(resp.results.len(), 1);
		assert_eq!(resp.results[0].index, 5);
	}
}
