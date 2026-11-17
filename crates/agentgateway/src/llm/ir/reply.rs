//! Converts a provider's reply back to the client's protocol for an IR attempt.
//!
//! Accounting reads the provider's events as decoded, before stop sequences or the client's
//! writer change what the client sees. Failures stay failures: before any output they are an HTTP error, after it an error
//! event that ends the stream.
//!
//! Response guards run at two points. A whole reply is guarded after it is decoded and
//! before the client's body is written. A stream holds output back in windows; each waits for
//! the guards' verdict while the provider is read on, up to the reply's limit.
//!
//! A passthrough reply, from a provider serving the client's own protocol, goes on as the
//! provider sent it and is only read for usage.

use agent_core::strng;
use agent_llm::{LLMResponse, StreamingUsageGuard};
use agent_llm_ir::{
	BufferedReply, HttpReply, IncompleteCall, ObservedReply, OutputChecks, ReplyPlace, ReplyProgress,
	StreamReply, Telemetry, Usage, Verdict, error_reply,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tracing::debug;

use super::guard::{BufferedGuard, ReplyGuards};
use super::{Attempt, PassthroughReply};
use crate::http::{Body, Response};
use crate::llm::policy::StreamingEvaluator;
use crate::llm::policy::streaming_guardrails::{
	DEFAULT_EVAL_THRESHOLD, OVERLAP_BYTES, evaluate_window,
};

/// Converts a whole reply. Returns what the client gets, or a guard's rejection in its place,
/// and what the provider reported.
pub fn buffered(
	attempt: Attempt,
	body: &[u8],
	max_output_bytes: usize,
	guards: Option<ReplyGuards<'_>>,
) -> (Result<HttpReply, Response>, LLMResponse) {
	let mut driver = BufferedReply::new(attempt.pair(max_output_bytes));
	// The whole reply's policy point: guards read and mask each whole text as it is converted.
	let guard = guards.map(|guards| {
		let (guard, rewrites) = BufferedGuard::new(guards);
		driver.set_rewrites(rewrites);
		guard
	});
	if let Err(err) = driver.read(body) {
		debug!("llm-ir: the reply could not be decoded: {err}");
	}
	assert!(
		matches!(driver.progress(), ReplyProgress::Ready),
		"a whole reply sets no output checks"
	);
	let mut response = LLMResponse::default();
	record(driver.usage(), driver.telemetry(), &mut response);
	match guard.and_then(BufferedGuard::finish) {
		Some(rejection) => (Err(rejection), response),
		None => (Ok(driver.into_reply()), response),
	}
}

/// A provider's HTTP error, as the client's protocol says it.
pub fn error(attempt: &Attempt, status: u16, body: &[u8]) -> HttpReply {
	error_reply(
		attempt.context.client,
		attempt.context.upstream,
		status,
		body,
	)
}

/// What a streamed reply yields. `Failed` comes only before any `Data`, and nothing follows it.
pub enum Output {
	Data(Bytes),
	Failed {
		reply: HttpReply,
		headers: Option<::http::HeaderMap>,
	},
}

/// Converts a streamed reply as it arrives: each provider chunk is decoded and the client
/// output it gives is yielded before the next chunk is read, so backpressure reaches the
/// provider. Usage is written to `logger` as it arrives and settled when the stream ends
/// or is dropped.
///
/// With `guards`, text is held back in windows of the legacy streaming guard's size and
/// overlap, and each window waits for every guard's verdict. A block before any output
/// returns the guard's rejection response; after output it ends the stream with a client
/// protocol error carrying the guard's message. Tool calls are not checked,
/// as on legacy, and a call the provider cut short is sent as it is.
pub fn stream(
	attempt: Attempt,
	upstream: Body,
	max_event_bytes: usize,
	logger: StreamingUsageGuard,
	mut guards: Vec<Box<dyn StreamingEvaluator>>,
) -> impl Stream<Item = Output> + Send {
	async_stream::stream! {
		let mut driver = StreamReply::new(attempt.pair(max_event_bytes), max_event_bytes);
		if !guards.is_empty() {
			driver.set_checks(OutputChecks {
				window_bytes: DEFAULT_EVAL_THRESHOLD,
				overlap_bytes: OVERLAP_BYTES,
				max_held_bytes: max_event_bytes,
				incomplete_call: IncompleteCall::Send,
			});
		}
		let mut chunks = upstream.into_boxed().into_data_stream();
		let mut timing = Timing::default();
		let mut ended = false;
		let mut rejection = None;
		loop {
			if !ended {
				ended = driver.is_finished() || read(&mut driver, chunks.next().await);
			}
			logger.update(|info| record(driver.usage(), driver.telemetry(), &mut info.response));
			// The stream's policy point: answer each held window before the next chunk is read
			// here, reading the provider on while a verdict is pending.
			while let ReplyProgress::Waiting(held) = driver.progress() {
				let check = held.check();
				if matches!(check.place, ReplyPlace::ToolCall(_)) {
					let _ = held.resolve(Verdict::Release);
					continue;
				}
				let window = check.text.to_string();
				let evaluation = evaluate_window(&mut guards, &window);
				tokio::pin!(evaluation);
				let blocked = loop {
					tokio::select! {
						blocked = &mut evaluation => break Some(blocked),
						chunk = chunks.next(), if !ended => {
							ended = driver.is_finished() || read(&mut driver, chunk);
							logger.update(|info| record(driver.usage(), driver.telemetry(), &mut info.response));
							// Past its limit, the held output fails the reply, which needs no verdict.
							if matches!(driver.progress(), ReplyProgress::Ready) {
								break None;
							}
						}
					}
				};
				let Some(blocked) = blocked else {
					break;
				};
				let ReplyProgress::Waiting(held) = driver.progress() else {
					unreachable!("the window waits until it is answered");
				};
				let verdict = match blocked {
					Some(response) => {
						let message = String::from_utf8_lossy(response.body()).into_owned();
						rejection = Some(response);
						Verdict::Stop(message)
					},
					None => Verdict::Release,
				};
				let _ = held.resolve(verdict);
				if let Some(data) = driver.take_output() {
					logger.update(|info| timing.record(&mut info.response));
					yield Output::Data(data.into());
				}
			}
			if let Some(data) = driver.take_output() {
				logger.update(|info| timing.record(&mut info.response));
				yield Output::Data(data.into());
			}
			if ended {
				break;
			}
		}
		if let Some(reply) = driver.early_failure() {
			let (reply, headers) = match rejection {
				Some(response) => {
					let (parts, body) = response.into_parts();
					(HttpReply { status: parts.status.as_u16(), body: body.to_vec() }, Some(parts.headers))
				},
				None => (reply, None),
			};
			yield Output::Failed { reply, headers };
		}
		// Settles usage once. Dropping the stream early, as when the client leaves, does the same.
		drop(logger);
	}
}

/// Feeds one read from the provider. Returns whether its input ended, which finishes it.
fn read(driver: &mut StreamReply<'_>, chunk: Option<Result<Bytes, crate::http::Error>>) -> bool {
	let ended = match chunk {
		Some(Ok(chunk)) => {
			if let Err(err) = driver.push(&chunk) {
				debug!("llm-ir: the reply could not be decoded: {err}");
			}
			false
		},
		Some(Err(err)) => {
			debug!("llm-ir: reading the reply failed: {err}");
			driver.fail();
			true
		},
		None => true,
	};
	if (ended || driver.is_finished())
		&& let Err(err) = driver.finish()
	{
		debug!("llm-ir: the reply ended before it could be read: {err}");
	}
	ended || driver.is_finished()
}

/// What the provider reported in a whole passthrough reply.
pub fn observed(passthrough: &PassthroughReply, body: &[u8], max_held_bytes: usize) -> LLMResponse {
	let mut observed = ObservedReply::new(
		passthrough.protocol,
		&passthrough.model,
		max_held_bytes,
		max_held_bytes,
	);
	if let Err(err) = observed.read(body) {
		debug!("llm-ir: the passthrough reply could not be read for usage: {err}");
	}
	let mut response = LLMResponse::default();
	record(observed.usage(), observed.telemetry(), &mut response);
	response
}

/// Sends a streamed passthrough reply on as it arrives, reading each chunk for usage
/// first. Usage is written to `logger` and settled when the stream ends or is dropped.
pub fn observed_stream(
	passthrough: PassthroughReply,
	upstream: Body,
	max_event_bytes: usize,
	logger: StreamingUsageGuard,
) -> impl Stream<Item = Result<Bytes, crate::http::Error>> + Send {
	async_stream::stream! {
		let mut observed = ObservedReply::new(
			passthrough.protocol,
			&passthrough.model,
			max_event_bytes,
			max_event_bytes,
		);
		let mut chunks = upstream.into_boxed().into_data_stream();
		let mut timing = Timing::default();
		while let Some(chunk) = chunks.next().await {
			let chunk = match chunk {
				Ok(chunk) => chunk,
				Err(err) => {
					yield Err(err);
					break;
				},
			};
			// Native items may be unsupported by conversion. Keep reading later chunks
			// for usage while forwarding the provider's bytes unchanged.
			if let Err(err) = observed.push(&chunk) {
				debug!("llm-ir: the passthrough reply could not be read for usage: {err}");
			}
			logger.update(|info| {
				record(observed.usage(), observed.telemetry(), &mut info.response);
				timing.record(&mut info.response);
			});
			yield Ok(chunk);
		}
		if let Err(err) = observed.finish() {
			debug!("llm-ir: the passthrough reply ended before it could be read: {err}");
		}
		drop(logger);
	}
}

/// Map neutral provider accounting into the gateway's response and log convention.
fn record(usage: Usage, telemetry: &Telemetry, response: &mut LLMResponse) {
	response.input_tokens = usage.input_tokens;
	response.output_tokens = usage.output_tokens;
	response.total_tokens = usage.total_tokens;
	response.cached_input_tokens = usage.cached_tokens;
	response.cache_creation_input_tokens = usage.cache_write_tokens;
	response.cache_creation_12h_input_tokens = usage.cache_write_12h_tokens;
	response.reasoning_tokens = usage.reasoning_tokens;
	// Called for every chunk of a stream, so a name is copied only when it changes.
	let model = telemetry.model.as_deref();
	if response.provider_model.as_deref() != model {
		response.provider_model = model.map(strng::new);
	}
	let service_tier = telemetry.service_tier.as_deref();
	if response.service_tier.as_deref() != service_tier {
		response.service_tier = service_tier.map(strng::new);
	}
}

/// When client output goes out, as legacy records it for each token.
#[derive(Default)]
struct Timing {
	last: Option<std::time::Instant>,
}

impl Timing {
	fn record(&mut self, response: &mut LLMResponse) {
		let now = std::time::Instant::now();
		match self.last.replace(now) {
			Some(previous) => response
				.inter_chunk_latencies
				.record(now.duration_since(previous)),
			None => response.first_token = Some(now),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fmt::Write;
	use std::sync::{Arc, Mutex};

	use agent_llm::{
		CacheTokenConvention, LLMInfo, LLMRequest, LLMRequestParams, StreamingUsageReporter,
	};
	use agent_llm_ir::{ClientProtocol, ReplyPlan, UpstreamProtocol};
	use futures::SinkExt;
	use futures::channel::mpsc::Sender;
	use rstest::rstest;
	use serde_json::Value;

	use super::super::AttemptContext;
	use super::*;

	const MESSAGE: &str = r#"{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}"#;
	const USAGE: &str = r#"{"input_tokens":12,"input_tokens_details":{"cached_tokens":4},"output_tokens":5,"output_tokens_details":{"reasoning_tokens":2},"total_tokens":17}"#;

	/// Long enough for a stream that is going to read to do so.
	const WAIT: std::time::Duration = std::time::Duration::from_millis(200);

	fn attempt(stop_sequences: &[&str]) -> Attempt {
		Attempt::new(
			AttemptContext {
				backend: "ns/backend".into(),
				provider: "openai".into(),
				model: "gpt-5".into(),
				client: ClientProtocol::Messages,
				upstream: UpstreamProtocol::Responses {
					safety_identifier: true,
					reasoning_context: false,
				},
				created: 0,
				fallback_reply_id: "reply-1".into(),
			},
			ReplyPlan {
				include_usage: false,
				routes: Vec::new(),
				show_reasoning: false,
				emulated_stop_sequences: stop_sequences.iter().map(ToString::to_string).collect(),
			},
		)
	}

	fn buffered_body() -> String {
		format!(
			r#"{{"id":"resp_1","object":"response","model":"gpt-5","status":"completed","service_tier":"default","output":[{MESSAGE}],"usage":{USAGE}}}"#
		)
	}

	fn sse(events: &[String]) -> String {
		events.iter().fold(String::new(), |mut stream, data| {
			let name = serde_json::from_str::<Value>(data).unwrap()["type"]
				.as_str()
				.unwrap()
				.to_string();
			write!(stream, "event: {name}\ndata: {data}\n\n").unwrap();
			stream
		})
	}

	fn created() -> String {
		r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","object":"response","model":"gpt-5","status":"in_progress","output":[]}}"#.to_string()
	}

	fn text_events() -> Vec<String> {
		vec![
			created(),
			r#"{"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}"#.to_string(),
			r#"{"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#.to_string(),
			r#"{"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello"}"#.to_string(),
			r#"{"type":"response.output_text.delta","sequence_number":4,"item_id":"msg_1","output_index":0,"content_index":0,"delta":" there"}"#.to_string(),
		]
	}

	fn completed_stream() -> String {
		let mut events = text_events();
		events.extend([
			r#"{"type":"response.output_text.done","sequence_number":5,"item_id":"msg_1","output_index":0,"content_index":0,"text":"Hello there"}"#.to_string(),
			r#"{"type":"response.content_part.done","sequence_number":6,"item_id":"msg_1","output_index":0,"content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]}}"#.to_string(),
			format!(r#"{{"type":"response.output_item.done","sequence_number":7,"output_index":0,"item":{MESSAGE}}}"#),
			format!(
				r#"{{"type":"response.completed","sequence_number":8,"response":{{"id":"resp_1","object":"response","model":"gpt-5","status":"completed","service_tier":"default","output":[{MESSAGE}],"usage":{USAGE}}}}}"#
			),
		]);
		sse(&events)
	}

	/// Records what the stream logs and the usage available at each settlement.
	struct Recorder {
		info: Arc<Mutex<LLMInfo>>,
		settled: Arc<Mutex<Vec<LLMResponse>>>,
	}

	impl StreamingUsageReporter for Recorder {
		fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
			f(&mut self.info.lock().unwrap());
		}

		fn report_usage(&mut self) {
			let response = self.info.lock().unwrap().response.clone();
			self.settled.lock().unwrap().push(response);
		}
	}

	impl Drop for Recorder {
		fn drop(&mut self) {
			self.report_usage();
		}
	}

	struct Run {
		info: Arc<Mutex<LLMInfo>>,
		settled: Arc<Mutex<Vec<LLMResponse>>>,
		output: std::pin::Pin<Box<dyn Stream<Item = Output> + Send>>,
	}

	fn run(body: String, chunk: usize, max_event_bytes: usize) -> Run {
		let chunks: Vec<Result<Bytes, std::convert::Infallible>> = body
			.into_bytes()
			.chunks(chunk)
			.map(|piece| Ok(Bytes::copy_from_slice(piece)))
			.collect();
		start(
			Body::from_stream(futures_util::stream::iter(chunks)),
			max_event_bytes,
		)
	}

	/// A stream fed by the returned sender, which holds `buffer` chunks beyond the one the
	/// stream is waiting for.
	fn live_run(buffer: usize) -> (Sender<Result<Bytes, std::io::Error>>, Run) {
		let (upstream, chunks) = futures::channel::mpsc::channel(buffer);
		(upstream, start(Body::from_stream(chunks), 1 << 20))
	}

	fn start(upstream: Body, max_event_bytes: usize) -> Run {
		start_with(attempt(&[]), upstream, max_event_bytes)
	}

	fn start_with(attempt: Attempt, upstream: Body, max_event_bytes: usize) -> Run {
		start_guarded(attempt, upstream, max_event_bytes, Vec::new())
	}

	fn start_guarded(
		attempt: Attempt,
		upstream: Body,
		max_event_bytes: usize,
		guards: Vec<Box<dyn StreamingEvaluator>>,
	) -> Run {
		let request = LLMRequest {
			input_tokens: None,
			input_format: agent_llm::InputFormat::Messages,
			cache_convention: CacheTokenConvention::default(),
			request_model: "gpt-5".into(),
			original_model: None,
			provider: "openai".into(),
			streaming: true,
			params: LLMRequestParams::default(),
			prompt: None,
			provider_state: None,
		};
		let info = Arc::new(Mutex::new(LLMInfo::new(request, LLMResponse::default())));
		let settled = Arc::new(Mutex::new(Vec::new()));
		let logger = StreamingUsageGuard::new(Box::new(Recorder {
			info: info.clone(),
			settled: settled.clone(),
		}));
		Run {
			info,
			settled,
			output: Box::pin(stream(attempt, upstream, max_event_bytes, logger, guards)),
		}
	}

	async fn collect(
		output: &mut std::pin::Pin<Box<dyn Stream<Item = Output> + Send>>,
	) -> Vec<Output> {
		let mut all = Vec::new();
		while let Some(item) = output.next().await {
			all.push(item);
		}
		all
	}

	fn event_names(data: &[u8]) -> Vec<String> {
		std::str::from_utf8(data)
			.unwrap()
			.lines()
			.filter_map(|line| line.strip_prefix("event: "))
			.map(str::to_string)
			.collect()
	}

	#[rstest]
	#[case::eof(StreamEnd::Eof)]
	#[case::client_leaves(StreamEnd::ClientLeaves)]
	#[case::connection_fails(StreamEnd::ConnectionFails)]
	#[tokio::test]
	async fn passthrough_reads_usage_after_an_unsupported_native_item(
		#[case] ending: StreamEnd,
		#[values(false, true)] with_usage: bool,
	) {
		let info = Arc::new(Mutex::new(LLMInfo::new(
			LLMRequest {
				input_tokens: None,
				input_format: agent_llm::InputFormat::Responses,
				cache_convention: CacheTokenConvention::default(),
				request_model: "gpt-5".into(),
				original_model: None,
				provider: "openai".into(),
				streaming: true,
				params: LLMRequestParams::default(),
				prompt: None,
				provider_state: None,
			},
			LLMResponse::default(),
		)));
		let settled = Arc::new(Mutex::new(Vec::new()));
		let logger = StreamingUsageGuard::new(Box::new(Recorder {
			info: info.clone(),
			settled: settled.clone(),
		}));
		let head = sse(&[
			r#"{"type":"response.created","response":{"id":"resp_1","model":"gpt-5","output":[]}}"#.to_string(),
			r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"tool_search_call","id":"search_1"}}"#.to_string(),
		]);
		let tail = sse(&[format!(
			r#"{{"type":"response.completed","response":{{"id":"resp_1","output":[{{"type":"tool_search_call"}}],"usage":{USAGE}}}}}"#
		)]);
		let head = Bytes::from(head);
		let tail = Bytes::from(tail);
		let (mut sender, chunks) = futures::channel::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
		sender.feed(Ok(head.clone())).await.unwrap();
		let upstream = Body::from_stream(chunks);
		let output = observed_stream(
			PassthroughReply {
				protocol: UpstreamProtocol::Responses {
					safety_identifier: true,
					reasoning_context: false,
				},
				model: "gpt-5".into(),
			},
			upstream,
			1 << 20,
			logger,
		);
		let mut output = Box::pin(output);
		let first = tokio::time::timeout(WAIT, output.next())
			.await
			.expect("forward the native item before the provider sends more")
			.unwrap()
			.unwrap();
		assert_eq!(first, head);
		assert_eq!(info.lock().unwrap().response.input_tokens, None);
		assert_eq!(settled.lock().unwrap().len(), 0);

		if with_usage {
			sender.feed(Ok(tail.clone())).await.unwrap();
			let last = tokio::time::timeout(WAIT, output.next())
				.await
				.expect("forward usage before the provider connection closes")
				.unwrap()
				.unwrap();
			assert_eq!(last, tail);
			let info = info.lock().unwrap();
			assert_eq!(info.response.input_tokens, Some(12));
			assert_eq!(info.response.output_tokens, Some(5));
			assert_eq!(info.response.reasoning_tokens, Some(2));
		}
		assert_eq!(settled.lock().unwrap().len(), 0);
		match ending {
			StreamEnd::Eof => {
				drop(sender);
				assert!(output.next().await.is_none());
			},
			StreamEnd::ClientLeaves => drop(output),
			StreamEnd::ConnectionFails => {
				sender
					.feed(Err(std::io::Error::other("reset")))
					.await
					.unwrap();
				let error = output.next().await.unwrap().unwrap_err();
				assert!(error.to_string().contains("reset"), "{error}");
				assert_eq!(settled.lock().unwrap().len(), 0);
				assert!(output.next().await.is_none());
			},
		}
		let settled = settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].provider_model.as_deref(), Some("gpt-5"));
		assert_eq!(settled[0].input_tokens, with_usage.then_some(12));
		assert_eq!(settled[0].output_tokens, with_usage.then_some(5));
		assert_eq!(settled[0].total_tokens, with_usage.then_some(17));
		assert_eq!(settled[0].cached_input_tokens, with_usage.then_some(4));
		assert_eq!(settled[0].reasoning_tokens, with_usage.then_some(2));
	}

	#[tokio::test]
	async fn a_streamed_reply_is_converted_as_it_arrives() {
		let mut run = run(completed_stream(), 64, 1 << 20);
		let outputs = collect(&mut run.output).await;
		assert!(outputs.len() > 1, "output should go out as it arrives");
		let mut all = Vec::new();
		for output in outputs {
			let Output::Data(data) = output else {
				panic!("expected data");
			};
			all.extend_from_slice(&data);
		}
		assert_eq!(
			event_names(&all),
			[
				"message_start",
				"content_block_start",
				"content_block_delta",
				"content_block_delta",
				"content_block_stop",
				"message_delta",
				"message_stop"
			]
		);
		drop(run.output);
		assert_eq!(run.settled.lock().unwrap().len(), 1);
		let response = run.info.lock().unwrap().response.clone();
		assert_eq!(response.input_tokens, Some(12));
		assert_eq!(response.cached_input_tokens, Some(4));
		assert_eq!(response.output_tokens, Some(5));
		assert_eq!(response.reasoning_tokens, Some(2));
		assert_eq!(response.total_tokens, Some(17));
		assert_eq!(response.provider_model.as_deref(), Some("gpt-5"));
		assert_eq!(response.service_tier.as_deref(), Some("default"));
		assert!(response.first_token.is_some());
	}

	#[tokio::test]
	async fn a_final_event_over_the_limit_is_read_in_pieces() {
		// Only response.completed, which repeats the whole reply, is over 300 bytes.
		let mut run = run(completed_stream(), 64, 300);
		let mut names = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("expected data");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.last().map(String::as_str), Some("message_stop"));
		assert_eq!(run.info.lock().unwrap().response.total_tokens, Some(17));
	}

	#[tokio::test]
	async fn a_failure_before_any_output_is_an_http_error() {
		let failed = format!(
			r#"{{"type":"response.failed","sequence_number":1,"response":{{"id":"resp_1","object":"response","model":"gpt-5","status":"failed","output":[],"error":{{"code":"rate_limit_exceeded","message":"slow down"}},"usage":{USAGE}}}}}"#
		);
		let mut run = run(sse(&[created(), failed]), 1 << 20, 1 << 20);
		let outputs = collect(&mut run.output).await;
		let [Output::Failed { reply, .. }] = outputs.as_slice() else {
			panic!("expected only an HTTP failure");
		};
		assert_eq!(reply.status, 429);
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		assert_eq!(body["type"], "error");
		assert_eq!(run.info.lock().unwrap().response.input_tokens, Some(12));
	}

	#[tokio::test]
	async fn a_failure_after_output_ends_the_stream_with_an_error_event() {
		let mut run = run(sse(&text_events()), 1 << 20, 1 << 20);
		let outputs = collect(&mut run.output).await;
		let mut names = Vec::new();
		for output in outputs {
			let Output::Data(data) = output else {
				panic!("a failure after output must be an event");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.first().map(String::as_str), Some("message_start"));
		assert_eq!(names.last().map(String::as_str), Some("error"));
	}

	#[rstest]
	#[case::before_usage(false)]
	#[case::after_usage(true)]
	#[tokio::test]
	async fn usage_is_settled_once_when_the_client_leaves(#[case] with_usage: bool) {
		let mut run = run(completed_stream(), 64, 1 << 20);
		loop {
			assert!(matches!(run.output.next().await, Some(Output::Data(_))));
			if !with_usage || run.info.lock().unwrap().response.output_tokens.is_some() {
				break;
			}
		}
		assert_eq!(run.settled.lock().unwrap().len(), 0);
		drop(run.output);
		let settled = run.settled.lock().unwrap().clone();
		assert_eq!(settled.len(), 1);
		let response = &settled[0];
		assert_eq!(response.provider_model.as_deref(), Some("gpt-5"));
		assert_eq!(response.input_tokens, with_usage.then_some(12));
		assert_eq!(response.output_tokens, with_usage.then_some(5));
		assert_eq!(response.total_tokens, with_usage.then_some(17));
	}

	#[tokio::test]
	async fn a_stream_cut_off_inside_an_event_ends_with_an_error_event() {
		let stream = completed_stream();
		let cut = sse(&text_events()).len() + 40;
		let mut run = run(stream[..cut].to_string(), 64, 1 << 20);
		let mut names = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("a failure after output must be an event");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.first().map(String::as_str), Some("message_start"));
		assert_eq!(names.last().map(String::as_str), Some("error"));
		drop(run.output);
		assert_eq!(run.settled.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn an_unreadable_event_ends_the_stream_but_usage_is_still_read() {
		let stream = completed_stream();
		let (head, tail) = stream.split_at(sse(&text_events()).len());
		let body = format!("{head}event: response.output_text.delta\ndata: {{not json\n\n{tail}");
		let mut run = run(body, 64, 1 << 20);
		let mut names = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("a failure after output must be an event");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.last().map(String::as_str), Some("error"));
		assert_eq!(names.iter().filter(|name| *name == "error").count(), 1);
		assert_eq!(run.info.lock().unwrap().response.total_tokens, Some(17));
		let settled = run.settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].input_tokens, Some(12));
		assert_eq!(settled[0].output_tokens, Some(5));
		assert_eq!(settled[0].total_tokens, Some(17));
	}

	#[tokio::test]
	async fn usage_is_settled_once_when_the_provider_connection_fails() {
		let (mut upstream, mut run) = live_run(2);
		upstream
			.feed(Ok(Bytes::from(sse(&text_events()))))
			.await
			.unwrap();
		upstream
			.feed(Err(std::io::Error::other("reset")))
			.await
			.unwrap();
		let mut names = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("a failure after output must be an event");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.first().map(String::as_str), Some("message_start"));
		assert_eq!(names.last().map(String::as_str), Some("error"));
		assert_eq!(run.settled.lock().unwrap().len(), 1);
		drop(run.output);
		let settled = run.settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].provider_model.as_deref(), Some("gpt-5"));
		assert_eq!(settled[0].input_tokens, None);
		assert_eq!(settled[0].output_tokens, None);
		assert_eq!(settled[0].total_tokens, None);
	}

	#[tokio::test]
	async fn output_goes_out_while_the_provider_is_still_sending() {
		let stream = completed_stream();
		let (first, rest) = stream.split_at(sse(&text_events()).len());
		let (mut upstream, mut run) = live_run(1);
		upstream
			.feed(Ok(Bytes::from(first.to_string())))
			.await
			.unwrap();
		let output = tokio::time::timeout(WAIT, run.output.next())
			.await
			.expect("output should go out before the provider sends more");
		let Some(Output::Data(data)) = output else {
			panic!("expected data");
		};
		assert_eq!(
			event_names(&data).first().map(String::as_str),
			Some("message_start")
		);
		upstream
			.feed(Ok(Bytes::from(rest.to_string())))
			.await
			.unwrap();
		drop(upstream);
		let mut names = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("expected data");
			};
			names.extend(event_names(&data));
		}
		assert_eq!(names.last().map(String::as_str), Some("message_stop"));
	}

	#[tokio::test]
	async fn the_provider_is_not_read_while_the_client_is_not() {
		let stream = completed_stream();
		let (first, rest) = stream.split_at(sse(&text_events()).len());
		let (second, third) = rest.split_at(rest.len() / 2);
		// The channel holds one chunk beyond what the stream has taken.
		let (mut upstream, mut run) = live_run(0);
		upstream
			.feed(Ok(Bytes::from(first.to_string())))
			.await
			.unwrap();
		assert!(matches!(run.output.next().await, Some(Output::Data(_))));
		upstream
			.feed(Ok(Bytes::from(second.to_string())))
			.await
			.unwrap();
		let third = Bytes::from(third.to_string());
		let sent = tokio::time::timeout(WAIT, upstream.feed(Ok(third.clone()))).await;
		assert!(
			sent.is_err(),
			"the provider was read while the client was not reading"
		);
		// Once the client reads, the provider is read again.
		let reading = run.output.next();
		let (_, sent) = tokio::join!(reading, async {
			tokio::time::timeout(WAIT, upstream.feed(Ok(third))).await
		});
		assert!(sent.is_ok());
	}

	#[test]
	fn a_buffered_reply_is_converted() {
		let (reply, response) = buffered(attempt(&[]), buffered_body().as_bytes(), usize::MAX, None);
		let Ok(reply) = reply else {
			panic!("no guard rejects");
		};
		assert_eq!(reply.status, 200);
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		assert_eq!(body["type"], "message");
		assert_eq!(body["content"][0]["text"], "Hello there");
		assert_eq!(body["stop_reason"], "end_turn");
		assert_eq!(response.input_tokens, Some(12));
		assert_eq!(response.cached_input_tokens, Some(4));
		assert_eq!(response.output_tokens, Some(5));
		assert_eq!(response.provider_model.as_deref(), Some("gpt-5"));
	}

	#[test]
	fn a_buffered_reply_respects_the_output_limit() {
		let (reply, _) = buffered(attempt(&[]), buffered_body().as_bytes(), 1, None);
		let Ok(reply) = reply else {
			panic!("no guard rejects");
		};
		assert_eq!(reply.status, 500);
	}

	#[test]
	fn a_stop_sequence_ends_the_reply_but_not_its_accounting() {
		let (reply, response) = buffered(
			attempt(&["there"]),
			buffered_body().as_bytes(),
			usize::MAX,
			None,
		);
		let Ok(reply) = reply else {
			panic!("no guard rejects");
		};
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		assert_eq!(body["content"][0]["text"], "Hello ");
		assert_eq!(body["stop_reason"], "stop_sequence");
		assert_eq!(body["stop_sequence"], "there");
		assert_eq!(response.output_tokens, Some(5));
	}

	/// Legacy has no Responses-to-Messages error conversion, so the attempt converts its own.
	#[test]
	fn a_provider_http_error_is_the_clients() {
		let body =
			br#"{"error":{"type":"requests","code":"rate_limit_exceeded","message":"Slow down"}}"#;
		let reply = error(&attempt(&[]), 429, body);
		assert_eq!(reply.status, 429);
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		assert_eq!(body["type"], "error");
		assert_eq!(body["error"]["type"], "rate_limit_error");
	}

	#[test]
	fn an_unreadable_buffered_reply_is_a_failure() {
		let (reply, response) = buffered(attempt(&[]), b"not json", usize::MAX, None);
		let Ok(reply) = reply else {
			panic!("no guard rejects");
		};
		assert_eq!(reply.status, 500);
		assert_eq!(response.input_tokens, None);
	}

	#[derive(Clone, Copy)]
	enum StreamEnd {
		Eof,
		ClientLeaves,
		ConnectionFails,
	}

	fn response_guards(guards: Value) -> Vec<crate::llm::policy::ResponseGuard> {
		serde_json::from_value(guards).unwrap()
	}

	fn guard_metric(
		client: &crate::proxy::httpproxy::PolicyClient,
		action: crate::telemetry::metrics::GuardrailAction,
	) -> u64 {
		client
			.inputs
			.metrics
			.guardrail_checks
			.get_or_create(&crate::telemetry::metrics::GuardrailLabels {
				phase: crate::telemetry::metrics::GuardrailPhase::Response,
				action,
			})
			.get()
	}

	#[test]
	fn a_response_guard_masks_a_buffered_reply() {
		use crate::telemetry::metrics::GuardrailAction;
		let guards = response_guards(serde_json::json!([
			{"regex": {"action": "mask", "rules": [{"pattern": "there"}]}}
		]));
		let client = crate::test_helpers::policy_client();
		let log = crate::telemetry::log::GuardrailLog::default();
		let (reply, response) = buffered(
			attempt(&[]),
			buffered_body().as_bytes(),
			usize::MAX,
			Some(ReplyGuards {
				guards: &guards,
				client: &client,
				log: Some(&log),
			}),
		);
		let Ok(reply) = reply else {
			panic!("a mask does not reject");
		};
		let body: Value = serde_json::from_slice(&reply.body).unwrap();
		assert_eq!(body["content"][0]["text"], "Hello <masked>");
		assert_eq!(response.output_tokens, Some(5));
		let entry = &log.take().unwrap()[0];
		assert_eq!(
			(entry.phase.as_str(), entry.guard.as_str()),
			("response", "regex")
		);
		assert_eq!(entry.action, GuardrailAction::Mask);
		assert_eq!(guard_metric(&client, GuardrailAction::Mask), 1);
	}

	/// The reply's usage is still recorded.
	#[tokio::test]
	async fn a_response_guard_replaces_a_buffered_reply_it_rejects() {
		use crate::telemetry::metrics::GuardrailAction;
		let guards = response_guards(serde_json::json!([
			{"regex": {"action": "mask", "rules": [{"pattern": "Hello"}]}},
			{"regex": {"action": "reject", "rules": [{"pattern": "<masked> there"}]},
				"rejection": {"status": 451, "body": "withheld"}},
			{"regex": {"action": "audit", "rules": [{"pattern": "there"}]}}
		]));
		let client = crate::test_helpers::policy_client();
		let log = crate::telemetry::log::GuardrailLog::default();
		let (reply, response) = buffered(
			attempt(&[]),
			buffered_body().as_bytes(),
			usize::MAX,
			Some(ReplyGuards {
				guards: &guards,
				client: &client,
				log: Some(&log),
			}),
		);
		let Err(rejection) = reply else {
			panic!("the second guard sees the first one's mask and rejects");
		};
		assert_eq!(rejection.status(), 451);
		let body = http_body_util::BodyExt::collect(rejection.into_body())
			.await
			.unwrap()
			.to_bytes();
		assert_eq!(body, "withheld");
		assert_eq!(response.output_tokens, Some(5));
		let actions: Vec<_> = log
			.take()
			.unwrap()
			.into_iter()
			.map(|entry| entry.action)
			.collect();
		assert_eq!(actions, [GuardrailAction::Mask, GuardrailAction::Reject]);
		assert_eq!(guard_metric(&client, GuardrailAction::Audit), 0);
	}

	async fn client_text(run: &mut Run) -> String {
		let mut all = Vec::new();
		for output in collect(&mut run.output).await {
			let Output::Data(data) = output else {
				panic!("expected data");
			};
			all.extend_from_slice(&data);
		}
		String::from_utf8(all).unwrap()
	}

	#[rstest]
	#[case::released("nothing matches", None)]
	#[case::stopped("there", Some(451))]
	#[tokio::test]
	async fn a_stream_guard_answers_each_window(#[case] pattern: &str, #[case] status: Option<u16>) {
		use crate::telemetry::metrics::GuardrailAction;
		let client = crate::test_helpers::policy_client();
		let log = crate::telemetry::log::GuardrailLog::default();
		let guards = crate::llm::policy::PromptGuard {
			streaming: crate::llm::policy::PromptGuardStreamingMode::Enabled,
			request: Vec::new(),
			response: response_guards(serde_json::json!([
				{"regex": {"action": "reject", "rules": [{"pattern": pattern}]},
					"rejection": {"status": 451, "body": "Withheld.",
						"headers": {"set": {"x-guard": "blocked"}}}}
			])),
		}
		.begin_streaming_response_guard(&client, &::http::HeaderMap::new(), None, log.clone());
		let upstream = Body::from(completed_stream());
		let mut run = start_guarded(attempt(&[]), upstream, 1 << 20, guards);
		let outputs = collect(&mut run.output).await;
		if let Some(status) = status {
			let [Output::Failed { reply, headers }] = outputs.as_slice() else {
				panic!("a first-window block must be an HTTP error");
			};
			assert_eq!(reply.status, status);
			assert_eq!(reply.body, b"Withheld.");
			assert_eq!(headers.as_ref().unwrap()["x-guard"], "blocked");
			assert_eq!(log.take().unwrap()[0].action, GuardrailAction::Reject);
			assert_eq!(guard_metric(&client, GuardrailAction::Reject), 1);
		} else {
			let mut client = String::new();
			for output in outputs {
				let Output::Data(data) = output else {
					panic!("expected data")
				};
				client.push_str(std::str::from_utf8(&data).unwrap());
			}
			assert_eq!(answer_text(&client), "Hello there", "{client}");
			assert!(client.contains(r#""stop_reason":"end_turn""#), "{client}");
		}
		drop(run.output);
		assert_eq!(run.settled.lock().unwrap().len(), 1);
		assert_eq!(run.info.lock().unwrap().response.output_tokens, Some(5));
	}

	/// What the guard double answers a window with.
	enum Answer {
		Release,
		Block(&'static str),
		/// The guard's call failed, as when it times out.
		Fail,
	}

	/// A streaming guard that answers each window when the test says, so a verdict can be
	/// slow, never come, or come after more of the reply has arrived.
	struct GuardDouble {
		answers: futures::channel::mpsc::UnboundedReceiver<Answer>,
		failure_mode: crate::llm::policy::FailureMode,
		asked: Arc<Mutex<Vec<String>>>,
	}

	struct Guard {
		answers: futures::channel::mpsc::UnboundedSender<Answer>,
		asked: Arc<Mutex<Vec<String>>>,
	}

	fn guard_double(
		failure_mode: crate::llm::policy::FailureMode,
	) -> (Guard, Box<dyn StreamingEvaluator>) {
		let (answers, receiver) = futures::channel::mpsc::unbounded();
		let asked = Arc::new(Mutex::new(Vec::new()));
		let double = GuardDouble {
			answers: receiver,
			failure_mode,
			asked: asked.clone(),
		};
		(Guard { answers, asked }, Box::new(double))
	}

	impl StreamingEvaluator for GuardDouble {
		fn failure_mode(&self) -> crate::llm::policy::FailureMode {
			self.failure_mode
		}

		fn evaluate<'a>(
			&'a mut self,
			window: &'a str,
		) -> futures_util::future::BoxFuture<
			'a,
			anyhow::Result<Option<crate::llm::policy::StreamingGuardrailOutcome>>,
		> {
			self.asked.lock().unwrap().push(window.to_string());
			Box::pin(async move {
				match self.answers.next().await {
					Some(Answer::Release) => Ok(None),
					Some(Answer::Block(message)) => Ok(Some(
						crate::llm::policy::StreamingGuardrailOutcome::Blocked(
							::http::Response::builder()
								.status(451)
								.body(Bytes::from(message))
								.unwrap(),
						),
					)),
					Some(Answer::Fail) => Err(anyhow::anyhow!("the guard did not answer in time")),
					None => std::future::pending().await,
				}
			})
		}
	}

	fn guarded_live_run(
		max_event_bytes: usize,
		failure_mode: crate::llm::policy::FailureMode,
	) -> (Sender<Result<Bytes, std::io::Error>>, Guard, Run) {
		let (upstream, chunks) = futures::channel::mpsc::channel(8);
		let (guard, double) = guard_double(failure_mode);
		let run = start_guarded(
			attempt(&[]),
			Body::from_stream(chunks),
			max_event_bytes,
			vec![double],
		);
		(upstream, guard, run)
	}

	fn answer_text(client: &str) -> String {
		client
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.filter_map(|data| serde_json::from_str::<Value>(data).ok())
			.filter_map(|event| event["delta"]["text"].as_str().map(str::to_string))
			.collect()
	}

	/// A whole reply arrives while its only window waits: nothing goes out, and its usage is
	/// counted, until the verdict.
	#[tokio::test]
	async fn output_waits_for_a_slow_verdict_while_usage_counts() {
		use crate::llm::policy::FailureMode;
		let (mut upstream, guard, mut run) = guarded_live_run(1 << 20, FailureMode::FailClosed);
		upstream
			.feed(Ok(Bytes::from(completed_stream())))
			.await
			.unwrap();
		assert!(
			tokio::time::timeout(WAIT, run.output.next()).await.is_err(),
			"nothing goes out before the verdict"
		);
		assert_eq!(*guard.asked.lock().unwrap(), ["Hello there"]);
		assert_eq!(run.info.lock().unwrap().response.output_tokens, Some(5));
		guard.answers.unbounded_send(Answer::Release).unwrap();
		drop(upstream);
		let client = client_text(&mut run).await;
		assert_eq!(answer_text(&client), "Hello there");
		assert!(client.contains(r#""stop_reason":"end_turn""#), "{client}");
		assert_eq!(run.settled.lock().unwrap().len(), 1);
	}

	/// More of the reply arrives after the window being checked; it goes out only after that
	/// window's verdict.
	#[tokio::test]
	async fn a_verdict_after_more_input_releases_in_order() {
		use crate::llm::policy::FailureMode;
		let mut events = text_events();
		events.truncate(3);
		let delta = |sequence: usize, text: &str| {
			format!(
				r#"{{"type":"response.output_text.delta","sequence_number":{sequence},"item_id":"msg_1","output_index":0,"content_index":0,"delta":"{text}"}}"#
			)
		};
		let first = "a".repeat(DEFAULT_EVAL_THRESHOLD);
		events.push(delta(3, &first));
		let (mut upstream, guard, mut run) = guarded_live_run(1 << 20, FailureMode::FailClosed);
		upstream.feed(Ok(Bytes::from(sse(&events)))).await.unwrap();
		assert!(tokio::time::timeout(WAIT, run.output.next()).await.is_err());
		upstream
			.feed(Ok(Bytes::from(sse(&[delta(4, "b")]))))
			.await
			.unwrap();
		assert!(tokio::time::timeout(WAIT, run.output.next()).await.is_err());
		guard.answers.unbounded_send(Answer::Release).unwrap();
		let Some(Output::Data(data)) = tokio::time::timeout(WAIT, run.output.next()).await.unwrap()
		else {
			panic!("the released window goes out");
		};
		assert_eq!(answer_text(std::str::from_utf8(&data).unwrap()), first);
		assert_eq!(
			guard.asked.lock().unwrap().len(),
			1,
			"the rest has no window yet"
		);
	}

	#[rstest]
	#[case::blocked(
		crate::llm::policy::FailureMode::FailClosed,
		Answer::Block("Withheld."),
		"Withheld.",
		Some(451)
	)]
	#[case::fails_closed(
		crate::llm::policy::FailureMode::FailClosed,
		Answer::Fail,
		"Content blocked by guardrail policy",
		Some(403)
	)]
	#[case::fails_open(
		crate::llm::policy::FailureMode::FailOpen,
		Answer::Fail,
		"Hello there",
		None
	)]
	#[tokio::test]
	async fn the_verdict_and_the_guards_failure_mode_decide_the_stream(
		#[case] failure_mode: crate::llm::policy::FailureMode,
		#[case] answer: Answer,
		#[case] sent: &str,
		#[case] status: Option<u16>,
	) {
		let (mut upstream, guard, mut run) = guarded_live_run(1 << 20, failure_mode);
		upstream
			.feed(Ok(Bytes::from(completed_stream())))
			.await
			.unwrap();
		drop(upstream);
		guard.answers.unbounded_send(answer).unwrap();
		if let Some(status) = status {
			let outputs = collect(&mut run.output).await;
			let [Output::Failed { reply, .. }] = outputs.as_slice() else {
				panic!("a block before output must be an HTTP error");
			};
			assert_eq!(reply.status, status);
			assert_eq!(reply.body, sent.as_bytes());
		} else {
			let client = client_text(&mut run).await;
			assert_eq!(answer_text(&client), sent, "{client}");
			assert!(client.contains(r#""stop_reason":"end_turn""#), "{client}");
		}
		drop(run.output);
		let settled = run.settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].output_tokens, Some(5));
	}

	#[rstest]
	#[case::blocked(Answer::Block("Withheld."), "Withheld.")]
	#[case::fails_closed(Answer::Fail, "Content blocked by guardrail policy")]
	#[tokio::test]
	async fn a_block_after_a_released_window_ends_with_an_error(
		#[case] answer: Answer,
		#[case] message: &str,
	) {
		use crate::llm::policy::FailureMode;
		let first = "a".repeat(DEFAULT_EVAL_THRESHOLD);
		let replace = |stream: String| stream.replace("Hello", &first);
		let prefix = replace(sse(&text_events()[..4]));
		let stream = replace(completed_stream());
		let (mut upstream, guard, mut run) = guarded_live_run(1 << 20, FailureMode::FailClosed);
		upstream
			.feed(Ok(Bytes::from(prefix.clone())))
			.await
			.unwrap();
		guard.answers.unbounded_send(Answer::Release).unwrap();
		let Some(Output::Data(data)) = run.output.next().await else {
			panic!("the first window is released");
		};
		assert_eq!(answer_text(std::str::from_utf8(&data).unwrap()), first);
		upstream
			.feed(Ok(Bytes::from(stream[prefix.len()..].to_string())))
			.await
			.unwrap();
		drop(upstream);
		guard.answers.unbounded_send(answer).unwrap();
		let client = client_text(&mut run).await;
		let events: Vec<Value> = client
			.lines()
			.filter_map(|line| line.strip_prefix("data: "))
			.map(|data| serde_json::from_str(data).unwrap())
			.collect();
		let last = events.last().unwrap();
		assert_eq!(last["type"], "error", "{client}");
		assert_eq!(last["error"]["type"], "invalid_request_error", "{client}");
		assert_eq!(last["error"]["message"], message, "{client}");
		assert!(answer_text(&client).is_empty(), "{client}");
		assert!(!client.contains("message_stop"), "{client}");
		drop(run.output);
		let settled = run.settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].output_tokens, Some(5));
	}

	/// The provider is read on while a verdict is pending, but only up to the reply's limit;
	/// past it the reply fails without waiting for the verdict, and no later window is checked.
	#[tokio::test]
	async fn held_output_past_the_limit_fails_the_reply() {
		use crate::llm::policy::FailureMode;
		let mut events = text_events();
		events.truncate(3);
		let delta = |sequence: usize| {
			format!(
				r#"{{"type":"response.output_text.delta","sequence_number":{sequence},"item_id":"msg_1","output_index":0,"content_index":0,"delta":"{}"}}"#,
				"a".repeat(200)
			)
		};
		events.extend((3..9).map(delta));
		let (mut upstream, guard, mut run) = guarded_live_run(4096, FailureMode::FailClosed);
		upstream.feed(Ok(Bytes::from(sse(&events)))).await.unwrap();
		assert!(tokio::time::timeout(WAIT, run.output.next()).await.is_err());
		assert_eq!(guard.asked.lock().unwrap().len(), 1);
		let more: Vec<String> = (9..40).map(delta).collect();
		upstream.feed(Ok(Bytes::from(sse(&more)))).await.unwrap();
		// A failed reply still reads the provider for usage until it ends.
		drop(upstream);
		let outputs = tokio::time::timeout(WAIT, collect(&mut run.output))
			.await
			.expect("the reply fails without a verdict");
		let [Output::Failed { reply, .. }] = outputs.as_slice() else {
			panic!("nothing went out, so the failure is the HTTP reply");
		};
		assert_eq!(reply.status, 500);
		assert_eq!(guard.asked.lock().unwrap().len(), 1);
		assert_eq!(run.settled.lock().unwrap().len(), 1);
	}

	/// A client that leaves while a verdict is pending settles usage once and drops the
	/// guard's pending call.
	#[tokio::test]
	async fn the_client_leaving_while_a_verdict_is_pending_settles_usage_once() {
		use crate::llm::policy::FailureMode;
		let (mut upstream, guard, mut run) = guarded_live_run(1 << 20, FailureMode::FailClosed);
		upstream
			.feed(Ok(Bytes::from(completed_stream())))
			.await
			.unwrap();
		assert!(tokio::time::timeout(WAIT, run.output.next()).await.is_err());
		drop(run.output);
		assert!(guard.answers.is_closed(), "the guard's call was dropped");
		let settled = run.settled.lock().unwrap();
		assert_eq!(settled.len(), 1);
		assert_eq!(settled[0].output_tokens, Some(5));
	}
}
