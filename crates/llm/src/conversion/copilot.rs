pub mod from_embeddings {
	use serde_json::Value;

	use crate::types::ResponseType;
	use crate::{AIError, logged_response_parsing, types};

	pub fn translate(req: &types::embeddings::Request) -> Result<Vec<u8>, AIError> {
		if let Value::String(s) = &req.input {
			let mut normalized = req.clone();
			normalized.input = Value::Array(vec![Value::String(s.clone())]);
			serde_json::to_vec(&normalized).map_err(AIError::RequestMarshal)
		} else {
			serde_json::to_vec(req).map_err(AIError::RequestMarshal)
		}
	}

	pub fn translate_response(bytes: &[u8], model: &str) -> Result<Box<dyn ResponseType>, AIError> {
		let mut resp: types::embeddings::Response =
			serde_json::from_slice(bytes).map_err(logged_response_parsing(bytes))?;
		if resp.model.is_empty() {
			resp.model = model.to_string();
		}
		Ok(Box::new(resp))
	}
}
