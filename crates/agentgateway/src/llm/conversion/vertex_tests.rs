use serde_json::json;

use super::*;
use crate::llm::types;

#[test]
fn test_embeddings_translation_with_all_fields() {
	let req = types::embeddings::Request {
		model: Some("text-embedding-004".to_string()),
		input: json!(["hello", "world"]),
		user: None,
		encoding_format: None,
		dimensions: Some(512),
		rest: json!({
			"task_type": "RETRIEVAL_DOCUMENT",
			"title": "My Document",
			"auto_truncate": true
		}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let raw: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	let instances = raw["instances"].as_array().unwrap();
	assert_eq!(instances.len(), 2);
	assert_eq!(instances[0]["content"], "hello");
	assert_eq!(instances[1]["content"], "world");
	assert_eq!(instances[0]["task_type"], "RETRIEVAL_DOCUMENT");
	assert_eq!(instances[0]["title"], "My Document");

	// Instance fields must be snake_case, parameters must be camelCase
	assert!(
		instances[0].get("taskType").is_none(),
		"instance fields must be snake_case"
	);
	assert_eq!(raw["parameters"]["outputDimensionality"], 512);
	assert_eq!(raw["parameters"]["autoTruncate"], true);
}

#[test]
fn test_embeddings_omits_optional_fields() {
	let req = types::embeddings::Request {
		model: Some("text-embedding-004".to_string()),
		input: json!("hello"),
		user: None,
		encoding_format: None,
		dimensions: None,
		rest: json!({}),
	};

	let translated = from_embeddings::translate(&req).unwrap();
	let raw: serde_json::Value = serde_json::from_slice(&translated).unwrap();

	assert_eq!(raw["instances"][0]["task_type"], "RETRIEVAL_QUERY");
	assert!(raw["instances"][0].get("title").is_none());
	assert!(raw.get("parameters").is_none());
}

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
fn test_embeddings_response_translation() {
	let vertex_resp = json!({
		"predictions": [
			{
				"embeddings": {
					"values": [0.1, 0.2, 0.3],
					"statistics": { "token_count": 3 }
				}
			},
			{
				"embeddings": {
					"values": [0.4, 0.5, 0.6],
					"statistics": { "token_count": 4 }
				}
			}
		]
	});
	let bytes = serde_json::to_vec(&vertex_resp).unwrap();

	let translated = from_embeddings::translate_response(&bytes, "text-embedding-004").unwrap();
	let resp = translated
		.serialize()
		.and_then(|b| serde_json::from_slice::<types::embeddings::Response>(&b))
		.unwrap();

	assert_eq!(resp.object, "list");
	assert_eq!(resp.model, "text-embedding-004");
	assert_eq!(resp.data.len(), 2);
	assert_eq!(resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
	assert_eq!(resp.data[1].embedding, vec![0.4, 0.5, 0.6]);
	assert_eq!(resp.data[0].index, 0);
	assert_eq!(resp.data[1].index, 1);
	assert_eq!(resp.usage.prompt_tokens, 7);
	assert_eq!(resp.usage.total_tokens, 7);
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

	assert_eq!(resp.usage.prompt_tokens, 0);
}
