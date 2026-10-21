use agentgateway::llm::{AIProvider, gemini, vertex};
use agentgateway::types::local::LocalNamedAIProvider;
use wiremock::{Request, Respond};

use crate::common::prelude::*;

const MODEL: &str = "gemini-3.8-flash";
const SIGNATURE: &str = "sig-1";
/// (input, output) tokens the mock reports, checked against the request log
const CALL_TOKENS: (u64, u64) = (10, 5);
const ANSWER_TOKENS: (u64, u64) = (20, 3);

#[derive(Clone, Copy, Debug)]
enum Client {
	Messages,
	Responses,
}

#[derive(Clone, Copy, Debug)]
enum Upstream {
	Gemini,
	Vertex,
}

#[derive(Clone, Copy, Debug)]
enum StreamEnding {
	ErrorAndHang,
	Close,
	Reset,
}

#[rstest::rstest]
#[tokio::test]
async fn tool_loop_replays_the_thought_signature(
	#[values(Client::Messages, Client::Responses)] client: Client,
	#[values(Upstream::Gemini, Upstream::Vertex)] upstream: Upstream,
	#[values(false, true)] stream: bool,
) {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::any())
		.respond_with(Gemini { stream })
		.mount(&mock)
		.await;
	let provider = upstream.provider(&mock);
	let (mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {"routes": {"/v1/messages": "messages", "/v1/responses": "responses"}}
		}))
		.await;

	let first = client.first_turn(stream);
	let reply = send(&io, client, &first, CALL_TOKENS).await;
	let second = client.second_turn(first, client.output(&reply, stream));
	let answer = send(&io, client, &second, ANSWER_TOKENS).await;
	let ended = match client {
		Client::Messages => "end_turn",
		Client::Responses => "completed",
	};
	assert_eq!(
		client.answer(&answer, stream),
		(vec![json!("Sunny in Paris.")], json!(ended))
	);

	let requests = mock.received_requests().await.unwrap();
	assert_eq!(requests.len(), 2);
	let replay: Value = serde_json::from_slice(&requests[1].body).unwrap();
	let parts = parts(&replay);
	let call = parts
		.iter()
		.find(|p| p.get("functionCall").is_some())
		.expect("the call is replayed");
	assert_eq!(call["thoughtSignature"], SIGNATURE);
	assert_eq!(
		call["functionCall"],
		json!({"name": "get_weather", "args": {"city": "Paris"}})
	);
	let result = parts
		.iter()
		.find(|p| p.get("functionResponse").is_some())
		.expect("the result is sent");
	assert_eq!(
		result["functionResponse"],
		json!({"name": "get_weather", "response": {"content": "Sunny"}})
	);
}

/// Gemini reports an error mid-stream and keeps the connection open.
/// The client gets the error in its own format as the last event, and
/// the body ends there without an error.
#[rstest::rstest]
#[tokio::test]
async fn a_mid_stream_error_reaches_the_client(
	#[values(Client::Messages, Client::Responses)] client: Client,
) {
	let mock = MockServer::start().await;
	let (upstream, _) = streaming_upstream(StreamEnding::ErrorAndHang).await;
	let provider = LocalNamedAIProvider {
		host_override: Some(Target::Address(upstream)),
		..Upstream::Gemini.provider(&mock)
	};
	let (_mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {"routes": {"/v1/messages": "messages", "/v1/responses": "responses"}}
		}))
		.await;

	let path = format!("/{}{}", rand::rng().random::<u64>(), client.path());
	let res = send_request_body(
		io.clone(),
		Method::POST,
		&format!("http://lo{path}"),
		&serde_json::to_vec(&client.first_turn(true)).unwrap(),
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);
	let (reply, failed) = tokio::time::timeout(Duration::from_secs(5), drain(res.into_body()))
		.await
		.expect("the body should end once Gemini reports an error");
	assert!(
		!failed,
		"the body should end after the error event:\n{reply}"
	);
	for ending in ["message_stop", "response.completed", "[DONE]"] {
		assert!(!reply.contains(ending), "{ending} in:\n{reply}");
	}
	let last = events(&reply).last().unwrap();
	match client {
		Client::Messages => assert_eq!(
			last,
			json!({"type": "error", "error": {"type": "overloaded_error", "message": "The model is overloaded."}})
		),
		Client::Responses => {
			assert_eq!(last["type"], "response.failed", "{reply}");
			assert_eq!(
				last["response"]["error"],
				json!({"code": "server_error", "message": "The model is overloaded."})
			);
		},
	}
	assert_logged_once(&path, (7, 1)).await;
}

#[rstest::rstest]
#[tokio::test]
async fn an_interrupted_stream_preserves_the_text_before_it_fails(
	#[values(Client::Messages, Client::Responses)] client: Client,
	#[values(StreamEnding::Close, StreamEnding::Reset)] ending: StreamEnding,
) {
	let mock = MockServer::start().await;
	let (upstream, end_stream) = streaming_upstream(ending).await;
	let provider = LocalNamedAIProvider {
		host_override: Some(Target::Address(upstream)),
		..Upstream::Gemini.provider(&mock)
	};
	let (_mock, mut bind, io) = setup_llm_named_provider_mock(mock, provider, "{}");
	bind
		.attach_route_policy(json!({
			"ai": {"routes": {"/v1/messages": "messages", "/v1/responses": "responses"}}
		}))
		.await;

	let path = format!("/{}{}", rand::rng().random::<u64>(), client.path());
	let res = send_request_body(
		io.clone(),
		Method::POST,
		&format!("http://lo{path}"),
		&serde_json::to_vec(&client.first_turn(true)).unwrap(),
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);
	let (reply, failed) = tokio::time::timeout(Duration::from_secs(5), async {
		let mut body = res.into_body();
		let mut reply = String::new();
		while !reply.contains("\n\n") {
			let frame = body.frame().await.unwrap().unwrap();
			reply.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap());
		}
		let started = match client {
			Client::Messages => "message_start",
			Client::Responses => "response.created",
		};
		assert_eq!(events(&reply).next().unwrap()["type"], started, "{reply}");
		end_stream.send(()).unwrap();
		let (rest, failed) = drain(body).await;
		reply.push_str(&rest);
		(reply, failed)
	})
	.await
	.expect("an interrupted stream should end");
	assert!(failed, "an interrupted stream must fail:\n{reply}");
	let delta = match client {
		Client::Messages => events(&reply)
			.into_iter()
			.find(|e| e["type"] == "content_block_delta")
			.map(|e| e["delta"]["text"].clone()),
		Client::Responses => events(&reply)
			.into_iter()
			.find(|e| e["type"] == "response.output_text.delta")
			.map(|e| e["delta"].clone()),
	};
	assert_eq!(delta, Some(json!("Sunny")), "{reply}");
	for ending in ["message_stop", "response.completed", "[DONE]"] {
		assert!(!reply.contains(ending), "{ending} in:\n{reply}");
	}
	assert_logged_once(&path, (7, 1)).await;
}

async fn send(io: &MemoryClient, client: Client, body: &Value, usage: (u64, u64)) -> String {
	let path = format!("/{}{}", rand::rng().random::<u64>(), client.path());
	let res = send_request_body(
		io.clone(),
		Method::POST,
		&format!("http://lo{path}"),
		&serde_json::to_vec(body).unwrap(),
	)
	.await;
	assert_eq!(res.status(), StatusCode::OK);
	let reply = String::from_utf8(read_body_raw(res.into_body()).await.to_vec()).unwrap();
	assert!(!reply.contains("__thought__"), "{reply}");
	assert_logged_once(&path, usage).await;
	reply
}

async fn assert_logged_once(path: &str, usage: (u64, u64)) {
	let want = [("scope", "request"), ("http.path", path)];
	let log = agent_core::telemetry::testing::eventually_find(&want)
		.await
		.unwrap();
	assert_eq!(log["gen_ai.usage.input_tokens"], usage.0, "{log:#}");
	assert_eq!(log["gen_ai.usage.output_tokens"], usage.1, "{log:#}");
	assert_eq!(agent_core::telemetry::testing::find(&want).len(), 1);
}

async fn drain(mut body: Body) -> (String, bool) {
	let mut reply = String::new();
	while let Some(frame) = body.frame().await {
		match frame {
			Ok(frame) => reply.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap()),
			Err(_) => return (reply, true),
		}
	}
	(reply, false)
}

impl Upstream {
	fn provider(self, mock: &MockServer) -> LocalNamedAIProvider {
		let (provider, policies) = match self {
			Upstream::Gemini => (
				AIProvider::Gemini(gemini::Provider {
					model_override: None,
				}),
				None,
			),
			Upstream::Vertex => (
				AIProvider::Vertex(vertex::Provider {
					project_id: strng::new("test-project"),
					model_override: None,
					region: None,
				}),
				// Replaces the default GCP auth, which needs credentials
				Some(serde_json::from_value(json!({"backendAuth": {"key": "test"}})).unwrap()),
			),
		};
		LocalNamedAIProvider {
			policies,
			..llm_named_provider(mock, provider, false)
		}
	}
}

impl Client {
	fn path(self) -> &'static str {
		match self {
			Client::Messages => "/v1/messages",
			Client::Responses => "/v1/responses",
		}
	}

	fn first_turn(self, stream: bool) -> Value {
		let prompt = json!({"role": "user", "content": "Weather in Paris?"});
		let schema = json!({
			"type": "object",
			"properties": {"city": {"type": "string"}},
			"required": ["city"]
		});
		match self {
			Client::Messages => json!({
				"model": MODEL,
				"max_tokens": 256,
				"stream": stream,
				"messages": [prompt],
				"tools": [{"name": "get_weather", "input_schema": schema}]
			}),
			Client::Responses => json!({
				"model": MODEL,
				"stream": stream,
				"input": [prompt],
				"tools": [{"type": "function", "name": "get_weather", "parameters": schema}]
			}),
		}
	}

	/// Replay the assistant output unchanged, including opaque signature carriers
	fn second_turn(self, mut body: Value, output: Vec<Value>) -> Value {
		match self {
			Client::Messages => {
				let id = output.iter().find(|b| b["type"] == "tool_use").unwrap()["id"].clone();
				let messages = body["messages"].as_array_mut().unwrap();
				messages.push(json!({"role": "assistant", "content": output}));
				messages.push(json!({
					"role": "user",
					"content": [{"type": "tool_result", "tool_use_id": id, "content": "Sunny"}]
				}));
			},
			Client::Responses => {
				let id = output
					.iter()
					.find(|i| i["type"] == "function_call")
					.unwrap()["call_id"]
					.clone();
				let input = body["input"].as_array_mut().unwrap();
				input.extend(output);
				input.push(json!({"type": "function_call_output", "call_id": id, "output": "Sunny"}));
			},
		}
		body
	}

	fn output(self, reply: &str, stream: bool) -> Vec<Value> {
		let output = match (self, stream) {
			(Client::Messages, false) => serde_json::from_str::<Value>(reply).unwrap()["content"].take(),
			(Client::Responses, false) => serde_json::from_str::<Value>(reply).unwrap()["output"].take(),
			(Client::Messages, true) => return message_blocks(reply),
			(Client::Responses, true) => completed(reply)["output"].take(),
		};
		serde_json::from_value(output).unwrap()
	}

	fn answer(self, reply: &str, stream: bool) -> (Vec<Value>, Value) {
		let output = self.output(reply, stream).into_iter();
		let text = match self {
			Client::Messages => output
				.filter(|b| b["type"] == "text")
				.map(|b| b["text"].clone())
				.collect(),
			Client::Responses => output
				.filter(|i| i["type"] == "message")
				.flat_map(|i| i["content"].as_array().unwrap().clone())
				.filter(|c| c["type"] == "output_text")
				.map(|c| c["text"].clone())
				.collect(),
		};
		let ended = match (self, stream) {
			(Client::Messages, false) => {
				serde_json::from_str::<Value>(reply).unwrap()["stop_reason"].take()
			},
			(Client::Responses, false) => serde_json::from_str::<Value>(reply).unwrap()["status"].take(),
			(Client::Messages, true) => events(reply)
				.find(|e| e["type"] == "message_delta")
				.unwrap()["delta"]["stop_reason"]
				.take(),
			(Client::Responses, true) => completed(reply)["status"].take(),
		};
		(text, ended)
	}
}

fn message_blocks(reply: &str) -> Vec<Value> {
	let mut blocks: Vec<Value> = vec![];
	let mut args = String::new();
	for event in events(reply) {
		match event["type"].as_str().unwrap() {
			"content_block_start" => blocks.push(event["content_block"].clone()),
			"content_block_delta" => {
				let delta = &event["delta"];
				let field = match delta["type"].as_str().unwrap() {
					"input_json_delta" => {
						args.push_str(delta["partial_json"].as_str().unwrap());
						continue;
					},
					"text_delta" => "text",
					"thinking_delta" => "thinking",
					"signature_delta" => "signature",
					other => panic!("unexpected delta {other}"),
				};
				let block = blocks.last_mut().unwrap();
				let joined = format!(
					"{}{}",
					block[field].as_str().unwrap_or_default(),
					delta[field].as_str().unwrap()
				);
				block[field] = joined.into();
			},
			"content_block_stop" if !args.is_empty() => {
				blocks.last_mut().unwrap()["input"] =
					serde_json::from_str(&std::mem::take(&mut args)).unwrap();
			},
			_ => {},
		}
	}
	blocks
}

fn completed(reply: &str) -> Value {
	events(reply)
		.find(|e| e["type"] == "response.completed")
		.unwrap()["response"]
		.take()
}

fn events(reply: &str) -> impl Iterator<Item = Value> + '_ {
	reply
		.lines()
		.filter_map(|line| line.strip_prefix("data: "))
		.filter(|data| *data != "[DONE]")
		.map(|data| serde_json::from_str(data).unwrap())
}

fn parts(request: &Value) -> Vec<&Value> {
	request["contents"]
		.as_array()
		.unwrap()
		.iter()
		.flat_map(|c| c["parts"].as_array().unwrap())
		.collect()
}

struct Gemini {
	stream: bool,
}

impl Respond for Gemini {
	fn respond(&self, request: &Request) -> ResponseTemplate {
		let body: Value = serde_json::from_slice(&request.body).unwrap();
		let answered = parts(&body)
			.iter()
			.any(|p| p.get("functionResponse").is_some());
		let (part, (input, output)) = if answered {
			(json!({"text": "Sunny in Paris."}), ANSWER_TOKENS)
		} else {
			(
				json!({
					"functionCall": {"name": "get_weather", "args": {"city": "Paris"}},
					"thoughtSignature": SIGNATURE
				}),
				CALL_TOKENS,
			)
		};
		let reply = json!({
			"candidates": [{"content": {"role": "model", "parts": [part]}, "finishReason": "STOP"}],
			"usageMetadata": {
				"promptTokenCount": input,
				"candidatesTokenCount": output,
				"totalTokenCount": input + output
			},
			"modelVersion": MODEL
		});
		if self.stream {
			ResponseTemplate::new(200)
				.insert_header("content-type", "text/event-stream")
				.set_body_string(format!("data: {reply}\r\n\r\n"))
		} else {
			ResponseTemplate::new(200).set_body_json(reply)
		}
	}
}

async fn streaming_upstream(ending: StreamEnding) -> (std::net::SocketAddr, oneshot::Sender<()>) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let (end_stream, client_ready) = oneshot::channel();
	tokio::spawn(async move {
		let (mut conn, _) = listener.accept().await.unwrap();
		let mut request = Vec::new();
		while !request.windows(4).any(|w| w == b"\r\n\r\n") {
			let mut buf = [0; 4096];
			let n = conn.read(&mut buf).await.unwrap();
			assert_ne!(n, 0, "the request headers must arrive before EOF");
			request.extend_from_slice(&buf[..n]);
		}
		// Consume the request body so unread data cannot reset the clean-EOF case
		let header_end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
		let content_length = std::str::from_utf8(&request[..header_end])
			.unwrap()
			.lines()
			.filter_map(|line| line.split_once(':'))
			.find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
			.unwrap()
			.1
			.trim()
			.parse::<usize>()
			.unwrap();
		let mut rest = vec![0; (header_end + content_length).saturating_sub(request.len())];
		conn.read_exact(&mut rest).await.unwrap();
		let mut frames = vec![json!({
			"candidates": [{"content": {"role": "model", "parts": [{"text": "Sunny"}]}}],
			"usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 1, "totalTokenCount": 8},
			"modelVersion": MODEL
		})];
		if matches!(ending, StreamEnding::ErrorAndHang) {
			frames.push(
				json!({"error": {"code": 503, "message": "The model is overloaded.", "status": "UNAVAILABLE"}}),
			);
		}
		let mut out = String::from(
			"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
		);
		for frame in frames {
			let data = format!("data: {frame}\r\n\r\n");
			out.push_str(&format!("{:x}\r\n{data}\r\n", data.len()));
		}
		conn.write_all(out.as_bytes()).await.unwrap();
		if !matches!(ending, StreamEnding::ErrorAndHang) {
			// A reset before the client receives an event can discard the response headers on Windows
			client_ready.await.unwrap();
		}
		match ending {
			StreamEnding::ErrorAndHang => std::future::pending::<()>().await,
			StreamEnding::Close => {
				conn.write_all(b"0\r\n\r\n").await.unwrap();
				conn.shutdown().await.unwrap();
			},
			StreamEnding::Reset => socket2::SockRef::from(&conn)
				.set_linger(Some(Duration::ZERO))
				.unwrap(),
		}
		drop(conn);
	});
	(addr, end_stream)
}
