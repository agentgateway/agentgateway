use super::parse_data_url;

#[test]
fn plain_base64_data_url() {
	assert_eq!(
		parse_data_url("data:application/pdf;base64,JVBERi0xLjQK"),
		Some(("application/pdf", "JVBERi0xLjQK"))
	);
}

#[test]
fn media_type_parameters_are_dropped() {
	// RFC 2397 permits parameters before the encoding marker. Rejecting these used to
	// push callers onto their raw-base64 fallback, sending the header as payload.
	assert_eq!(
		parse_data_url("data:text/plain;charset=utf-8;base64,dGVzdA=="),
		Some(("text/plain", "dGVzdA=="))
	);
}

#[test]
fn empty_media_type_is_preserved() {
	assert_eq!(
		parse_data_url("data:;base64,dGVzdA=="),
		Some(("", "dGVzdA=="))
	);
}

#[test]
fn non_base64_encodings_are_rejected() {
	assert_eq!(parse_data_url("data:image/png,iVBORw0KGgo="), None);
	assert_eq!(parse_data_url("data:text/plain;charset=utf-8,hi"), None);
}

#[test]
fn non_data_urls_are_rejected() {
	assert_eq!(parse_data_url("https://example.com/cat.png"), None);
	assert_eq!(parse_data_url("data:no-comma"), None);
}

#[test]
fn messages_stop_sequences_are_forwarded_as_chat_completions_stop() {
	let request: crate::types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "test-model",
		"max_tokens": 64,
		"stop_sequences": ["STOPPROBE", "DONE"],
		"messages": [{"role": "user", "content": "hello"}]
	}))
	.unwrap();
	let translated = super::from_messages::translate(&request).unwrap();
	let translated: serde_json::Value = serde_json::from_slice(&translated).unwrap();
	assert_eq!(translated["stop"], serde_json::json!(["STOPPROBE", "DONE"]));
}

mod stop_sequence_reporting {
	use super::super::from_messages::{choice_stop_sequence, translate_response_internal};
	use crate::types::completions::typed as completions;
	use crate::types::messages::typed as messages;

	fn raw(choice_extra: &str) -> String {
		format!(
			r#"{{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"m",
			"choices":[{{"index":0,"finish_reason":"stop",
			"message":{{"role":"assistant","content":"A\nB\nC\nD\nE\n"}}{choice_extra}}}],
			"usage":{{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}}"#
		)
	}

	fn translate(body: &str) -> messages::MessagesResponse {
		let typed: completions::Response = serde_json::from_str(body).expect("valid chat completion");
		translate_response_internal(typed).unwrap()
	}

	#[test]
	fn extensions_accept_only_string_values_from_the_known_engine_fields() {
		let seq = |choice_extra: &str| {
			let typed: completions::Response =
				serde_json::from_str(&raw(choice_extra)).expect("valid chat completion");
			choice_stop_sequence(&typed.choices[0].rest)
		};
		// vLLM: a matched stop *string* is a string; a stop *token* is an integer.
		assert_eq!(seq(r#","stop_reason":"F""#), Some("F".into()));
		assert_eq!(seq(r#","stop_reason":128001"#), None);
		// SGLang
		assert_eq!(seq(r#","matched_stop":"END""#), Some("END".into()));
		assert_eq!(seq(r#","matched_stop":154827"#), None);
		// nothing reported or empty
		assert_eq!(seq(""), None);
		assert_eq!(seq(r#","stop_reason":"""#), None);
		assert_eq!(
			seq(r#","stop_reason":"","matched_stop":"END""#),
			Some("END".into())
		);
	}

	#[test]
	fn matched_stop_string_becomes_stop_sequence_with_the_sequence_named() {
		for (choice_extra, want) in [
			(r#","stop_reason":"F""#, "F"),
			(r#","matched_stop":"END""#, "END"),
		] {
			let out = translate(&raw(choice_extra));
			assert_eq!(
				out.stop_reason,
				Some(messages::StopReason::StopSequence),
				"{choice_extra}"
			);
			assert_eq!(out.stop_sequence.as_deref(), Some(want), "{choice_extra}");
		}
	}

	#[test]
	fn natural_end_of_turn_is_unchanged() {
		// No extension field, or a stop *token id* (integer): both stay end_turn.
		for choice_extra in ["", r#","stop_reason":128001"#, r#","matched_stop":154827"#] {
			let out = translate(&raw(choice_extra));
			assert_eq!(
				out.stop_reason,
				Some(messages::StopReason::EndTurn),
				"{choice_extra}"
			);
			assert_eq!(out.stop_sequence, None, "{choice_extra}");
		}
	}

	#[test]
	fn other_finish_reasons_never_report_a_stop_sequence() {
		let body = r#"{"id":"x","object":"chat.completion","created":0,"model":"m",
			"choices":[{"index":0,"finish_reason":"length","stop_reason":"F",
			"message":{"role":"assistant","content":"..."}}],
			"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
		let out = translate(body);
		assert_eq!(out.stop_reason, Some(messages::StopReason::MaxTokens));
		assert_eq!(out.stop_sequence, None);

		let body = r#"{"id":"x","object":"chat.completion","created":0,"model":"m",
			"choices":[{"index":0,"finish_reason":"content_filter","stop_reason":"F",
			"message":{"role":"assistant","content":"..."}}],
			"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
		let out = translate(body);
		assert_eq!(out.stop_reason, Some(messages::StopReason::EndTurn));
		assert_eq!(out.stop_sequence, None);
	}
}

mod stop_sequence_reporting_streaming {
	use super::super::from_messages::stop_sequence_from_fields;
	use crate::types::completions::typed as completions;

	#[test]
	fn stream_chunk_types_keep_engine_extension_fields() {
		// Without `rest` these were dropped at parse time, so the streaming path
		// could never see them.
		let chunk: completions::StreamResponse = serde_json::from_str(
			r#"{"id":"c","object":"chat.completion.chunk","created":0,"model":"m",
			"choices":[{"index":0,"delta":{},"finish_reason":"stop","stop_reason":"F","matched_stop":"F"}]}"#,
		)
		.unwrap();
		assert_eq!(chunk.choices[0].rest["stop_reason"], "F");
		assert_eq!(chunk.choices[0].rest["matched_stop"], "F");
		// ...and a plain chunk serialises exactly as before: no phantom fields.
		let plain: completions::StreamResponse = serde_json::from_str(
			r#"{"id":"c","object":"chat.completion.chunk","created":0,"model":"m",
			"choices":[{"index":0,"delta":{"content":"hi"}}]}"#,
		)
		.unwrap();
		let out = serde_json::to_value(&plain).unwrap();
		assert!(out["choices"][0].get("rest").is_none());
	}

	#[test]
	fn precedence_and_type_rules_are_shared_with_the_buffered_path() {
		let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
		let (a, b, c) = (v(r#""F""#), v("128001"), v(r#""GREEN""#));
		assert_eq!(
			stop_sequence_from_fields([Some(&a), None, None]),
			Some("F".into())
		);
		assert_eq!(
			stop_sequence_from_fields([Some(&b), None, Some(&c)]),
			Some("GREEN".into())
		);
		assert_eq!(stop_sequence_from_fields([Some(&b), None, None]), None);
		assert_eq!(stop_sequence_from_fields([None, None, None]), None);
	}
}
