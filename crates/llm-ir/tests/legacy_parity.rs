//! Runs legacy's goldens through the LLM IR and compares the two, grouped by route. Every
//! input has a `Disposition` that says how the IR treats it compared with legacy, and a new input
//! fails until it gets one. The IR's result is snapshotted beside the legacy snapshots, which are
//! unchanged. This module holds what the files share; `parity` instead checks the IR against
//! legacy's typed requests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_core::strng;
use agent_llm::{
	CacheTokenConvention, InputFormat, LLMInfo, LLMRequest, LLMResponse, StreamingUsageGuard,
	StreamingUsageReporter,
};
use agent_llm_ir::{
	BufferedReply, ClientProtocol, HttpReply, IdentifierPseudonymizer, RenderSettings, ReplyContext,
	StreamReply, Upstream, Usage, prepare,
};
use serde_json::{Value, json};

#[path = "legacy_parity/chat_client.rs"]
mod chat_client;
#[path = "legacy_parity/chat_provider.rs"]
mod chat_provider;
#[path = "legacy_parity/messages_responses.rs"]
mod messages_responses;
#[path = "legacy_parity/parity.rs"]
mod parity;
#[path = "legacy_parity/refusals.rs"]
mod refusals;

use refusals::assert_request_refusal;

/// How the IR's request compares with legacy's.
#[derive(Debug, Clone, Copy)]
enum Disposition {
	/// The same request, apart from representation; see `without_representation`.
	Equivalent,
	/// Legacy is wrong here and the IR is right.
	LegacyBug(&'static str),
	/// A deliberate difference that is neither a fix nor a new capability.
	Changed(&'static str),
	/// The IR refuses what legacy sends: a compatibility restriction.
	Unsupported(&'static str),
	/// Legacy's goldens don't send this input on this route.
	OutsideRoute,
}

use Disposition::*;

fn describe(disposition: Disposition) -> String {
	match disposition {
		Equivalent => "equivalent".to_string(),
		LegacyBug(why) => format!("legacy bug: {why}"),
		Changed(why) => format!("changed: {why}"),
		Unsupported(what) => format!("unsupported: {what}"),
		OutsideRoute => unreachable!("not converted"),
	}
}

fn assert_every_input_listed<'a>(directory: &str, listed: impl Iterator<Item = &'a str>) {
	let inputs: BTreeSet<String> = fs::read_dir(legacy_fixture(directory))
		.unwrap()
		.map(|entry| entry.unwrap().path())
		.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
		.map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
		.collect();
	let listed: BTreeSet<String> = listed.map(str::to_string).collect();
	assert_eq!(
		inputs.difference(&listed).collect::<Vec<_>>(),
		Vec::<&String>::new(),
		"{directory}: inputs without a disposition"
	);
	assert_eq!(
		listed.difference(&inputs).collect::<Vec<_>>(),
		Vec::<&String>::new(),
		"{directory}: dispositions without an input"
	);
}

struct IrRequest {
	request: Value,
	dropped: Vec<&'static str>,
	emulated_stop_sequences: Vec<String>,
}

fn ir_request(
	body: &str,
	client: ClientProtocol,
	upstream: Upstream<'_>,
) -> Result<IrRequest, String> {
	let converted = prepare(body.as_bytes(), client, upstream)
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.map_err(|refusal| refusal.to_string())?;
	Ok(IrRequest {
		request: serde_json::from_str(&converted.body).unwrap(),
		dropped: converted.dropped,
		emulated_stop_sequences: converted.reply.emulated_stop_sequences,
	})
}

/// Leaked so an upstream can outlive the body it was read from; these are short test runs.
fn model_of(body: &str) -> &'static str {
	let model = serde_json::from_str::<Value>(body).unwrap()["model"]
		.as_str()
		.unwrap()
		.to_string();
	model.leak()
}

/// `case` is the snapshot's file name without `.snap`, beside the input in `directory`.
fn snapshot(
	directory: &str,
	case: &str,
	disposition: &str,
	body: &str,
	ir: &Result<IrRequest, String>,
) {
	let mut report = match ir {
		Ok(ir) => json!({"disposition": disposition, "request": ir.request, "dropped": ir.dropped}),
		Err(refusal) => json!({"disposition": disposition, "refused": refusal}),
	};
	if let Ok(ir) = ir
		&& !ir.emulated_stop_sequences.is_empty()
	{
		report["emulated_stop_sequences"] = json!(ir.emulated_stop_sequences);
	}
	let input: Value = serde_json::from_str(body).unwrap();
	snapshot_beside_input(directory, case, &input, &report);
}

/// `case` is the snapshot's file name without `.snap`, beside the reply in `directory`.
/// `client` is what the IR wrote for the client.
fn reply_snapshot(
	directory: &str,
	case: &str,
	disposition: Disposition,
	body: &[u8],
	client: &Value,
	usage: &Value,
) {
	let report = json!({
		"disposition": describe(disposition),
		"client": client,
		"usage": usage,
	});
	snapshot_beside_input(directory, case, &reply_input(body), &report);
}

// Refused and malformed fixtures are expected here. The public drivers record their
// failures in the client reply while retaining any provider usage they could read.
fn drive_buffered_reply(body: &[u8], context: ReplyContext<'_>) -> (HttpReply, Usage) {
	let mut driver = BufferedReply::new(context);
	let _ = driver.read(body);
	let usage = driver.usage();
	(driver.into_reply(), usage)
}

fn drive_streamed_reply(
	body: &[u8],
	context: ReplyContext<'_>,
) -> (Result<Vec<u8>, HttpReply>, Usage) {
	let mut driver = StreamReply::new(context, usize::MAX);
	let _ = driver.push(body);
	let mut out = driver.take_output().unwrap_or_default();
	let _ = driver.finish();
	out.extend(driver.take_output().unwrap_or_default());
	let reply = match driver.early_failure() {
		Some(reply) => {
			assert!(
				out.is_empty(),
				"an HTTP error must precede all stream output"
			);
			Err(reply)
		},
		None => Ok(out),
	};
	(reply, driver.usage())
}

fn assert_reply_refusal(body: &[u8], context: ReplyContext<'_>, streamed: bool, reason: &str) {
	let error = if streamed {
		StreamReply::new(context, usize::MAX)
			.push(body)
			.unwrap_err()
	} else {
		BufferedReply::new(context).read(body).unwrap_err()
	};
	assert_eq!(error.to_string(), reason, "reply refusal reason");
}

/// A reply body as JSON, or a stream as its text.
fn reply_input(body: &[u8]) -> Value {
	serde_json::from_slice::<Value>(body)
		.unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).replace("\r\n", "\n")))
}

/// The path is made absolute because insta reads a relative one from this file's directory,
/// not from the crate.
fn snapshot_beside_input(directory: &str, case: &str, input: &Value, report: &Value) {
	let name = case.split('.').next().unwrap();
	insta::with_settings!({
		info => input,
		description => format!("{directory}/{name}.json"),
		omit_expression => true,
		prepend_module_to_snapshot => false,
		snapshot_path => legacy_fixture(directory),
	}, {
		insta::assert_json_snapshot!(case, report);
	});
}

fn legacy_fixture(directory: &str) -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../llm")
		.join(directory)
}

/// Legacy's stream records usage through a guard, as in the gateway; read it from the info
/// once the stream has been collected.
fn legacy_stream_usage(input_format: InputFormat) -> (Arc<Mutex<LLMInfo>>, StreamingUsageGuard) {
	let info = Arc::new(Mutex::new(LLMInfo::new(
		LLMRequest {
			input_tokens: None,
			input_format,
			cache_convention: CacheTokenConvention::pending(),
			request_model: strng::literal!("golden"),
			original_model: None,
			provider: Default::default(),
			streaming: true,
			params: Default::default(),
			prompt: None,
			provider_state: None,
		},
		LLMResponse::default(),
	)));
	let guard = StreamingUsageGuard::new(Box::new(Reporter { info: info.clone() }));
	(info, guard)
}

/// The usage both sides record, in the IR's terms.
fn legacy_usage(response: &LLMResponse) -> Value {
	json!({
		"input_tokens": response.input_tokens,
		"output_tokens": response.output_tokens,
		"total_tokens": response.total_tokens,
		"cached_tokens": response.cached_input_tokens,
		"cache_write_tokens": response.cache_creation_input_tokens,
		"reasoning_tokens": response.reasoning_tokens,
	})
}

fn ir_usage(usage: Usage) -> Value {
	json!({
		"input_tokens": usage.input_tokens,
		"output_tokens": usage.output_tokens,
		"total_tokens": usage.total_tokens,
		"cached_tokens": usage.cached_tokens,
		"cache_write_tokens": usage.cache_write_tokens,
		"reasoning_tokens": usage.reasoning_tokens,
	})
}

/// The `data` of each event of a well-formed SSE stream, read without the IR's parser.
fn sse_data(stream: &[u8]) -> Vec<Value> {
	std::str::from_utf8(stream)
		.unwrap()
		.replace("\r\n", "\n")
		.split("\n\n")
		.filter_map(|event| {
			let data: Vec<&str> = event
				.lines()
				.filter_map(|line| line.strip_prefix("data: "))
				.collect();
			(!data.is_empty()).then(|| serde_json::from_str(&data.join("\n")).unwrap())
		})
		.collect()
}

#[derive(Clone)]
struct Reporter {
	info: Arc<Mutex<LLMInfo>>,
}

impl StreamingUsageReporter for Reporter {
	fn update(&self, f: &mut dyn FnMut(&mut LLMInfo)) {
		f(&mut self.info.lock().unwrap());
	}

	fn report_usage(&mut self) {}
}

/// These inputs need no identifier shortening.
struct FixedIdentifier;

impl IdentifierPseudonymizer for FixedIdentifier {
	fn pseudonymize(&self, _identifier: &str) -> String {
		"01".into()
	}
}
