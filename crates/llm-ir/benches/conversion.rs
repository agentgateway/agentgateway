//! Agent traffic through the IR for every pair of protocols, converted and passed through.
//! Run with `cargo bench -p agent-llm-ir --bench conversion`.
//!
//! Each case is called the way the gateway calls the crate, with inputs from `support/workload.rs`.
//! Setup checks the output carries all the content before timing.

#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::fmt;

use agent_llm_ir::{
	BufferedReply, Delivery, IncompleteCall, ObservedReply, OutputChecks, RenderSettings,
	ReplyProgress, ReplyRewrites, RequestFields, StreamReply, Verdict, prepare,
};
use divan::{Bencher, black_box};
use workload::{
	Conversation, LIMIT, PROTOCOLS, Pair, RequestWorkload, Size, conversation, conversation_workload,
	reply_text,
};

#[path = "support/workload.rs"]
mod workload;

#[path = "support/rewrite.rs"]
mod rewrite;

use rewrite::Replace;

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

const REQUEST_SIZES: &[Size] = &[Size(4 << 10), Size(64 << 10), Size(1 << 20)];
const REPLY_SIZES: &[Size] = &[Size(1 << 10), Size(16 << 10), Size(128 << 10)];
const POLICY_REQUEST_SIZE: Size = Size(64 << 10);
const POLICY_REPLY_SIZE: Size = Size(16 << 10);

fn main() {
	divan::main();
}

/// Client bytes to provider bytes.
#[divan::bench(args = cases(REQUEST_SIZES, false), max_time = 2)]
fn request(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	workload::check_request(
		&case.to_string(),
		turns,
		convert(case.pair, &body).as_bytes(),
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| convert(case.pair, black_box(&body)));
}

/// What the gateway reads from a request it sends on unchanged: the model, then the settings
/// it records. The body itself is forwarded as is.
#[divan::bench(args = cases(REQUEST_SIZES, true), max_time = 2)]
fn request_passthrough_read_only(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	workload::check_request(&case.to_string(), turns, &body);
	read_passthrough(case.pair, &body);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| read_passthrough(case.pair, black_box(&body)));
}

/// A converted request whose text policy visits every text and changes nothing.
#[divan::bench(args = policy_request_cases(), max_time = 2)]
fn request_edit_no_match(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	workload::check_request(
		&case.to_string(),
		turns,
		&convert_with_edit(case.pair, &body, None),
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| convert_with_edit(case.pair, black_box(&body), None));
}

/// A converted request whose text policy replaces one token in the user's question.
#[divan::bench(args = policy_request_cases(), max_time = 2)]
fn request_edit_replace(b: Bencher, case: &Case) {
	let Conversation { body, turns } = case.conversation();
	let out = convert_with_edit(case.pair, &body, Some(("gzip-compressed", "encoded")));
	workload::check_request(&case.to_string(), turns, &out);
	assert!(std::str::from_utf8(&out).unwrap().contains("encoded"));
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| {
			convert_with_edit(
				case.pair,
				black_box(&body),
				Some(("gzip-compressed", "encoded")),
			)
		});
}

/// Representative request shapes on one converted route, outside the full pair matrix.
#[divan::bench(args = representative_workload_cases(), max_time = 2)]
fn request_representative_workload(b: Bencher, case: &WorkloadCase) {
	let Conversation { body, turns } = case.conversation();
	let out = convert(case.pair, &body);
	workload::check_request_lines(
		&case.to_string(),
		turns,
		case.lines_per_turn(),
		out.as_bytes(),
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| convert(case.pair, black_box(&body)));
}

/// A whole provider body to the client's whole body, with usage read.
#[divan::bench(args = cases(REPLY_SIZES, false), max_time = 2)]
fn reply_buffered(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let body = case.pair.provider.buffered(&text);
	workload::check_buffered(
		&case.to_string(),
		case.pair.client,
		&text,
		&buffered(case.pair, &body),
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| buffered(case.pair, black_box(&body)));
}

#[divan::bench(args = policy_reply_cases(), max_time = 2)]
fn reply_buffered_rewrite_no_match(b: Bencher, case: &Case) {
	buffered_rewrite_bench(b, case, "not in the fixture", "unused");
}

#[divan::bench(args = policy_reply_cases(), max_time = 2)]
fn reply_buffered_rewrite_replace(b: Bencher, case: &Case) {
	buffered_rewrite_bench(b, case, "gzip", "encoded");
}

/// A provider stream to the client's stream, one provider event per read, with usage read and
/// output taken after each.
#[divan::bench(args = cases(REPLY_SIZES, false), max_time = 2)]
fn reply_stream(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let events = case.pair.provider.stream(&text);
	let mut out = Vec::new();
	stream(case.pair, &events, |data| out.extend_from_slice(data));
	workload::check_stream(&case.to_string(), case.pair.client, &text, &out);
	b.counter(divan::counter::BytesCount::new(stream_len(&events)))
		.bench_local(|| {
			let mut sent = 0;
			stream(case.pair, black_box(&events), |data| sent += data.len());
			sent
		});
}

/// Stream policy windows that are all released.
#[divan::bench(args = policy_reply_cases(), max_time = 2)]
fn reply_stream_checks_release(b: Bencher, case: &Case) {
	stream_policy_bench(b, case, false);
}

/// A reply rewrite followed by stream policy windows, in production order.
#[divan::bench(args = policy_reply_cases(), max_time = 2)]
fn reply_stream_rewrite_and_checks(b: Bencher, case: &Case) {
	stream_policy_bench(b, case, true);
}

/// A whole passthrough reply, read only for usage.
#[divan::bench(args = cases(REPLY_SIZES, true), max_time = 2)]
fn reply_passthrough_buffered(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let body = case.pair.provider.buffered(&text);
	assert_eq!(
		observe_buffered(case.pair, &body),
		Some(300),
		"{case}: usage"
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| observe_buffered(case.pair, black_box(&body)));
}

/// A streamed passthrough reply, read for usage as each provider event arrives.
#[divan::bench(args = cases(REPLY_SIZES, true), max_time = 2)]
fn reply_passthrough_stream(b: Bencher, case: &Case) {
	let text = reply_text(case.size.0);
	let events = case.pair.provider.stream(&text);
	assert_eq!(
		observe_stream(case.pair, &events),
		Some(300),
		"{case}: usage"
	);
	b.counter(divan::counter::BytesCount::new(stream_len(&events)))
		.bench_local(|| observe_stream(case.pair, black_box(&events)));
}

struct Case {
	pair: Pair,
	size: Size,
}

struct WorkloadCase {
	pair: Pair,
	size: Size,
	workload: RequestWorkload,
}

impl fmt::Display for WorkloadCase {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}/{}/{}", self.pair, self.workload, self.size)
	}
}

impl WorkloadCase {
	fn conversation(&self) -> Conversation {
		conversation_workload(
			self.pair.client,
			self.pair.model(),
			self.size.0,
			self.workload,
		)
	}

	fn lines_per_turn(&self) -> usize {
		if self.workload == RequestWorkload::ManySmallTurns {
			4
		} else {
			workload::FILE_LINES_PER_TURN
		}
	}
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
}

/// Every pair of different protocols, or every passthrough, at each size.
fn cases(sizes: &'static [Size], passthrough: bool) -> Vec<Case> {
	let mut cases = Vec::new();
	for &client in PROTOCOLS {
		for &provider in PROTOCOLS {
			let pair = Pair::new(client, provider);
			if pair.is_passthrough() != passthrough {
				continue;
			}
			for &size in sizes {
				cases.push(Case { pair, size });
			}
		}
	}
	cases
}

fn policy_request_cases() -> Vec<Case> {
	vec![Case {
		pair: Pair::new(workload::Protocol::Messages, workload::Protocol::Responses),
		size: POLICY_REQUEST_SIZE,
	}]
}

fn policy_reply_cases() -> Vec<Case> {
	vec![Case {
		pair: Pair::new(workload::Protocol::Messages, workload::Protocol::Responses),
		size: POLICY_REPLY_SIZE,
	}]
}

fn representative_workload_cases() -> Vec<WorkloadCase> {
	let pair = Pair::new(workload::Protocol::Messages, workload::Protocol::Responses);
	[
		RequestWorkload::Standard,
		RequestWorkload::EscapedText,
		RequestWorkload::SchemaHeavy,
		RequestWorkload::ManySmallTurns,
		RequestWorkload::ReasoningSettings,
	]
	.into_iter()
	.map(|workload| WorkloadCase {
		pair,
		size: POLICY_REQUEST_SIZE,
		workload,
	})
	.collect()
}

fn convert(pair: Pair, body: &[u8]) -> String {
	prepare(body, pair.client.client(), pair.upstream())
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.unwrap()
		.body
}

fn convert_with_edit(pair: Pair, body: &[u8], replacement: Option<(&str, &str)>) -> Vec<u8> {
	let mut prepared = prepare(body, pair.client.client(), pair.upstream()).unwrap();
	prepared.conversation_mut().edit_texts(|mut text| {
		let current = text.read();
		let Some((from, to)) = replacement else {
			black_box(current.contains("not in the fixture"));
			return;
		};
		if current.contains(from) {
			text.replace(current.replace(from, to));
		}
	});
	prepared
		.render(RenderSettings::ModelDefaults)
		.unwrap()
		.body
		.into_bytes()
}

fn buffered_rewrite_bench(b: Bencher, case: &Case, from: &'static str, to: &'static str) {
	let text = reply_text(case.size.0);
	let expected = text.replace(from, to);
	let body = case.pair.provider.buffered(&text);
	workload::check_buffered(
		&case.to_string(),
		case.pair.client,
		&expected,
		&buffered_with_rewrite(case.pair, &body, from, to),
	);
	b.counter(divan::counter::BytesCount::new(body.len()))
		.bench_local(|| buffered_with_rewrite(case.pair, black_box(&body), from, to));
}

fn buffered_with_rewrite(pair: Pair, body: &[u8], from: &'static str, to: &'static str) -> Vec<u8> {
	let mut driver = BufferedReply::new(pair.reply_context());
	driver.set_rewrites(ReplyRewrites::new(vec![Box::new(Replace::new(from, to))]));
	driver.read(body).unwrap();
	driver.into_reply().body
}

fn stream_policy_bench(b: Bencher, case: &Case, rewrite: bool) {
	let text = reply_text(case.size.0);
	let expected = if rewrite {
		text.replace("body", "BODY")
	} else {
		text.clone()
	};
	let events = case.pair.provider.stream(&text);
	let mut out = Vec::new();
	stream_with_policy(case.pair, &events, rewrite, |data| {
		out.extend_from_slice(data)
	});
	workload::check_stream(&case.to_string(), case.pair.client, &expected, &out);
	b.counter(divan::counter::BytesCount::new(stream_len(&events)))
		.bench_local(|| {
			let mut sent = 0;
			stream_with_policy(case.pair, black_box(&events), rewrite, |data| {
				sent += data.len()
			});
			sent
		});
}

fn stream_with_policy(pair: Pair, events: &[Vec<u8>], rewrite: bool, mut write: impl FnMut(&[u8])) {
	let mut driver = StreamReply::new(pair.reply_context(), LIMIT);
	if rewrite {
		driver.set_rewrites(ReplyRewrites::new(vec![Box::new(Replace::new(
			"body", "BODY",
		))]));
	}
	driver.set_checks(OutputChecks {
		window_bytes: 1024,
		overlap_bytes: 256,
		max_held_bytes: LIMIT,
		incomplete_call: IncompleteCall::Drop,
	});
	for event in events {
		driver.push(event).unwrap();
		release_checks(&mut driver);
		if let Some(data) = driver.take_output() {
			write(&data);
		}
	}
	driver.finish().unwrap();
	release_checks(&mut driver);
	if let Some(data) = driver.take_output() {
		write(&data);
	}
}

fn release_checks(driver: &mut StreamReply<'_>) {
	let mut progress = driver.progress();
	while let ReplyProgress::Waiting(held) = progress {
		black_box(held.check().text);
		progress = held.resolve(Verdict::Release);
	}
}

/// As the gateway's passthrough: the model for routing, then the settings for the request log.
fn read_passthrough(pair: Pair, body: &[u8]) {
	let fields = RequestFields::read(body).unwrap();
	black_box(fields.model().unwrap());
	black_box(
		fields
			.read_passthrough(pair.client.endpoint(Delivery::Buffered))
			.unwrap(),
	);
}

/// As the gateway's buffered reply: read the body, record usage, write the client's reply.
fn buffered(pair: Pair, body: &[u8]) -> Vec<u8> {
	let mut driver = BufferedReply::new(pair.reply_context());
	driver.read(body).unwrap();
	black_box(driver.usage());
	let reply = driver.into_reply();
	assert_eq!(reply.status, 200);
	reply.body
}

/// As the gateway's streamed reply, without the async body around it. Each piece of client
/// output goes to `write` and is dropped, as a socket write would.
fn stream(pair: Pair, events: &[Vec<u8>], mut write: impl FnMut(&[u8])) {
	let mut driver = StreamReply::new(pair.reply_context(), LIMIT);
	for event in events {
		driver.push(event).unwrap();
		black_box(driver.usage());
		if let Some(data) = driver.take_output() {
			write(&data);
		}
	}
	driver.finish().unwrap();
	if let Some(data) = driver.take_output() {
		write(&data);
	}
	assert!(driver.early_failure().is_none());
}

fn observe_buffered(pair: Pair, body: &[u8]) -> Option<u64> {
	let mut observed = ObservedReply::new(pair.provider.upstream(), pair.model(), LIMIT, LIMIT);
	observed.read(body).unwrap();
	observed.usage().output_tokens
}

fn observe_stream(pair: Pair, events: &[Vec<u8>]) -> Option<u64> {
	let mut observed = ObservedReply::new(pair.provider.upstream(), pair.model(), LIMIT, LIMIT);
	for event in events {
		observed.push(event).unwrap();
		black_box(observed.usage());
	}
	observed.finish().unwrap();
	observed.usage().output_tokens
}

fn stream_len(events: &[Vec<u8>]) -> usize {
	events.iter().map(Vec::len).sum()
}
