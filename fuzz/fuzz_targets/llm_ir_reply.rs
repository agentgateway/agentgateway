#![no_main]

//! Malformed provider input must not panic or produce broken client JSON/SSE.

#[path = "llm_ir_common.rs"]
mod common;

use agent_llm_ir::{
	BufferedReply, ClientProtocol, IncompleteCall, OutputChecks, ReplyContext, ReplyProgress,
	StreamReply, UpstreamProtocol, Verdict,
};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
	if data.len() > 64 * 1024 {
		return;
	}
	let Some((limit, chunk, input)) = common::split_config(data) else {
		return;
	};
	let protocols = [
		(ClientProtocol::Messages, UpstreamProtocol::Messages),
		(
			ClientProtocol::Responses,
			UpstreamProtocol::Responses {
				safety_identifier: false,
				reasoning_context: false,
			},
		),
		(
			ClientProtocol::ChatCompletions,
			UpstreamProtocol::ChatCompletions,
		),
		(ClientProtocol::Gemini, UpstreamProtocol::Gemini),
	];
	for (provider, &(_, upstream)) in protocols.iter().enumerate() {
		for (target, &(client, _)) in protocols.iter().enumerate() {
			if provider == target {
				continue;
			}
			let context = || ReplyContext {
				client,
				upstream,
				scope: common::SCOPE,
				routes: &[],
				show_reasoning: true,
				include_usage: true,
				created: 1,
				fallback_reply_id: "fuzz-reply",
				stop_sequences: Vec::new(),
				max_held_bytes: limit.min(64 * 1024),
			};
			for checks in [None, Some(checks(limit, chunk))] {
				let mut buffered = BufferedReply::new(context());
				if let Some(checks) = checks.clone() {
					buffered.set_checks(checks);
				}
				let _ = buffered.read(input);
				answer(buffered.progress());
				let body = buffered.into_reply().body;
				common::assert_json_body(&body);
				if client == ClientProtocol::Gemini {
					assert_signatures(&serde_json::from_slice(&body).unwrap());
				}

				let mut stream = StreamReply::new(context(), limit);
				if let Some(checks) = checks {
					stream.set_checks(checks);
				}
				let mut sent = Vec::new();
				for piece in input.chunks(chunk) {
					// Decode errors mark the reply as failed; keep reading for later usage.
					let _ = stream.push(piece);
					answer(stream.progress());
					sent.extend(stream.take_output().unwrap_or_default());
				}
				let _ = stream.finish();
				answer(stream.progress());
				sent.extend(stream.take_output().unwrap_or_default());
				if let Some(reply) = stream.early_failure() {
					assert!(sent.is_empty(), "an early failure after output");
					common::assert_json_body(&reply.body);
				} else {
					assert_events(client, &sent);
				}
			}
		}
	}
});

/// Small windows, so short replies are offered for checking several times.
fn checks(limit: usize, chunk: usize) -> OutputChecks {
	OutputChecks {
		window_bytes: 8,
		overlap_bytes: 2,
		max_held_bytes: limit.min(64 * 1024),
		incomplete_call: match chunk % 3 {
			0 => IncompleteCall::Stop("blocked".to_owned()),
			1 => IncompleteCall::Drop,
			_ => IncompleteCall::Send,
		},
	}
}

/// Answers every check: the offered text's length decides, so the input picks the verdict.
fn answer(mut progress: ReplyProgress<'_>) {
	while let ReplyProgress::Waiting(held) = progress {
		let verdict = if held.check().text.len() % 2 == 0 {
			Verdict::Release
		} else {
			Verdict::Stop("blocked".to_owned())
		};
		progress = held.resolve(verdict);
	}
}

// Read the wire independently of the SSE parser being exercised by the driver.
fn assert_events(client: ClientProtocol, stream: &[u8]) {
	let text = std::str::from_utf8(stream).expect("SSE output is UTF-8");
	let text = text.replace("\r\n", "\n");
	assert!(
		text.ends_with("\n\n"),
		"output ends inside an event: {text:?}"
	);
	let mut ended = false;
	for event in text.split_terminator("\n\n") {
		assert!(!ended, "an event follows the end: {text:?}");
		let data = event
			.lines()
			.find_map(|line| line.strip_prefix("data: "))
			.expect("an SSE event has data");
		if data == "[DONE]" {
			assert_eq!(client, ClientProtocol::ChatCompletions);
			ended = true;
			continue;
		}
		let value: Value = serde_json::from_str(data).expect("SSE data is JSON");
		ended = value.get("error").is_some()
			|| match client {
				ClientProtocol::Messages => value["type"] == "message_stop",
				ClientProtocol::Responses => matches!(
					value["type"].as_str(),
					Some("response.completed" | "response.incomplete" | "response.failed")
				),
				ClientProtocol::ChatCompletions => false,
				ClientProtocol::Gemini => {
					assert_signatures(&value);
					value["candidates"][0].get("finishReason").is_some()
				},
			};
	}
	assert!(ended, "output ends without a terminal event: {text:?}");
}

fn assert_signatures(response: &Value) {
	let Some(parts) = response["candidates"][0]["content"]["parts"].as_array() else {
		return;
	};
	for part in parts {
		if let Some(signature) = part.get("thoughtSignature") {
			assert_ne!(
				part["thought"],
				Value::Bool(true),
				"a signature on a thought"
			);
			assert!(signature.as_str().is_some_and(|text| !text.is_empty()));
		}
	}
}
