//! The legacy converters and the LLM IR on the same inputs, for every protocol pair both serve,
//! including passthroughs. Run with `cargo bench -p agent-llm-ir --bench legacy_vs_ir`.
//!
//! Each side goes from client bytes to provider bytes, or from provider bytes to client bytes,
//! the way the gateway calls it. Setup checks both sides carry the same content before timing.
//! The inputs are the IR suite's; see `support/workload.rs`. What this does
//! not compare is listed in `README.md`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::borrow::Cow;
use std::convert::Infallible;
use std::fmt;
use std::sync::{Arc, Mutex};

use agent_core::strng;
use agent_llm::conversion::{completions, messages, openai_compat, responses, vertex_gemini};
use agent_llm::{
	CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, LogContentFields,
	ResponseType, StreamingUsageGuard, StreamingUsageReporter, types,
};
use agent_llm_ir::{
	BufferedReply, Delivery, ObservedReply, RenderSettings, RequestFields, StreamReply, prepare,
};
use bytes::Bytes;
use divan::{Bencher, black_box};
use workload::{Conversation, LIMIT, Pair, Protocol, Size, conversation, reply_text};

#[path = "support/workload.rs"]
mod workload;

#[path = "support/sse.rs"]
mod sse;

// #[global_allocator]
// static ALLOC: divan::AllocProfiler<tikv_jemallocator::Jemalloc> =
// 	divan::AllocProfiler::new(tikv_jemallocator::Jemalloc);

const REQUEST_SIZES: &[Size] = &[
	Size(4 << 10),
	Size(32 << 10),
	Size(256 << 10),
	Size(1 << 20),
	Size(4 << 20),
];
const REPLY_SIZES: &[Size] = &[Size(1 << 10), Size(16 << 10), Size(128 << 10)];

/// Every pair the legacy converters serve without Bedrock or Vertex. The IR serves more; those
/// are in its own suite.
const PAIRS: &[Pair] = &[
	Pair::new(Protocol::Messages, Protocol::Responses),
	Pair::new(Protocol::Messages, Protocol::Chat),
	Pair::new(Protocol::Chat, Protocol::Messages),
	Pair::new(Protocol::Chat, Protocol::Gemini),
	Pair::new(Protocol::Responses, Protocol::Chat),
	Pair::new(Protocol::Messages, Protocol::Messages),
	Pair::new(Protocol::Responses, Protocol::Responses),
	Pair::new(Protocol::Chat, Protocol::Chat),
	Pair::new(Protocol::Gemini, Protocol::Gemini),
];

fn main() {
	divan::main();
}

#[divan::bench(args = cases(REQUEST_SIZES), max_time = 3)]
fn request_legacy(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	case.check_request(turns, &legacy_request(case.pair, &body));
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| legacy_request(case.pair, black_box(&body)));
}

#[divan::bench(args = cases(REQUEST_SIZES), max_time = 3)]
fn request_ir(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	case.check_request(turns, &ir_request(case.pair, &body));
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| ir_request(case.pair, black_box(&body)));
}

#[divan::bench(args = cases(REPLY_SIZES), max_time = 3)]
fn reply_buffered_legacy(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let body = Bytes::from(case.pair.provider.buffered(&text));
	case.check_buffered(&text, &legacy_buffered(case.pair, &body));
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| legacy_buffered(case.pair, black_box(&body)));
}

#[divan::bench(args = cases(REPLY_SIZES), max_time = 3)]
fn reply_buffered_ir(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let body = Bytes::from(case.pair.provider.buffered(&text));
	case.check_buffered(&text, &ir_buffered(case.pair, &body));
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| ir_buffered(case.pair, black_box(&body)));
}

#[divan::bench(args = cases(REPLY_SIZES), max_time = 3)]
fn reply_stream_legacy(b: Bencher, case: &Case) {
	let runtime = runtime();
	let _runtime = runtime.enter();
	let text = reply_text(case.size.0);
	let events = events(case.pair, &text);
	let mut out = Vec::new();
	drain(legacy_stream(case.pair, body_of(&events)), |data| {
		out.extend_from_slice(data)
	});
	case.check_stream(&text, &out);
	b.counter(divan::counter::BytesCount::new(stream_len(&events)))
		.with_inputs(|| body_of(&events))
		.bench_local_values(|body| sent(legacy_stream(case.pair, body)));
}

#[divan::bench(args = cases(REPLY_SIZES), max_time = 3)]
fn reply_stream_ir(b: Bencher, case: &Case) {
	let runtime = runtime();
	let _runtime = runtime.enter();
	let text = reply_text(case.size.0);
	let events = events(case.pair, &text);
	let mut out = Vec::new();
	drain(ir_stream(case.pair, body_of(&events)), |data| {
		out.extend_from_slice(data)
	});
	case.check_stream(&text, &out);
	b.counter(divan::counter::BytesCount::new(stream_len(&events)))
		.with_inputs(|| body_of(&events))
		.bench_local_values(|body| sent(ir_stream(case.pair, body)));
}

struct Case {
	pair: Pair,
	size: Size,
}

impl fmt::Display for Case {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}/{}", self.pair, self.size)
	}
}

impl Case {
	fn conversation(&self) -> Conversation {
		conversation(self.pair.client, self.pair.model(), self.size.0)
	}

	fn check_request(&self, turns: usize, out: &[u8]) {
		workload::check_request(&self.to_string(), turns, out);
	}

	fn check_buffered(&self, text: &str, out: &[u8]) {
		workload::check_buffered(&self.to_string(), self.pair.client, text, out);
	}

	fn check_stream(&self, text: &str, out: &[u8]) {
		workload::check_stream(&self.to_string(), self.pair.client, text, out);
	}
}

fn cases(sizes: &'static [Size]) -> Vec<Case> {
	PAIRS
		.iter()
		.flat_map(|&pair| sizes.iter().map(move |&size| Case { pair, size }))
		.collect()
}

// --- Requests ---

/// As the gateway's legacy path: parse the client's typed request, translate it, and write it.
/// A passthrough is parsed and written again in the same protocol.
fn legacy_request(pair: Pair, body: &[u8]) -> Vec<u8> {
	use Protocol::*;
	match (pair.client, pair.provider) {
		(Messages, Responses) => {
			let req: types::messages::Request = serde_json::from_slice(body).unwrap();
			let translated = responses::from_messages::translate_request(&req).unwrap();
			serde_json::to_vec(&translated).unwrap()
		},
		(Messages, Chat) => {
			let req: types::messages::Request = serde_json::from_slice(body).unwrap();
			let mut translated = completions::from_messages::translate_request(&req).unwrap();
			translated.strip_tool_call_extra_content();
			serde_json::to_vec(&translated).unwrap()
		},
		(Chat, Messages) => {
			let req: types::completions::Request = serde_json::from_slice(body).unwrap();
			messages::from_completions::translate(&req, None, None).unwrap()
		},
		(Chat, Gemini) => {
			let req: types::completions::Request = serde_json::from_slice(body).unwrap();
			vertex_gemini::from_completions::translate(&req, false).unwrap()
		},
		(Responses, Chat) => {
			let req: types::responses::Request = serde_json::from_slice(body).unwrap();
			let mut translated = openai_compat::from_responses::translate_request(&req)
				.unwrap()
				.request;
			translated.strip_tool_call_extra_content();
			serde_json::to_vec(&translated).unwrap()
		},
		(Messages, Messages) => rewrite::<types::messages::Request>(body),
		(Responses, Responses) => rewrite::<types::responses::Request>(body),
		(Chat, Chat) => rewrite::<types::completions::Request>(body),
		(Gemini, Gemini) => {
			let req: types::gemini::Request = serde_json::from_slice(body).unwrap();
			serde_json::to_vec(&req.inner).unwrap()
		},
		_ => unreachable!("{pair}: not served by legacy"),
	}
}

fn rewrite<T: serde::de::DeserializeOwned + serde::Serialize>(body: &[u8]) -> Vec<u8> {
	let req: T = serde_json::from_slice(body).unwrap();
	serde_json::to_vec(&req).unwrap()
}

/// As the gateway's IR path. A passthrough reads the model and the settings it logs, and sends
/// the client's bytes on without copying them.
fn ir_request(pair: Pair, body: &[u8]) -> Cow<'_, [u8]> {
	if pair.is_passthrough() {
		let fields = RequestFields::read(body).unwrap();
		black_box(fields.model().unwrap());
		black_box(
			fields
				.read_passthrough(pair.client.endpoint(Delivery::Buffered))
				.unwrap(),
		);
		return Cow::Borrowed(body);
	}
	Cow::Owned(
		prepare(body, pair.client.client(), pair.upstream())
			.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
			.unwrap()
			.body
			.into_bytes(),
	)
}

// --- Buffered replies ---

/// As the gateway's legacy path: translate the reply, or parse it in a passthrough, read its
/// usage, and write it.
fn legacy_buffered(pair: Pair, body: &Bytes) -> Vec<u8> {
	use Protocol::*;
	let reply: Box<dyn ResponseType> = match (pair.client, pair.provider) {
		(Messages, Responses) => responses::from_messages::translate_response(body).unwrap(),
		(Messages, Chat) => completions::from_messages::translate_response(body, pair.model()).unwrap(),
		(Chat, Messages) => messages::from_completions::translate_response(body).unwrap(),
		(Chat, Gemini) => vertex_gemini::to_completions::translate_response(body).unwrap(),
		(Responses, Chat) => {
			openai_compat::to_responses::translate_response(body, pair.model(), None).unwrap()
		},
		(Messages, Messages) => parse::<types::messages::Response>(body),
		(Responses, Responses) => parse::<types::responses::Response>(body),
		(Chat, Chat) => parse::<types::completions::Response>(body),
		(Gemini, Gemini) => parse::<types::gemini::Response>(body),
		_ => unreachable!("{pair}: not served by legacy"),
	};
	black_box(reply.to_llm_response(LogContentFields::default()));
	reply.serialize().unwrap()
}

fn parse<T: ResponseType + serde::de::DeserializeOwned + 'static>(
	body: &Bytes,
) -> Box<dyn ResponseType> {
	Box::new(serde_json::from_slice::<T>(body).unwrap())
}

/// As the gateway's IR path; see `ir::reply`. A passthrough is read for usage and sent on
/// without copying it.
fn ir_buffered(pair: Pair, body: &Bytes) -> Bytes {
	if pair.is_passthrough() {
		let mut observed = ObservedReply::new(pair.provider.upstream(), pair.model(), LIMIT, LIMIT);
		observed.read(body).unwrap();
		black_box(observed.usage());
		return body.clone();
	}
	let mut driver = BufferedReply::new(pair.reply_context());
	driver.read(body).unwrap();
	black_box(driver.usage());
	let reply = driver.into_reply();
	assert_eq!(reply.status, 200);
	Bytes::from(reply.body)
}

// --- Streamed replies ---

/// A provider's stream, one event per transport read.
fn events(pair: Pair, text: &str) -> Vec<Bytes> {
	pair
		.provider
		.stream(text)
		.into_iter()
		.map(Bytes::from)
		.collect()
}

/// As the gateway's legacy path: the converter, or the passthrough reader, wraps the provider
/// body.
fn legacy_stream(pair: Pair, body: agent_http::Body) -> agent_http::Body {
	use Protocol::*;
	let guard = usage_guard(pair.client);
	let log = LogContentFields::default();
	match (pair.client, pair.provider) {
		(Messages, Responses) => responses::from_messages::translate_stream(body, LIMIT, guard, log),
		(Messages, Chat) => completions::from_messages::translate_stream(body, LIMIT, guard, log),
		(Chat, Messages) => messages::from_completions::translate_stream(body, LIMIT, guard, log),
		(Chat, Gemini) => vertex_gemini::to_completions::translate_stream(
			body,
			LIMIT,
			strng::new(pair.model()),
			guard,
			log,
		),
		(Responses, Chat) => {
			openai_compat::to_responses::translate_stream(body, LIMIT, guard, log, None)
		},
		(Messages, Messages) => messages::passthrough_stream(body, LIMIT, guard, log),
		(Responses, Responses) => responses::passthrough_stream(body, LIMIT, guard, log),
		(Chat, Chat) => {
			completions::passthrough_stream(guard, log, ::http::Response::new(body)).into_body()
		},
		(Gemini, Gemini) => vertex_gemini::passthrough_stream(body, LIMIT, guard, log),
		_ => unreachable!("{pair}: not served by legacy"),
	}
}

/// As the gateway's IR path, without output checks; see `ir::reply`. Each provider chunk is
/// pushed, usage recorded, and its output yielded as a body frame. A passthrough yields the
/// provider's chunk after reading it for usage.
fn ir_stream(pair: Pair, body: agent_http::Body) -> agent_http::Body {
	let guard = usage_guard(pair.client);
	if pair.is_passthrough() {
		return agent_http::Body::from_stream(async_stream::stream! {
				let mut observed = ObservedReply::new(pair.provider.upstream(), pair.model(), LIMIT, LIMIT);
				let mut chunks = body.into_boxed().into_data_stream();
				while let Some(chunk) = futures_util::StreamExt::next(&mut chunks).await {
						let chunk = chunk.unwrap();
						observed.push(&chunk).unwrap();
						let usage = observed.usage();
						guard.update(|info| record(usage, info));
						yield Ok::<_, Infallible>(chunk);
				}
				observed.finish().unwrap();
		});
	}
	agent_http::Body::from_stream(async_stream::stream! {
			let mut driver = StreamReply::new(pair.reply_context(), LIMIT);
			let mut chunks = body.into_boxed().into_data_stream();
			loop {
					let ended = driver.is_finished() || match futures_util::StreamExt::next(&mut chunks).await {
							Some(chunk) => {
									driver.push(&chunk.unwrap()).unwrap();
									false
							},
							None => true,
					};
					if ended {
							driver.finish().unwrap();
					}
					let usage = driver.usage();
					guard.update(|info| record(usage, info));
					if let Some(data) = driver.take_output() {
							yield Ok::<_, Infallible>(Bytes::from(data));
					}
					if ended {
							break;
					}
			}
			assert!(driver.early_failure().is_none());
	})
}

const fn record(usage: agent_llm_ir::Usage, info: &mut LLMInfo) {
	info.response.input_tokens = usage.input_tokens;
	info.response.output_tokens = usage.output_tokens;
}

/// The bytes a client is sent, counted as each frame is dropped, as a socket write would. Keeping
/// them would make the client's whole reply each side's peak memory.
fn sent(body: agent_http::Body) -> usize {
	let mut sent = 0;
	drain(body, |data| sent += data.len());
	sent
}

/// Reads a converted body to its end on this thread, polling again whenever it is pending,
/// so the time is conversion work rather than runtime scheduling. Legacy wakes itself and
/// returns pending after each provider event; under a runtime each of those is a trip through
/// the scheduler. Every input is ready, so nothing waits.
fn drain(mut body: agent_http::Body, mut write: impl FnMut(&[u8])) {
	use http_body::Body as _;
	let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
	loop {
		match std::pin::Pin::new(&mut body).poll_frame(&mut cx) {
			std::task::Poll::Ready(Some(frame)) => {
				if let Ok(data) = frame.unwrap().into_data() {
					write(&data);
				}
			},
			std::task::Poll::Ready(None) => return,
			std::task::Poll::Pending => {},
		}
	}
}

/// Provider events arrive one per read, as a provider flushes each token.
fn body_of(events: &[Bytes]) -> agent_http::Body {
	let events = events.to_vec();
	agent_http::Body::from_stream(futures_util::stream::iter(
		events.into_iter().map(Ok::<_, Infallible>),
	))
}

fn stream_len(events: &[Bytes]) -> usize {
	events.iter().map(Bytes::len).sum()
}

/// The body reads timers, so polling needs a runtime context, though nothing is scheduled on it.
fn runtime() -> tokio::runtime::Runtime {
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
}

/// Legacy's stream records usage through a guard, as in the gateway. The IR side records the
/// same way, so both pay for it. The guard settles usage when the body is dropped.
fn usage_guard(client: Protocol) -> StreamingUsageGuard {
	let input_format = match client {
		Protocol::Messages => InputFormat::Messages,
		Protocol::Responses => InputFormat::Responses,
		Protocol::Chat => InputFormat::Completions,
		Protocol::Gemini => InputFormat::Gemini,
	};
	let info = Arc::new(Mutex::new(LLMInfo::new(
		LLMRequest {
			input_tokens: None,
			input_format,
			cache_convention: CacheTokenConvention::pending(),
			request_model: strng::literal!("bench"),
			original_model: None,
			provider: Default::default(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		LLMResponse::default(),
	)));
	StreamingUsageGuard::new(Box::new(Reporter { info }))
}

struct Reporter {
	info: Arc<Mutex<LLMInfo>>,
}

impl StreamingUsageReporter for Reporter {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		f(&mut self.info.lock().unwrap());
	}

	fn report_usage(&mut self) {}
}
