use crate::types::ResponseType;
use crate::{AIError, logged_response_parsing, types};

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;

pub mod from_rerank {
	use super::*;

	pub fn translate(
		req: &types::rerank::Request,
		_provider: &crate::vertex::Provider,
	) -> Result<Vec<u8>, AIError> {
		if req.documents.is_empty() {
			return Err(AIError::MissingField("rerank documents".into()));
		}
		let model = req.model.clone().unwrap_or_default();
		let records = req
			.documents
			.iter()
			.enumerate()
			.map(|(idx, d)| types::vertex::RankRecord {
				// Numeric id = original position, so the response can be inverted back to the index.
				id: idx.to_string(),
				content: d.as_text(),
			})
			.collect();
		let vertex_req = types::vertex::RankRequest {
			model,
			query: req.query.clone(),
			records,
			top_n: req.top_n,
			// Cohere `return_documents` -> Vertex inverse `ignoreRecordDetailsInResponse`.
			ignore_record_details_in_response: !req.return_documents.unwrap_or(false),
		};
		serde_json::to_vec(&vertex_req).map_err(AIError::RequestMarshal)
	}

	/// Discovery Engine returns synthetic ids + scores in rank order; it does not echo document text.
	pub fn translate_response(bytes: &[u8]) -> Result<Box<dyn ResponseType>, AIError> {
		let resp: types::vertex::RankResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		let results: Vec<types::rerank::RerankResult> = resp
			.records
			.into_iter()
			.map(|r| {
				// Invert the synthetic id back to the original document index; a wrong mapping here
				// attaches scores to the wrong documents.
				let index = r.id.parse::<u32>().map_err(|_| {
					AIError::ResponseParsing(serde::de::Error::custom(format!(
						"vertex rerank returned non-numeric record id: {}",
						r.id
					)))
				})?;
				Ok(types::rerank::RerankResult {
					index,
					// Vertex omits score when details are suppressed; default to 1.0.
					relevance_score: r.score.unwrap_or(1.0),
					document: None,
				})
			})
			.collect::<Result<_, AIError>>()?;
		let out = types::rerank::Response {
			id: None,
			results,
			meta: None,
			rest: serde_json::Value::Null,
		};
		Ok(Box::new(out))
	}

	pub fn translate_error(bytes: &bytes::Bytes) -> Result<bytes::Bytes, AIError> {
		// Reuse the Google error normalizer used by completions.
		crate::conversion::completions::translate_google_error(bytes)
	}
}

pub mod from_embeddings {
	use serde_json::Value;

	use super::*;
	use crate::json;
	use crate::types::vertex_gemini as vg;

	/// Vertex embedding knobs, resolved once and shared by both endpoints. Everything except
	/// `output_dimensionality` has no OpenAI equivalent and arrives via the passthrough `rest`.
	struct Params {
		task_type: Option<String>,
		title: Option<String>,
		auto_truncate: Option<bool>,
		output_dimensionality: Option<u64>,
	}

	impl Params {
		fn extract(
			req: &types::embeddings::Request,
			typed: &types::embeddings::typed::Request,
		) -> Self {
			Self {
				task_type: req
					.rest
					.get("task_type")
					.and_then(|v| v.as_str().map(|s| s.to_string())),
				title: req
					.rest
					.get("title")
					.and_then(|v| v.as_str().map(|s| s.to_string())),
				auto_truncate: req.rest.get("auto_truncate").and_then(|v| v.as_bool()),
				output_dimensionality: typed.dimensions.map(|d| d as u64),
			}
		}
	}

	pub fn translate(
		req: &types::embeddings::Request,
		provider: &crate::vertex::Provider,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, types::embeddings::typed::Request>(req)
			.map_err(AIError::RequestMarshal)?;
		let params = Params::extract(req, &typed);

		if provider.uses_embed_content(req.model.as_deref()) {
			translate_embed_content_request(&typed, params)
		} else {
			translate_predict_request(&typed, params)
		}
	}

	fn translate_predict_request(
		typed: &types::embeddings::typed::Request,
		params: Params,
	) -> Result<Vec<u8>, AIError> {
		let Params {
			task_type,
			title,
			auto_truncate,
			output_dimensionality,
		} = params;
		let task_type = task_type.unwrap_or_else(|| "RETRIEVAL_QUERY".to_string());

		// Vertex natively supports batching via the instances array,
		// so we map each input string to an Instance directly.
		let instances = typed
			.input
			.as_strings()
			.into_iter()
			.map(|content| types::vertex::Instance {
				content,
				task_type: Some(task_type.clone()),
				title: title.clone(),
			})
			.collect();

		let parameters = if auto_truncate.is_some() || output_dimensionality.is_some() {
			Some(types::vertex::Parameters {
				auto_truncate,
				output_dimensionality,
			})
		} else {
			None
		};

		let vertex_req = types::vertex::PredictRequest {
			instances,
			parameters,
		};
		serde_json::to_vec(&vertex_req).map_err(AIError::RequestMarshal)
	}

	fn translate_embed_content_request(
		typed: &types::embeddings::typed::Request,
		params: Params,
	) -> Result<Vec<u8>, AIError> {
		// embedContent embeds one content and returns one embedding. Vertex has no batch
		// variant, and extra parts would silently collapse into a single vector.
		let mut inputs = typed.input.as_strings();
		if inputs.len() != 1 {
			return Err(AIError::RequestParsing(serde::de::Error::custom(
				"Vertex embedContent does not support batching; `input` must contain exactly one string",
			)));
		}

		let Params {
			task_type,
			title,
			auto_truncate,
			output_dimensionality,
		} = params;

		let embed_content_config = if task_type.is_some()
			|| title.is_some()
			|| output_dimensionality.is_some()
			|| auto_truncate.is_some()
		{
			Some(vg::EmbedContentConfig {
				task_type,
				title,
				output_dimensionality,
				auto_truncate,
			})
		} else {
			None
		};

		let vertex_req = vg::EmbedContentRequest {
			content: vg::Content {
				role: None,
				parts: vec![vg::Part::Text(vg::TextPart {
					text: inputs.remove(0),
					thought: None,
					thought_signature: None,
					rest: Value::Null,
				})],
				rest: Value::Null,
			},
			embed_content_config,
		};
		serde_json::to_vec(&vertex_req).map_err(AIError::RequestMarshal)
	}

	pub fn translate_response(
		bytes: &[u8],
		provider: &crate::vertex::Provider,
		model: &str,
	) -> Result<Box<dyn ResponseType>, AIError> {
		let (data, prompt_tokens) = if provider.uses_embed_content(Some(model)) {
			translate_embed_content_response(bytes)?
		} else {
			translate_predict_response(bytes)?
		};

		let typed_resp = types::embeddings::typed::Response {
			object: "list".to_string(),
			data,
			model: model.to_string(),
			usage: types::embeddings::typed::Usage {
				prompt_tokens: prompt_tokens as u32,
				total_tokens: prompt_tokens as u32,
			},
		};
		// Convert the normalized internal typed response back to the passthrough-preserving OpenAI format
		let openai_resp = json::convert::<_, types::embeddings::Response>(&typed_resp)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(openai_resp))
	}

	fn translate_predict_response(
		bytes: &[u8],
	) -> Result<(Vec<types::embeddings::typed::Embedding>, u64), AIError> {
		let resp: types::vertex::PredictResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;

		let mut total_prompt_tokens = 0;
		let mut data = Vec::new();

		for (i, pred) in resp.predictions.into_iter().enumerate() {
			let mut embeddings = pred.embeddings;
			if let Some(stats) = &embeddings.statistics {
				total_prompt_tokens += stats.token_count;
			}
			data.push(types::embeddings::typed::Embedding {
				object: "embedding".to_string(),
				// Zero-clone optimization: Move the large vector out of the response body
				// to avoid expensive re-allocations during translation.
				embedding: std::mem::take(&mut embeddings.values),
				index: i as u32,
			});
		}
		Ok((data, total_prompt_tokens))
	}

	fn translate_embed_content_response(
		bytes: &[u8],
	) -> Result<(Vec<types::embeddings::typed::Embedding>, u64), AIError> {
		let mut resp: vg::EmbedContentResponse =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;

		let prompt_tokens = resp
			.usage_metadata
			.as_ref()
			.and_then(|u| u.prompt_token_count.or(u.total_token_count))
			.unwrap_or_default();

		let data = vec![types::embeddings::typed::Embedding {
			object: "embedding".to_string(),
			embedding: std::mem::take(&mut resp.embedding.values),
			index: 0,
		}];
		Ok((data, prompt_tokens))
	}
}
