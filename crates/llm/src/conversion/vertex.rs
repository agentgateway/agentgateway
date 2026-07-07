use crate::llm::types::ResponseType;
use crate::llm::{AIError, logged_response_parsing, types};

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;

pub mod from_rerank {
	use super::*;

	pub fn translate(
		req: &types::rerank::Request,
		_provider: &crate::llm::vertex::Provider,
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
		crate::llm::conversion::completions::translate_google_error(bytes)
	}
}

pub mod from_embeddings {
	use super::*;
	use crate::json;

	pub fn translate(req: &types::embeddings::Request) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, types::embeddings::typed::Request>(req)
			.map_err(AIError::RequestMarshal)?;

		let input = typed.input.as_strings();

		let task_type = req
			.rest
			.get("task_type")
			.and_then(|v| v.as_str())
			.unwrap_or("RETRIEVAL_QUERY")
			.to_string();

		// Vertex natively supports batching via the instances array,
		// so we map each input string to an Instance directly.
		let instances = input
			.into_iter()
			.map(|content| types::vertex::Instance {
				content,
				task_type: Some(task_type.clone()),
				title: req
					.rest
					.get("title")
					.and_then(|v| v.as_str().map(|s| s.to_string())),
			})
			.collect();

		let auto_truncate = req.rest.get("auto_truncate").and_then(|v| v.as_bool());
		let output_dimensionality = typed.dimensions.map(|d| d as u64);

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

	pub fn translate_response(bytes: &[u8], model: &str) -> Result<Box<dyn ResponseType>, AIError> {
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

		let typed_resp = types::embeddings::typed::Response {
			object: "list".to_string(),
			data,
			model: model.to_string(),
			usage: types::embeddings::typed::Usage {
				prompt_tokens: total_prompt_tokens as u32,
				total_tokens: total_prompt_tokens as u32,
			},
		};
		// Convert the normalized internal typed response back to the passthrough-preserving OpenAI format
		let openai_resp = json::convert::<_, types::embeddings::Response>(&typed_resp)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(openai_resp))
	}
}
