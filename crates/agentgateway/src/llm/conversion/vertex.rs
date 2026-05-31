use crate::llm::types::ResponseType;
use crate::llm::{AIError, logged_response_parsing, types};

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;

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

		let title = req
			.rest
			.get("title")
			.and_then(|v| v.as_str())
			.map(str::to_string);

		// Vertex natively supports batching via the instances array,
		// so we map each input string to an Instance directly.
		let instances = input
			.into_iter()
			.map(|content| types::vertex::Instance {
				content,
				task_type: Some(task_type.clone()),
				title: title.clone(),
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
			let embeddings = pred.embeddings;
			if let Some(stats) = &embeddings.statistics {
				total_prompt_tokens += stats.token_count;
			}
			data.push(types::embeddings::typed::Embedding {
				object: "embedding".to_string(),
				embedding: embeddings.values,
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
		let openai_resp = json::convert::<_, types::embeddings::Response>(&typed_resp)
			.map_err(AIError::ResponseParsing)?;
		Ok(Box::new(openai_resp))
	}
}
