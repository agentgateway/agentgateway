use serde_json::json;

use super::*;
use crate::types;

#[test]
fn test_embeddings_rejects_invalid_input() {
	let bad_inputs = vec![json!(42), json!(["hello", 42])];
	for input in bad_inputs {
		let req = types::embeddings::Request {
			model: Some("text-embedding-004".to_string()),
			input,
			user: None,
			encoding_format: None,
			dimensions: None,
			rest: json!({}),
		};
		assert!(from_embeddings::translate(&req).is_err());
	}
}

#[test]
fn test_embeddings_response_missing_statistics() {
	let vertex_resp = json!({
		"predictions": [{
			"embeddings": { "values": [0.1, 0.2] }
		}]
	});
	let bytes = serde_json::to_vec(&vertex_resp).unwrap();

	let translated = from_embeddings::translate_response(&bytes, "model").unwrap();
	let resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(resp.usage.unwrap().prompt_tokens, 0);
}
