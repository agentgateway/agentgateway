use agent_core::strng;
use serde_json::json;

use super::*;
use crate::types;

fn provider() -> crate::vertex::Provider {
	crate::vertex::Provider {
		model: None,
		region: Some(strng::new("global")),
		project_id: strng::new("test-project"),
	}
}

fn request(model: &str, input: serde_json::Value) -> types::embeddings::Request {
	types::embeddings::Request {
		model: Some(model.to_string()),
		input,
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	}
}

#[test]
fn test_embeddings_rejects_invalid_input() {
	let bad_inputs = vec![json!(42), json!(["hello", 42])];
	for input in bad_inputs {
		let req = request("text-embedding-004", input);
		assert!(from_embeddings::translate(&req, &provider()).is_err());
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

	let translated =
		from_embeddings::translate_response(&bytes, &provider(), "text-embedding-004").unwrap();
	let resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(resp.usage.unwrap().prompt_tokens, 0);
}

#[test]
fn test_embed_content_rejects_multiple_inputs() {
	let req = request("gemini-embedding-2", json!(["hello", "world"]));
	assert!(from_embeddings::translate(&req, &provider()).is_err());
}

#[test]
fn test_embed_content_accepts_single_element_array() {
	let req = request("gemini-embedding-2", json!(["hello"]));
	let body = from_embeddings::translate(&req, &provider()).unwrap();
	let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

	assert_eq!(body["content"]["parts"], json!([{"text": "hello"}]));
	// Unlike :predict, no task type is invented when the client did not ask for one.
	assert!(body.get("embedContentConfig").is_none());
}

#[test]
fn test_embed_content_passes_through_rest_params() {
	let mut req = request("gemini-embedding-2", json!("hello"));
	req.rest = json!({
		"task_type": "RETRIEVAL_DOCUMENT",
		"title": "My Document",
		"auto_truncate": true,
	});
	let body = from_embeddings::translate(&req, &provider()).unwrap();
	let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

	assert_eq!(
		body["embedContentConfig"],
		json!({
			"taskType": "RETRIEVAL_DOCUMENT",
			"title": "My Document",
			"autoTruncate": true,
		})
	);
}

#[test]
fn test_predict_still_defaults_task_type() {
	let req = request("text-embedding-005", json!("hello"));
	let body = from_embeddings::translate(&req, &provider()).unwrap();
	let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

	assert_eq!(body["instances"][0]["task_type"], json!("RETRIEVAL_QUERY"));
}

#[test]
fn test_embed_content_response_missing_usage_metadata() {
	let vertex_resp = json!({ "embedding": { "values": [0.1, 0.2] } });
	let bytes = serde_json::to_vec(&vertex_resp).unwrap();

	let translated =
		from_embeddings::translate_response(&bytes, &provider(), "gemini-embedding-2").unwrap();
	let resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(resp.usage.unwrap().prompt_tokens, 0);
}
