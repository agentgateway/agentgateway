use serde_json::json;

use super::*;

#[test]
fn confidence_uses_typesafe_formulas() {
	assert!((choice_confidence(&[0.7, 0.3]) - 0.4).abs() < 1e-10);
	assert!((score_confidence(&[0.1, 0.7, 0.2]) - 0.55).abs() < 1e-10);
	assert_eq!(score_confidence(&[0.5, 0.0, 0.5]), 0.0);
	// Score probabilities follow level order, independent of the provider's map order.
	let score = IndexMap::from([("2".into(), 0.2), ("0".into(), 0.1), ("1".into(), 0.7)]);
	assert_eq!(level_probabilities(&score, 3), [0.1, 0.7, 0.2]);
}

#[test]
fn missing_usage_is_rejected() {
	let req = serde_json::from_value(json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "noul", "instructions": "yes?"}}})).unwrap();
	let (_, state) = from_systemone::translate(&req).unwrap();
	let missing_usage = json!({"answers": {"q_0": {"type": "noul", "noul": 0.5}}});
	assert!(matches!(
		from_systemone::translate_response(&serde_json::to_vec(&missing_usage).unwrap(), &state),
		Err(AIError::ResponseParsing(_))
	));
}

#[test]
fn invalid_requests_are_rejected() {
	for request in [
		json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {"only": null}}}}),
		json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "score", "instructions": "rate", "criteria": ["only"]}}}),
	] {
		let request = serde_json::from_value(request).unwrap();
		assert!(matches!(
			from_systemone::translate(&request),
			Err(AIError::UnsupportedConversion(_))
		));
	}

	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": "text",
		"questions": [{"type": "choice", "instructions": "pick", "choices": [{"value": "only"}]}]
	}))
	.unwrap();
	assert!(matches!(
		from_decisions::translate(&request),
		Err(AIError::UnsupportedConversion(_))
	));
}

#[test]
fn gemini_request_limits_are_left_to_gemini() {
	let many_choices = (0..256)
		.map(|i| json!({"value": i.to_string()}))
		.collect::<Vec<_>>();
	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": "text",
		"questions": [
			{"type": "choice", "instructions": "pick", "choices": many_choices},
			{"type": "choice", "instructions": "pick", "choices": [{"value": "a"}, {"value": "a"}]},
			{"type": "score", "instructions": "rate", "levels": [{"label": ""}, {"label": " "}]}
		]
	}))
	.unwrap();
	assert!(from_decisions::translate(&request).is_ok());

	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"questions": {}
	}))
	.unwrap();
	assert!(from_systemone::translate(&request).is_ok());
}

#[test]
fn unknown_request_fields_do_not_block_conversion() {
	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"temperature": 0,
		"questions": {"q": {
			"type": "noul",
			"instructions": "yes?",
			"foo": true,
			"criteria": {"true": "yes", "foo": true}
		}}
	}))
	.unwrap();
	assert!(from_systemone::translate(&request).is_ok());

	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": [{
			"type": "message",
			"role": "user",
			"foo": true,
			"content": [{"type": "input_text", "text": "text", "foo": true}]
		}],
		"questions": [{"type": "predicate", "instructions": "yes?", "foo": true}],
		"temperature": 0
	}))
	.unwrap();
	assert!(from_decisions::translate(&request).is_ok());
}

#[test]
fn unknown_response_fields_do_not_block_conversion() {
	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"questions": {"q": {
			"type": "choice",
			"instructions": "pick",
			"criteria": {"a": null, "b": null}
		}}
	}))
	.unwrap();
	let (_, state) = from_systemone::translate(&request).unwrap();
	let response = json!({
		"answers": {
			"q_0": {
				"type": "choice",
				"choice": "o_000",
				"probabilities": {"o_000": 0.6, "o_001": 0.4},
				"confidence": 0.8,
				"future_field": true
			}
		},
		"usage": {"total_input_tokens": 1, "total_output_tokens": 0, "future_field": true},
		"future_field": true
	});
	let translated: Value = serde_json::from_slice(
		&from_systemone::translate_response(&serde_json::to_vec(&response).unwrap(), &state).unwrap(),
	)
	.unwrap();
	assert_eq!(translated["answers"]["q"]["choice"], "a");
	assert_eq!(
		translated["answers"]["q"]["probabilities"],
		json!({"a": 0.6, "b": 0.4})
	);
	assert!((translated["answers"]["q"]["confidence"].as_f64().unwrap() - 0.2).abs() < 1e-10);
}

#[test]
fn imperfect_answers_are_converted_best_effort() {
	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"questions": {
			"rounded": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}},
			"sparse": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}},
			"unknown_choice": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}},
			"wrong_type": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}},
			"unanswered": {"type": "noul", "instructions": "yes?"},
			"score": {"type": "score", "instructions": "rate", "criteria": ["low", "mid", "high"]}
		}
	}))
	.unwrap();
	let (_, state) = from_systemone::translate(&request).unwrap();
	let response = json!({
		"answers": {
			// Rounded probabilities, no confidence.
			"q_0": {"type": "choice", "choice": "o_000", "probabilities": {"o_000": 0.6, "o_001": 0.37}},
			// A missing option and an unknown one, with an out-of-range confidence.
			"q_1": {"type": "choice", "choice": "o_001", "probabilities": {"o_001": 0.9, "extra": 0.1}, "confidence": 1.5},
			"q_2": {"type": "choice", "choice": "unknown", "probabilities": {"o_000": 0.5, "o_001": 0.5}, "confidence": 0.5},
			"q_3": {"type": "noul", "noul": 0.5},
			// An out-of-range score and a missing level.
			"q_5": {"type": "score", "score": 7, "probabilities": {"0": 0.2, "2": 0.8}, "confidence": -0.1},
			"extra": {"type": "noul", "noul": 0.5}
		},
		"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
	});
	let translated: Value = serde_json::from_slice(
		&from_systemone::translate_response(&serde_json::to_vec(&response).unwrap(), &state).unwrap(),
	)
	.unwrap();
	let answers = &translated["answers"];
	assert_eq!(
		answers.as_object().unwrap().keys().collect::<Vec<_>>(),
		["rounded", "sparse", "score"]
	);
	assert_eq!(answers["rounded"]["choice"], "a");
	assert_eq!(
		answers["rounded"]["probabilities"],
		json!({"a": 0.6, "b": 0.37})
	);
	assert_eq!(answers["sparse"]["choice"], "b");
	assert_eq!(
		answers["sparse"]["probabilities"],
		json!({"a": 0.0, "b": 0.9})
	);
	assert!((answers["sparse"]["confidence"].as_f64().unwrap() - 0.8).abs() < 1e-10);
	assert_eq!(answers["score"]["score"], 7);
	assert_eq!(
		answers["score"]["probabilities"],
		json!({"0": 0.2, "1": 0.0, "2": 0.8})
	);

	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": "text",
		"questions": [
			{"type": "choice", "instructions": "pick", "choices": [{"value": "a"}, {"value": true}]},
			{"type": "score", "instructions": "rate", "levels": [{"label": "low"}, {"label": "high"}]}
		]
	}))
	.unwrap();
	let (_, state) = from_decisions::translate(&request).unwrap();
	let response = json!({
		"answers": {
			"q_0": {"type": "choice", "choice": "o_001", "probabilities": {"o_001": 0.97}},
			"q_1": {"type": "score", "score": 7, "probabilities": {"1": 1.2}, "confidence": -0.1}
		},
		"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
	});
	let translated: Value = serde_json::from_slice(
		&from_decisions::translate_response(&serde_json::to_vec(&response).unwrap(), &state).unwrap(),
	)
	.unwrap();
	let answers = &translated["answers"];
	assert_eq!(answers[0]["choice"], true);
	assert_eq!(
		answers[0]["probabilities"],
		json!([{"value": "a", "probability": 0.0}, {"value": true, "probability": 0.97}])
	);
	assert_eq!(answers[1]["score"], 7.0);
	assert_eq!(
		answers[1]["probabilities"],
		json!([
			{"value": 0, "label": "low", "probability": 0.0},
			{"value": 1, "label": "high", "probability": 1.2}
		])
	);
}

#[test]
fn confidence_uses_compatible_formulas_for_both_clients() {
	let systemone_request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}}}
	}))
	.unwrap();
	let (_, systemone_state) = from_systemone::translate(&systemone_request).unwrap();
	let decisions_request = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": "text",
			"questions": [{"type": "choice", "instructions": "pick", "choices": [{"value": "a"}, {"value": "b"}]}]
		}))
		.unwrap();
	let (_, decisions_state) = from_decisions::translate(&decisions_request).unwrap();
	let response = serde_json::to_vec(&json!({
			"answers": {"q_0": {"type": "choice", "choice": "o_000", "probabilities": {"o_000": 0.7, "o_001": 0.3}, "confidence": 0.9}},
			"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
		}))
		.unwrap();

	let systemone: Value = serde_json::from_slice(
		&from_systemone::translate_response(&response, &systemone_state).unwrap(),
	)
	.unwrap();
	let decisions: Value = serde_json::from_slice(
		&from_decisions::translate_response(&response, &decisions_state).unwrap(),
	)
	.unwrap();
	assert!((systemone["answers"]["q"]["confidence"].as_f64().unwrap() - 0.4).abs() < 1e-10);
	assert!((decisions["answers"][0]["confidence"].as_f64().unwrap() - 0.4).abs() < 1e-10);
}

#[test]
fn google_errors_are_translated_to_client_shapes() {
	let numeric = bytes::Bytes::from_static(
		br#"{"error":{"code":401,"message":"bad key","status":"UNAUTHENTICATED"}}"#,
	);
	let openai: Value = serde_json::from_slice(
		&translate_error_to_decisions(&numeric, http::StatusCode::UNAUTHORIZED).unwrap(),
	)
	.unwrap();
	assert_eq!(
		openai,
		json!({"error": {"type": "authentication_error", "message": "bad key", "code": "UNAUTHENTICATED"}})
	);

	let string = bytes::Bytes::from_static(br#"{"error":{"code":"safety","message":"blocked"}}"#);
	let systemone: Value =
		serde_json::from_slice(&translate_error_to_systemone(&string).unwrap()).unwrap();
	assert_eq!(
		systemone,
		json!({"detail": {"code": "safety", "message": "blocked"}})
	);

	let array_wrapped =
		bytes::Bytes::from_static(br#"[{"error":{"code":"safety","message":"blocked"}}]"#);
	let openai: Value = serde_json::from_slice(
		&translate_error_to_decisions(&array_wrapped, http::StatusCode::BAD_REQUEST).unwrap(),
	)
	.unwrap();
	assert_eq!(
		openai,
		json!({"error": {"type": "invalid_request_error", "message": "blocked", "code": "safety"}})
	);
}

#[test]
fn unsupported_media_is_rejected() {
	let req = serde_json::from_value(json!({"model": "gemini-decision-1", "state": "{image}", "assets": {"image": "data:image/png;base64,AA=="}, "questions": {}})).unwrap();
	assert!(matches!(
		from_systemone::translate(&req),
		Err(AIError::UnsupportedContent)
	));
	let req = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": [{"role": "user", "content": [{"type": "input_image", "image_url": "ftp://example.com/image.png"}]}],
			"questions": [{"type": "predicate", "instructions": "Is this a diagram?"}]
		}))
		.unwrap();
	let error = from_decisions::translate(&req).unwrap_err();
	assert!(matches!(error, AIError::UnsupportedContent), "{error:?}");
}

#[test]
fn decisions_public_image_urls_are_translated_to_typed_content() {
	let req: decisions::Request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": [{"role": "user", "content": [{
			"type": "input_image",
			"image_url": "https://example.com/image.png",
			"detail": "low"
		}]}],
		"questions": [{"type": "predicate", "instructions": "Is this a diagram?"}]
	}))
	.unwrap();
	let (body, _) = from_decisions::translate(&req).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		body["input"],
		json!([{
			"type": "user_input",
			"content": [{
				"type": "image",
				"uri": "https://example.com/image.png",
				"resolution": "low"
			}]
		}])
	);
}

#[test]
fn decisions_inline_images_are_translated_to_typed_content() {
	let req: decisions::Request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": [{"role": "user", "content": [
			{"type": "input_text", "text": "What is shown?"},
			{"type": "input_image", "image_url": "data:image/png;base64,aGk=", "detail": "high"}
		]}],
		"questions": [{"type": "predicate", "instructions": "Is this a diagram?"}]
	}))
	.unwrap();
	let (body, _) = from_decisions::translate(&req).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		body["input"],
		json!([{
			"type": "user_input",
			"content": [
				{"type": "text", "text": "What is shown?"},
				{"type": "image", "data": "aGk=", "mime_type": "image/png", "resolution": "high"}
			]
		}])
	);
}

#[test]
fn decisions_image_media_types_are_forwarded_to_gemini() {
	let req: decisions::Request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": [{"role": "user", "content": [
			{"type": "input_image", "image_url": "data:image/avif;base64,aGk="},
			{"type": "input_image", "image_url": "data:application/pdf;base64,aGk=", "detail": "original"}
		]}],
		"questions": [{"type": "predicate", "instructions": "Is this a diagram?"}]
	}))
	.unwrap();
	let (body, _) = from_decisions::translate(&req).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		body["input"][0]["content"],
		json!([
			{"type": "image", "data": "aGk=", "mime_type": "image/avif"},
			{"type": "image", "data": "aGk=", "mime_type": "application/pdf", "resolution": "high"}
		])
	);
}

#[test]
fn decisions_prompt_cache_fields_are_dropped() {
	let request = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": [{"role": "user", "content": [{
			"type": "input_text",
			"text": "text",
			"prompt_cache_breakpoint": {"mode": "explicit"}
		}]}],
		"questions": [{"type": "predicate", "instructions": "yes?"}],
		"prompt_cache_key": "tenant-a",
		"prompt_cache_options": {"mode": "explicit", "ttl": "30m"}
	}))
	.unwrap();
	let (body, _) = from_decisions::translate(&request).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(
		body,
		json!({
			"model": "gemini-decision-1",
			"input": [{"type": "user_input", "content": [{"type": "text", "text": "text"}]}],
			"questions": {"q_0": {"type": "noul", "instructions": "yes?"}}
		})
	);
}

#[test]
fn blank_choice_descriptions_use_the_value() {
	let req = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"state": "text",
		"questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {
			"first": "", "second": "   "
		}}}
	}))
	.unwrap();
	let (body, _) = from_systemone::translate(&req).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(body["questions"]["q_0"]["criteria"]["o_000"], "first");
	assert_eq!(body["questions"]["q_0"]["criteria"]["o_001"], "second");

	let req = serde_json::from_value(json!({
		"model": "gemini-decision-1",
		"input": "text",
		"questions": [{"type": "choice", "instructions": "pick", "choices": [
			{"value": "first", "description": ""},
			{"value": true, "description": "   "}
		]}]
	}))
	.unwrap();
	let (body, _) = from_decisions::translate(&req).unwrap();
	let body: Value = serde_json::from_slice(&body).unwrap();
	assert_eq!(body["questions"]["q_0"]["criteria"]["o_000"], "first");
	assert_eq!(body["questions"]["q_0"]["criteria"]["o_001"], "true");
}
