use serde_json::json;

use super::*;
use crate::{LogContentFields, ResponseType, types};

fn expected(values: &[&str]) -> Option<Vec<Strng>> {
	Some(values.iter().map(strng::new).collect())
}

#[test]
fn buffered_order_missing_reasons_and_non_generation_endpoints() {
	let value = json!({"model":"test","choices":[{"index":2,"finish_reason":"length"},{"index":0,"finish_reason":"stop"},{"index":1}]});
	let response: types::completions::Response = serde_json::from_value(value.clone()).unwrap();
	assert_eq!(
		response
			.to_llm_response(LogContentFields::default())
			.finish_reasons,
		expected(&["length", "stop", "error"])
	);
	assert_eq!(
		detect_buffered(&value),
		expected(&["length", "stop", "error"])
	);
	for value in [
		json!({"data":[{"embedding":[0.1]}]}),
		json!({"totalTokens":23}),
		json!({"results":[]}),
		json!({"usage":{"total_tokens":3}}),
	] {
		assert_eq!(detect_buffered(&value), None);
	}
}

#[test]
fn buffered_responses_terminal_statuses() {
	for (status, want) in [
		("completed", "completed"),
		("incomplete", "incomplete"),
		("failed", "error"),
		("cancelled", "error"),
	] {
		let mut value: Value =
			serde_json::from_str(include_str!("tests/response/responses/basic.json")).unwrap();
		value["status"] = status.into();
		let response: types::responses::Response = serde_json::from_value(value.clone()).unwrap();
		assert_eq!(
			response
				.to_llm_response(LogContentFields::default())
				.finish_reasons,
			expected(&[want])
		);
		assert_eq!(detect_buffered(&value), expected(&[want]));
	}
}

#[test]
fn buffered_background_responses_have_no_finish_reasons() {
	for status in ["queued", "in_progress"] {
		let value = json!({
			"id": "resp_background",
			"object": "response",
			"model": "test",
			"background": true,
			"status": status,
			"output": [],
			"usage": null,
		});
		let response: types::responses::Response = serde_json::from_value(value.clone()).unwrap();
		for response in [
			response.to_llm_response(LogContentFields::default()),
			types::detect::Response::Json(value).to_llm_response(LogContentFields::default()),
		] {
			assert_eq!(response.finish_reasons, None, "{status}");
			assert!(
				serde_json::to_value(response)
					.unwrap()
					.get("finish_reasons")
					.is_none()
			);
		}
	}
}
