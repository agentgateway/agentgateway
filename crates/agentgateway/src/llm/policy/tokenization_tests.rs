use http_body_util::BodyExt;

use super::*;
use crate::llm::types::completions::Request;

fn tokenize(text: &str) -> (Arc<EmailTokens>, String) {
	let mut req: Request = serde_json::from_value(json!({
		"model": "test", "messages": [{"role": "user", "content": text}]
	}))
	.unwrap();
	let tokens = EmailTokens::tokenize(&mut req).unwrap();
	let value = serde_json::to_value(req).unwrap();
	(
		tokens,
		value["messages"][0]["content"].as_str().unwrap().to_owned(),
	)
}

#[test]
fn email_round_trip_repeats_unicode_and_request_isolation() {
	let original = "Привет alice+tag@example.org, alice+tag@example.org и bob@example.net!";
	let (tokens, masked) = tokenize(original);
	assert!(!masked.contains("@"));
	let pieces: Vec<_> = masked.split(", ").collect();
	assert!(pieces[1].starts_with(pieces[0].strip_prefix("Привет ").unwrap()));
	assert_eq!(tokens.restore(&masked, &mut String::new(), true), original);
	let (other, _) = tokenize(original);
	assert_eq!(other.restore(&masked, &mut String::new(), true), masked);
	assert!(!format!("{tokens:?}").contains("example"));
}

#[test]
fn restoration_handles_every_split_and_releases_plain_text_immediately() {
	let (tokens, masked) = tokenize("alice@example.org");
	for split in 0..=masked.len() {
		let mut pending = String::new();
		let first = tokens.restore(&format!("Hello {}", &masked[..split]), &mut pending, false);
		assert!(first.starts_with("Hello "));
		let second = tokens.restore(&format!("{} 世界", &masked[split..]), &mut pending, true);
		assert_eq!(
			first + &second,
			"Hello alice@example.org 世界",
			"split {split}"
		);
	}
	let mut pending = String::new();
	let output: String = masked
		.chars()
		.map(|c| tokens.restore(&c.to_string(), &mut pending, false))
		.collect();
	assert_eq!(output, "alice@example.org");
	assert!(pending.is_empty());
}

#[test]
fn unknown_and_truncated_tokens_remain_literal() {
	let (tokens, masked) = tokenize("alice@example.org");
	let unknown = format!("{}9000__", tokens.prefix);
	assert_eq!(tokens.restore(&unknown, &mut String::new(), true), unknown);
	for split in 0..masked.len() {
		let partial = &masked[..split];
		let mut pending = String::new();
		let output = tokens.restore(partial, &mut pending, false);
		assert_eq!(output + &tokens.restore("", &mut pending, true), partial);
	}
}

#[test]
fn request_visits_system_tool_results_and_arguments() {
	let mut req: Request = serde_json::from_value(json!({
		"model":"test", "messages":[
			{"role":"system","content":"alice@example.org"},
			{"role":"user","content":[{"type":"text","text":"alice@example.org"}]},
			{"role":"assistant","tool_calls":[{"id":"1","type":"function","function":{"name":"email","arguments":"{\"to\":\"alice@example.org\"}"}}]},
			{"role":"tool","tool_call_id":"1","content":"alice@example.org"}
		]
	})).unwrap();
	let tokens = EmailTokens::tokenize(&mut req).unwrap();
	let masked = serde_json::to_string(&req).unwrap();
	assert!(!masked.contains("alice@example.org"));
	let restored = tokens.restore(&masked, &mut String::new(), true);
	let value: Value = serde_json::from_str(&restored).unwrap();
	assert_eq!(value["messages"][0]["content"], "alice@example.org");
	assert_eq!(
		value["messages"][2]["tool_calls"][0]["function"]["arguments"],
		"{\"to\":\"alice@example.org\"}"
	);
}

#[test]
fn stream_channels_do_not_mix_choices_tools_or_reasoning() {
	let (tokens, token) = tokenize("alice@example.org");
	let split = token.len() / 2;
	let mut stream = StreamTokens {
		tokens,
		pending: BTreeMap::new(),
	};
	let mut first = json!({"choices":[
		{"index":0,"delta":{"content":&token[..split],"reasoning_content":&token[..split],"tool_calls":[{"index":0,"function":{"arguments":&token[..split]}},{"index":1,"function":{"arguments":"ordinary"}}]}},
		{"index":1,"delta":{"content":"unrelated"}}
	]});
	stream.restore_event(&mut first).unwrap();
	assert_eq!(first["choices"][0]["delta"]["content"], "");
	assert_eq!(first["choices"][1]["delta"]["content"], "unrelated");
	let mut second = json!({"choices":[{"index":0,"delta":{"content":&token[split..],"reasoning_content":&token[split..],"tool_calls":[{"index":0,"function":{"arguments":&token[split..]}}]},"finish_reason":"stop"}]});
	stream.restore_event(&mut second).unwrap();
	assert_eq!(
		second["choices"][0]["delta"]["content"],
		"alice@example.org"
	);
	assert_eq!(
		second["choices"][0]["delta"]["reasoning_content"],
		"alice@example.org"
	);
	assert_eq!(
		second["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
		"alice@example.org"
	);
	stream.check_finished().unwrap();
}

#[tokio::test]
async fn sse_transport_preserves_metadata_usage_and_flushes_incomplete_token() {
	let (tokens, token) = tokenize("alice@example.org");
	let mut wire = String::from(": heartbeat\n\n");
	for c in token.chars() {
		wire.push_str(&format!(
			"data: {}\n\n",
			json!({"id":"chat-1","choices":[{"index":0,"delta":{"content":c.to_string()}}]})
		));
	}
	wire.push_str(&format!(
		"data: {}\n\n",
		json!({"choices":[{"index":0,"delta":{"content":"__AGW_"}}]})
	));
	wire.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
	wire.push_str("data: {\"choices\":[],\"usage\":{\"completion_tokens\":12}}\n\ndata: [DONE]\n\n");
	let chunks: Vec<_> = wire
		.bytes()
		.map(|b| Ok::<_, std::io::Error>(Bytes::from(vec![b])))
		.collect();
	let body = agent_http::RawBody::from_stream(futures_util::stream::iter(chunks));
	let output = tokens
		.restore_stream(body, 65536)
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let output = String::from_utf8(output.to_vec()).unwrap();
	let mut content = String::new();
	let mut usage = None;
	for line in output
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
	{
		if line == "[DONE]" {
			continue;
		}
		let value: Value = serde_json::from_str(line).unwrap();
		if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
			content.push_str(text);
		}
		if value.get("usage").is_some() {
			usage = Some(value["usage"].clone());
		}
	}
	assert_eq!(content, "alice@example.org__AGW_");
	assert_eq!(usage.unwrap()["completion_tokens"], 12);
	assert!(output.contains(": heartbeat"));
	assert!(output.contains("chat-1"));
	assert!(output.contains("[DONE]"));
}

#[tokio::test]
async fn stream_emits_before_upstream_finishes_and_errors_on_truncation() {
	let (tokens, token) = tokenize("alice@example.org");
	let wire = format!(
		"data: {}\n\n",
		json!({"choices":[{"index":0,"delta":{"content":format!("Hello {}", &token[..20])}}]})
	);
	let chunks =
		futures_util::stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(wire)) })
			.chain(futures_util::stream::pending());
	let mut output = tokens
		.clone()
		.restore_stream(agent_http::RawBody::from_stream(chunks), 65536)
		.into_data_stream();
	let first = tokio::time::timeout(std::time::Duration::from_secs(1), output.next())
		.await
		.unwrap()
		.unwrap()
		.unwrap();
	assert!(
		String::from_utf8(first.to_vec())
			.unwrap()
			.contains("Hello ")
	);
	let wire = format!(
		"data: {}\n\ndata: [DONE]\n\n",
		json!({"choices":[{"index":0,"delta":{"content":&token[..20]}}]})
	);
	assert!(
		tokens
			.restore_stream(agent_http::RawBody::from(wire), 65536)
			.collect()
			.await
			.is_err()
	);
}

#[tokio::test]
async fn gateway_tokenization_masks_upstream_and_restores_after_response_guards() {
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use crate::llm::{AIProvider, openai};
	use crate::test_helpers::proxymock::{
		llm_named_provider, send_request_body, setup_llm_named_provider_mock,
	};

	for streaming in [false, true] {
		let mock = MockServer::start().await;
		Mock::given(wiremock::matchers::any()).respond_with(move |req: &wiremock::Request| {
			let value: Value = serde_json::from_slice(&req.body).unwrap();
			let content = value["messages"][0]["content"].as_str().unwrap();
			assert!(!content.contains("alice@example.org"));
			if streaming {
				let mut wire = String::new();
				for c in content.chars() {
					wire.push_str(&format!("data: {}\n\n", json!({"id":"chat-test","object":"chat.completion.chunk","created":0,"model":"test","choices":[{"index":0,"delta":{"content":c.to_string()},"finish_reason":null}]})));
				}
				wire.push_str(&format!("data: {}\n\ndata: [DONE]\n\n", json!({"id":"chat-test","object":"chat.completion.chunk","created":0,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})));
				ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(wire)
			} else {
				ResponseTemplate::new(200).set_body_json(json!({"id":"chat-test","object":"chat.completion","created":0,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}))
			}
		}).mount(&mock).await;
		let mut provider = llm_named_provider(
			&mock,
			AIProvider::OpenAI(openai::Provider {
				model_override: None,
				moderation: None,
			}),
			false,
		);
		provider.policies = Some(
			serde_json::from_value(json!({"ai":{"promptGuard":{
				"tokenization":{"email":true}, "streaming":"Enabled",
				"response":[{"regex":{"action":"reject","rules":[{"builtin":"email"}]}}]
			}}}))
			.unwrap(),
		);
		let (mock, _gateway, io) = setup_llm_named_provider_mock(mock, provider, "{}");
		let body = serde_json::to_vec(&json!({"model":"test","stream":streaming,"messages":[{"role":"user","content":"Hello alice@example.org"}]})).unwrap();
		let response = send_request_body(
			io,
			::http::Method::POST,
			"http://localhost/v1/chat/completions",
			&body,
		)
		.await;
		assert_eq!(response.status(), 200);
		let bytes = response.into_body().collect().await.unwrap().to_bytes();
		let text = String::from_utf8(bytes.to_vec()).unwrap();
		if streaming {
			let content: String = text
				.lines()
				.filter_map(|line| line.strip_prefix("data: "))
				.filter_map(|line| serde_json::from_str::<Value>(line).ok())
				.filter_map(|value| {
					value["choices"][0]["delta"]["content"]
						.as_str()
						.map(str::to_owned)
				})
				.collect();
			assert_eq!(content, "Hello alice@example.org");
		} else {
			let value: Value = serde_json::from_str(&text).unwrap();
			assert_eq!(
				value["choices"][0]["message"]["content"],
				"Hello alice@example.org"
			);
		}
		assert_eq!(mock.received_requests().await.unwrap().len(), 1);
	}
}

#[tokio::test]
async fn gateway_tokenization_leaves_other_api_formats_unchanged() {
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use crate::llm::{AIProvider, openai};
	use crate::test_helpers::proxymock::{
		llm_named_provider, send_request_body, setup_llm_named_provider_mock,
	};

	for (route, path, request) in [
		(
			"responses",
			"/v1/responses",
			json!({"model":"test","input":"alice@example.org"}),
		),
		(
			"messages",
			"/v1/messages",
			json!({"model":"test","max_tokens":64,"messages":[{"role":"user","content":"alice@example.org"}]}),
		),
		(
			"embeddings",
			"/v1/embeddings",
			json!({"model":"test","input":"alice@example.org"}),
		),
		(
			"detect",
			"/v1/custom",
			json!({"model":"test","input":"alice@example.org"}),
		),
		(
			"passthrough",
			"/v1/custom",
			json!({"model":"test","input":"alice@example.org"}),
		),
	] {
		let mut results = Vec::new();
		for enabled in [false, true] {
			let mock = MockServer::start().await;
			Mock::given(wiremock::matchers::any())
				.respond_with(move |_: &wiremock::Request| {
					let response = if matches!(route, "responses" | "messages") {
                        use crate::llm::types::responses::{ResponseBuilder, typed};

                        let mut response = ResponseBuilder::new("resp-test", "test")
                            .response(typed::Status::Completed, None, None, None);
                        response.created_at = 0;
                        response.output = serde_json::from_value(json!([
                            {"id":"msg-test","type":"message","role":"assistant","status":"completed",
                             "content":[{"type":"output_text","text":"alice@example.org","annotations":[]}]}
                        ])).unwrap();
                        serde_json::to_value(response).unwrap()
					} else if route == "embeddings" {
						json!({"object":"list","model":"test","data":[{"object":"embedding","index":0,"embedding":[0.5]}]})
					} else {
						json!({"text":"alice@example.org"})
					};
					ResponseTemplate::new(200).set_body_json(response)
				})
				.expect(1)
				.mount(&mock)
				.await;
			let mut provider = llm_named_provider(
				&mock,
				AIProvider::OpenAI(openai::Provider {
					model_override: None,
					moderation: None,
				}),
				false,
			);
			provider.policies = Some(
				serde_json::from_value(json!({"ai":{
					"routes":{"*":route}, "promptGuard":{"tokenization":{"email":enabled}}
				}}))
				.unwrap(),
			);
			let (mock, _gateway, io) = setup_llm_named_provider_mock(mock, provider, "{}");
			let response = send_request_body(
				io,
				::http::Method::POST,
				&format!("http://localhost{path}"),
				&serde_json::to_vec(&request).unwrap(),
			)
			.await;
			assert_eq!(
				response.status(),
				200,
				"route {route}, tokenization {enabled}"
			);
			let body = response.into_body().collect().await.unwrap().to_bytes();
			let response: Value = serde_json::from_slice(&body).unwrap();
			let received = mock.received_requests().await.unwrap();
			assert_eq!(received.len(), 1, "route {route}");
			let forwarded: Value = serde_json::from_slice(&received[0].body).unwrap();
			assert!(
				forwarded.to_string().contains("alice@example.org"),
				"route {route}: {forwarded}"
			);
			if route == "embeddings" {
				assert_eq!(response["data"][0]["embedding"], json!([0.5]));
			} else {
				assert!(
					response.to_string().contains("alice@example.org"),
					"route {route}: {response}"
				);
			}
			results.push((forwarded, response));
		}
		assert_eq!(results[0], results[1], "route {route}");
	}
}

#[tokio::test]
async fn malformed_and_oversized_streams_fail_without_panicking() {
	let (tokens, token) = tokenize("alice@example.org");
	for wire in [
		"data: invalid\n\n".to_owned(),
		"data: {\"choices\":[{\"index\":0,\"delta\":false}]}\n\n".to_owned(),
		format!(
			"data: {}\n\n",
			json!({"choices":[{"index":0,"delta":{"content":token}}]})
		),
	] {
		assert!(
			tokens
				.clone()
				.restore_stream(agent_http::RawBody::from(wire), 64)
				.collect()
				.await
				.is_err()
		);
	}
}

#[test]
fn oversized_mapping_rejects_request_instead_of_leaking_remaining_addresses() {
	let text = (0..4097)
		.map(|n| format!("user{n}@example.org"))
		.collect::<Vec<_>>()
		.join(" ");
	let mut req: Request =
		serde_json::from_value(json!({"model":"test","messages":[{"role":"user","content":text}]}))
			.unwrap();
	assert!(matches!(
		EmailTokens::tokenize(&mut req),
		Err(AIError::RequestTooLarge)
	));
}

#[test]
fn malformed_prefix_does_not_consume_a_later_valid_token() {
	let (tokens, token) = tokenize("alice@example.org");
	let text = format!("{}invalid {token}", tokens.prefix);
	assert_eq!(
		tokens.restore(&text, &mut String::new(), true),
		format!("{}invalid alice@example.org", tokens.prefix)
	);
}

#[test]
fn buffered_response_restores_tool_arguments_and_preserves_usage() {
	let (tokens, token) = tokenize("alice@example.org");
	let mut response: crate::llm::types::completions::Response = serde_json::from_value(json!({
		"id":"chat-test", "object":"chat.completion", "created":0, "model":"test",
		"choices":[{"index":0,"message":{"role":"assistant","content":token,
			"tool_calls":[{"id":"call-1","type":"function","function":{"name":"send_email","arguments":format!("{{\"to\":\"{token}\"}}")}}]
		},"finish_reason":"tool_calls"}],
		"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}
	})).unwrap();
	tokens.restore_response(&mut response);
	let value = serde_json::to_value(response).unwrap();
	assert_eq!(
		value["choices"][0]["message"]["content"],
		"alice@example.org"
	);
	let call = &value["choices"][0]["message"]["tool_calls"][0];
	let args: Value = serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
	assert_eq!(args["to"], "alice@example.org");
	assert_eq!(call["id"], "call-1");
	assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
	assert_eq!(value["usage"]["total_tokens"], 7);
}

#[test]
fn choice_finish_flushes_partial_tool_and_legacy_arguments() {
	let (tokens, token) = tokenize("alice@example.org");
	let partial = &token[..20];
	let mut stream = StreamTokens {
		tokens,
		pending: BTreeMap::new(),
	};
	let mut first = json!({"choices":[{"index":0,"delta":{
		"tool_calls":[{"index":2,"function":{"arguments":partial}},{"index":3,"custom":{"input":partial}}],
		"function_call":{"arguments":partial}
	}}]});
	stream.restore_event(&mut first).unwrap();
	let mut finish = json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]});
	stream.restore_event(&mut finish).unwrap();
	let delta = &finish["choices"][0]["delta"];
	assert_eq!(delta["function_call"]["arguments"], partial);
	assert_eq!(delta["tool_calls"][0]["index"], 2);
	assert_eq!(delta["tool_calls"][0]["function"]["arguments"], partial);
	assert_eq!(delta["tool_calls"][1]["index"], 3);
	assert_eq!(delta["tool_calls"][1]["custom"]["input"], partial);
	stream.check_finished().unwrap();
}

#[test]
fn buffered_response_restores_tokens_in_content_parts() {
	let (tokens, token) = tokenize("alice@example.org");
	let mut response: crate::llm::types::completions::Response = serde_json::from_value(json!({
		"id":"chat-test", "object":"chat.completion", "created":0, "model":"test",
		"choices":[{"index":0,"message":{"role":"assistant","content":[
			{"type":"thinking","thinking":format!("Contact {token}")},
			{"type":"text","text":format!("Email {token}")}
		]},"finish_reason":"stop"}]
	}))
	.unwrap();
	tokens.restore_response(&mut response);
	let value = serde_json::to_value(response).unwrap();
	assert_eq!(
		value["choices"][0]["message"]["content"],
		json!([
			{"type":"thinking","thinking":"Contact alice@example.org"},
			{"type":"text","text":"Email alice@example.org"}
		])
	);
}

#[tokio::test]
async fn stream_preserves_upstream_error_after_a_partial_token() {
	let (tokens, token) = tokenize("alice@example.org");
	let error = json!({"error":{"message":"upstream interrupted","type":"upstream_error","code":"upstream_error"}});
	let wire = format!(
		"data: {}\n\ndata: {error}\n\ndata: [DONE]\n\n",
		json!({
			"choices":[{"index":0,"delta":{"content":format!("Hello {}", &token[..20])}}]
		})
	);
	let bytes = tokens
		.restore_stream(agent_http::RawBody::from(wire), 65536)
		.collect()
		.await
		.unwrap()
		.to_bytes();
	let text = String::from_utf8(bytes.to_vec()).unwrap();
	let events: Vec<_> = text
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.collect();
	let first: Value = serde_json::from_str(events[0]).unwrap();
	assert_eq!(first["choices"][0]["delta"]["content"], "Hello ");
	assert_eq!(serde_json::from_str::<Value>(events[1]).unwrap(), error);
	assert_eq!(events[2], "[DONE]");
}
