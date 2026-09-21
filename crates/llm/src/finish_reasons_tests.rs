use std::sync::{Arc, Mutex};

use agent_http::Body;
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::json;

use super::*;
use crate::{
	InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields, ResponseType,
	StreamingUsageReporter, conversion,
};

struct Reporter(Arc<Mutex<LLMInfo>>);
impl StreamingUsageReporter for Reporter {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		f(&mut self.0.lock().unwrap());
	}
	fn report_usage(&mut self) {}
}
fn reporter() -> (StreamingUsageGuard, Arc<Mutex<LLMInfo>>) {
	let info = Arc::new(Mutex::new(LLMInfo::new(
		LLMRequest {
			input_tokens: None,
			input_format: InputFormat::Detect,
			cache_convention: crate::CacheTokenConvention::pending(),
			request_model: "test".into(),
			provider: "test".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		LLMResponse::default(),
	)));
	(
		StreamingUsageGuard::new(Box::new(Reporter(info.clone()))),
		info,
	)
}
fn reasons(info: &Arc<Mutex<LLMInfo>>) -> Option<Vec<Strng>> {
	info.lock().unwrap().response.finish_reasons.clone()
}
fn expected(values: &[&str]) -> Option<Vec<Strng>> {
	Some(values.iter().map(strng::new).collect())
}
fn sse(values: &[Value]) -> String {
	values
		.iter()
		.map(|v| {
			let mut v = v.clone();
			if v.get("choices").is_some() {
				v["id"] = "test".into();
				v["model"] = "test".into();
				for choice in v["choices"].as_array_mut().unwrap() {
					if choice.get("delta").is_none() {
						choice["delta"] = json!({});
					}
				}
			}
			format!("data: {v}\n\n")
		})
		.collect()
}
fn completions(body: Body, log: StreamingUsageGuard) -> Body {
	conversion::completions::passthrough_stream(
		log,
		LogContentFields::default(),
		http::Response::new(body),
	)
	.into_body()
}

#[test]
fn finalization_keeps_slots_and_first_terminal_reason() {
	let (log, info) = reporter();
	log.record_finish_reason(8, None);
	log.record_finish_reason(3, Some("stop".into()));
	log.record_finish_reason(1, Some("stop".into()));
	log.record_finish_reason(3, None);
	log.record_finish_reason(3, Some("length".into()));
	assert_eq!(reasons(&info), None);
	drop(log);
	assert_eq!(reasons(&info), expected(&["stop", "stop", "error"]));
	let (log, info) = reporter();
	drop(log);
	assert_eq!(reasons(&info), None);
}

#[tokio::test]
async fn completions_interleaved_reasons_survive_usage_and_repeated_terminals() {
	let input = sse(&[
		json!({"choices":[{"index":1,"delta":{"content":"B"}},{"index":0,"delta":{"content":"A"}}]}),
		json!({"choices":[{"index":1,"finish_reason":"length"}]}),
		json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
		json!({"choices":[{"index":1,"finish_reason":"length"}]}),
		json!({"choices":[],"model":"test","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}),
	]);
	let (log, info) = reporter();
	let output = completions(Body::from(input.clone()), log)
		.collect()
		.await
		.unwrap()
		.to_bytes();
	assert_eq!(output, input);
	assert_eq!(reasons(&info), expected(&["stop", "length"]));
	assert_eq!(info.lock().unwrap().response.total_tokens, Some(3));
	assert!(info.lock().unwrap().response.completion.is_none());
	assert!(info.lock().unwrap().response.output_messages.is_none());
}

#[tokio::test]
async fn disconnect_and_body_error_finalize_pending_choices() {
	for fail in [false, true] {
		let (log, info) = reporter();
		let input = sse(&[
			json!({"choices":[{"index":0,"finish_reason":"stop"},{"index":1,"delta":{"content":"partial"}}]}),
		]);
		let body = Body::from_stream(futures_util::stream::iter([
			Ok(Bytes::from(input)),
			Err(std::io::Error::other("upstream disconnected")),
		]));
		let mut body = completions(body, log);
		body.frame().await.unwrap().unwrap();
		if fail {
			assert!(body.frame().await.unwrap().is_err());
		}
		drop(body);
		assert_eq!(reasons(&info), expected(&["stop", "error"]));
	}
}

#[tokio::test]
async fn gemini_indexes_and_missing_reasons() {
	let input = sse(&[
		json!({"candidates":[{"index":4,"content":{"parts":[{"text":"partial"}]}},{"index":2,"finishReason":"MAX_TOKENS"}]}),
		json!({"candidates":[{"index":0,"finishReason":"STOP"},{"index":2,"finishReason":"MAX_TOKENS"}]}),
		json!({"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2}}),
	]);
	for detect in [false, true] {
		let (log, info) = reporter();
		let body = Body::from(input.clone());
		let body = if detect {
			types::detect::passthrough_stream(log, http::Response::new(body)).into_body()
		} else {
			conversion::vertex_gemini::passthrough_stream(
				body,
				1024 * 1024,
				log,
				LogContentFields::default(),
			)
		};
		body.collect().await.unwrap();
		assert_eq!(reasons(&info), expected(&["STOP", "MAX_TOKENS", "error"]));
	}
}

#[tokio::test]
async fn responses_statuses_and_cancellation() {
	for (status, want) in [
		("completed", "completed"),
		("incomplete", "incomplete"),
		("failed", "error"),
		("cancelled", "error"),
		("in_progress", "error"),
		("queued", "error"),
	] {
		let input = sse(&[
			json!({"type":"response.created","response":{"status":"in_progress","object":"response","output":[]}}),
			json!({"type":format!("response.{status}"),"response":{"status":status,"object":"response","output":[]}}),
			json!({"type":format!("response.{status}"),"response":{"status":status,"object":"response","output":[]}}),
		]);
		for detect in [false, true] {
			let (log, info) = reporter();
			let body = Body::from(input.clone());
			let body = if detect {
				types::detect::passthrough_stream(log, http::Response::new(body)).into_body()
			} else {
				conversion::responses::passthrough_stream(
					body,
					1024 * 1024,
					log,
					LogContentFields::default(),
				)
			};
			body.collect().await.unwrap();
			assert_eq!(reasons(&info), expected(&[want]), "{status}");
		}
	}
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
