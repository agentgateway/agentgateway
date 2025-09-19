// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

// We build our own Date cacher, as formatting the date string is ~50% of the cost of the log
mod date;

// We have a force of tracing_appender to support batching writes, which leads to massive throughput benefits
mod msg;
mod nonblocking;
mod worker;

use std::cell::RefCell;
use std::fmt::{Debug, Display, Write as FmtWrite};
use std::str::FromStr;
use std::time::Instant;
use std::{env, fmt, io};

use itertools::Itertools;
use nonblocking::NonBlocking;
use once_cell::sync::{Lazy, OnceCell};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use thiserror::Error;
use tracing::{Event, Subscriber, error, field, info, warn};
use tracing_core::Field;
use tracing_core::field::Visit;
use tracing_core::span::Record;
use tracing_log::NormalizeEvent;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::{JsonVisitor, Writer};
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry, filter, reload};
pub use value_bag::ValueBag;

pub static APPLICATION_START_TIME: Lazy<Instant> = Lazy::new(Instant::now);
static LOG_HANDLE: OnceCell<LogHandle> = OnceCell::new();
static NON_BLOCKING: OnceCell<(NonBlocking, bool)> = OnceCell::new();

pub trait OptionExt<T>: Sized {
	fn display(&self) -> Option<ValueBag>
	where
		T: Display;
	fn debug(&self) -> Option<ValueBag>
	where
		T: Debug;
	fn quoted(&self) -> Option<ValueBag>
	where
		T: Serialize;
}

impl<T: 'static> OptionExt<T> for Option<T> {
	fn display(&self) -> Option<ValueBag>
	where
		T: Display,
	{
		self.as_ref().map(display)
	}
	fn debug(&self) -> Option<ValueBag>
	where
		T: Debug,
	{
		self.as_ref().map(debug)
	}
	fn quoted(&self) -> Option<ValueBag>
	where
		T: Serialize,
	{
		self.as_ref().map(quoted)
	}
}

pub fn display<T: Display + 'static>(value: &T) -> ValueBag {
	ValueBag::capture_display(value)
}

pub fn debug<T: Debug + 'static>(value: &T) -> ValueBag {
	ValueBag::capture_debug(value)
}

pub fn quoted<T: Serialize + 'static>(value: &T) -> ValueBag {
	ValueBag::from_serde1(value)
}

/// A safe function to determine if a target is enabled.
/// Do NOT use `tracing::enabled!` which is broken (https://github.com/tokio-rs/tracing/issues/3345)
pub fn enabled(target: &'static str, level: &tracing::Level) -> bool {
	if let Some(handle) = LOG_HANDLE.get() {
		handle
			.with_current(|f| f.filter().would_enable(target, level))
			.unwrap_or_default()
	} else {
		false
	}
}

// log is like using tracing macros, but allows arbitrary k/v pairs. Tracing requires compile-time keys!
// This does NOT respect tracing enable/log level; users can do that themselves before calling this function.
pub fn log(level: &str, target: &str, kv: &[(&str, Option<ValueBag>)]) {
	let Some((nb, json)) = NON_BLOCKING.get() else {
		return;
	};
	thread_local! {
		static BUF: RefCell<String> = const { RefCell::new(String::new()) };
	}

	// Re-use the buffer to reduce allocations
	let _ = BUF.with(|buf| {
		let borrow = buf.try_borrow_mut();
		let mut a;
		let mut b;
		let buf = match borrow {
			Ok(buf) => {
				a = buf;
				&mut *a
			},
			_ => {
				b = String::with_capacity(100);
				&mut b
			},
		};

		if *json {
			let mut sx = serde_json::Serializer::new(StringWriteAdaptor::new(buf));
			let mut s = sx.serialize_map(Some(kv.len() + 3))?;
			s.serialize_entry("level", level)?;
			s.serialize_entry("time", &date::build())?;
			s.serialize_entry("scope", target)?;
			for (k, v) in kv {
				match v {
					None => {},
					Some(i) => {
						s.serialize_entry(k, i)?;
					},
				}
			}
			s.end()?;
		} else {
			date::write(buf);
			write!(buf, "\t{level}\t{target}")?;
			for (k, v) in kv {
				match v {
					None => {},
					Some(i) => {
						write!(buf, " {k}={i}")?;
					},
				}
			}
		}
		buf.push('\n');
		// Send a copy to the logging thread
		nb.write_vec(buf.as_bytes().to_vec())?;
		buf.clear();
		Ok::<(), anyhow::Error>(())
	});
}

pub fn setup_logging() -> nonblocking::WorkerGuard {
	Lazy::force(&APPLICATION_START_TIME);
	let (non_blocking, _guard) = nonblocking::NonBlockingBuilder::default()
		.lossy(false)
		.buffered_lines_limit(10000) // Buffer up to 10l lines to avoid blocking on logs
		.finish(std::io::stdout());
	let use_json = env::var("LOG_FORMAT").unwrap_or("plain".to_string()) == "json";
	let _ = NON_BLOCKING.set((non_blocking.clone(), use_json));
	tracing_subscriber::registry()
		.with(fmt_layer(non_blocking, use_json))
		.init();
	_guard
}

fn json_fmt(writer: NonBlocking) -> Box<dyn Layer<Registry> + Send + Sync + 'static> {
	let format = tracing_subscriber::fmt::layer()
		.with_writer(writer)
		.event_format(IstioJsonFormat())
		.fmt_fields(IstioJsonFormat());
	Box::new(format)
}

fn plain_fmt(writer: NonBlocking) -> Box<dyn Layer<Registry> + Send + Sync + 'static> {
	let format = tracing_subscriber::fmt::layer()
		.with_writer(writer)
		.event_format(IstioFormat())
		.fmt_fields(IstioFormat());
	Box::new(format)
}

fn fmt_layer(
	writer: NonBlocking,
	use_json: bool,
) -> Box<dyn Layer<Registry> + Send + Sync + 'static> {
	let format = if use_json {
		json_fmt(writer)
	} else {
		plain_fmt(writer)
	};
	let filter = default_filter();
	let (layer, reload) = reload::Layer::new(format.with_filter(filter));
	LOG_HANDLE
		.set(reload)
		.map_or_else(|_| warn!("setup log handler failed"), |_| {});
	Box::new(layer)
}

fn default_filter() -> filter::Targets {
	const CUSTOM_TARGETS: &str = "rmcp=warn,hickory_server::server::server_future=off,typespec_client_core::http::policies::logging=warn";
	// Read from env var, but prefix with setting DNS logs to warn as they are noisy; they can be explicitly overriden
	let var: String = env::var("RUST_LOG")
		.map_err(|_| ())
		.map(|v| CUSTOM_TARGETS.to_string() + "," + v.as_str())
		.unwrap_or(CUSTOM_TARGETS.to_string() + ",info");
	filter::Targets::from_str(&var).expect("static filter should build")
}

// a handle to get and set the log level
type BoxLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;
type FilteredLayer = filter::Filtered<BoxLayer, filter::Targets, Registry>;
type LogHandle = reload::Handle<FilteredLayer, Registry>;

/// set_level dynamically updates the logging level to *include* level. If `reset` is true, it will
/// reset the entire logging configuration first.
pub fn set_level(reset: bool, level: &str) -> Result<(), Error> {
	if let Some(handle) = LOG_HANDLE.get() {
		// new_directive will be current_directive + level
		// it can be duplicate, but the Target's parse() will properly handle it
		let new_directive = if let Ok(current) = handle.with_current(|f| f.filter().to_string()) {
			if reset {
				if level.is_empty() {
					default_filter().to_string()
				} else {
					format!("{},{}", default_filter(), level)
				}
			} else {
				format!("{current},{level}")
			}
		} else {
			level.to_string()
		};

		// create the new Targets based on the new directives
		let new_filter = filter::Targets::from_str(&new_directive)?;
		info!("new log filter is {new_filter}");

		// set the new filter
		Ok(handle.modify(|layer| {
			*layer.filter_mut() = new_filter;
		})?)
	} else {
		warn!("failed to get log handle");
		Err(Error::Uninitialized)
	}
}

pub fn get_current_loglevel() -> Result<String, Error> {
	if let Some(handle) = LOG_HANDLE.get() {
		Ok(handle.with_current(|f| f.filter().to_string())?)
	} else {
		Err(Error::Uninitialized)
	}
}

#[derive(Error, Debug)]
pub enum Error {
	#[error("parse failure: {0}")]
	InvalidFilter(#[from] filter::ParseError),
	#[error("reload failure: {0}")]
	Reload(#[from] reload::Error),
	#[error("logging is not initialized")]
	Uninitialized,
}

// IstioFormat encodes logs in the "standard" Istio JSON formatting used in the rest of the code
struct IstioJsonFormat();

// IstioFormat encodes logs in the "standard" Istio formatting used in the rest of the code
struct IstioFormat();

struct Visitor<'writer> {
	res: std::fmt::Result,
	is_empty: bool,
	writer: Writer<'writer>,
}

impl Visitor<'_> {
	fn write_padded(&mut self, value: &impl Debug) -> std::fmt::Result {
		let padding = if self.is_empty {
			self.is_empty = false;
			""
		} else {
			" "
		};
		write!(self.writer, "{padding}{value:?}")
	}
}

impl field::Visit for Visitor<'_> {
	fn record_str(&mut self, field: &field::Field, value: &str) {
		if self.res.is_err() {
			return;
		}

		self.record_debug(field, &value)
	}

	fn record_debug(&mut self, field: &field::Field, val: &dyn std::fmt::Debug) {
		self.res = match field.name() {
			// Skip fields that are actually log metadata that have already been handled
			name if name.starts_with("log.") => Ok(()),
			// For the message, write out the message and a tab to separate the future fields
			"message" => write!(self.writer, "{val:?}\t"),
			// For the rest, k=v.
			_ => self.write_padded(&format_args!("{}={:?}", field.name(), val)),
		}
	}
}

impl<'writer> FormatFields<'writer> for IstioFormat {
	fn format_fields<R: tracing_subscriber::field::RecordFields>(
		&self,
		writer: Writer<'writer>,
		fields: R,
	) -> std::fmt::Result {
		let mut visitor = Visitor {
			writer,
			res: Ok(()),
			is_empty: true,
		};
		fields.record(&mut visitor);
		visitor.res
	}
}

impl<S, N> FormatEvent<S, N> for IstioFormat
where
	S: Subscriber + for<'a> LookupSpan<'a>,
	N: for<'a> FormatFields<'a> + 'static,
{
	fn format_event(
		&self,
		ctx: &FmtContext<'_, S, N>,
		mut writer: Writer<'_>,
		event: &Event<'_>,
	) -> std::fmt::Result {
		let normalized_meta = event.normalized_metadata();
		SystemTime.format_time(&mut writer)?;
		let meta = normalized_meta.as_ref().unwrap_or_else(|| event.metadata());
		write!(
			writer,
			"\t{}\t",
			meta.level().to_string().to_ascii_lowercase()
		)?;

		let target = meta.target();
		// No need to prefix everything
		let target = target.strip_prefix("agentgateway::").unwrap_or(target);
		write!(writer, "{target}")?;

		// Write out span fields. Istio logging outside of Rust doesn't really have this concept
		if let Some(scope) = ctx.event_scope() {
			for span in scope.from_root() {
				write!(writer, ":{}", span.metadata().name())?;
				let ext = span.extensions();
				if let Some(fields) = &ext.get::<FormattedFields<N>>()
					&& !fields.is_empty()
				{
					write!(writer, "{{{fields}}}")?;
				}
			}
		};
		// Insert tab only if there is fields
		if event.fields().any(|_| true) {
			write!(writer, "\t")?;
		}

		ctx.format_fields(writer.by_ref(), event)?;

		writeln!(writer)
	}
}

struct JsonVisitory<S: SerializeMap> {
	serializer: S,
	state: Result<(), S::Error>,
}

impl<S: SerializeMap> JsonVisitory<S> {
	pub(crate) fn done(self) -> Result<S, S::Error> {
		let JsonVisitory { serializer, state } = self;
		state?;
		Ok(serializer)
	}
}

impl<S: SerializeMap> Visit for JsonVisitory<S> {
	fn record_bool(&mut self, field: &Field, value: bool) {
		// If previous fields serialized successfully, continue serializing,
		// otherwise, short-circuit and do nothing.
		if self.state.is_ok() {
			self.state = self.serializer.serialize_entry(field.name(), &value)
		}
	}

	fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
		if self.state.is_ok() {
			self.state = self
				.serializer
				.serialize_entry(field.name(), &format_args!("{value:?}"))
		}
	}

	fn record_u64(&mut self, field: &Field, value: u64) {
		if self.state.is_ok() {
			self.state = self.serializer.serialize_entry(field.name(), &value)
		}
	}

	fn record_i64(&mut self, field: &Field, value: i64) {
		if self.state.is_ok() {
			self.state = self.serializer.serialize_entry(field.name(), &value)
		}
	}

	fn record_f64(&mut self, field: &Field, value: f64) {
		if self.state.is_ok() {
			self.state = self.serializer.serialize_entry(field.name(), &value)
		}
	}

	fn record_str(&mut self, field: &Field, value: &str) {
		if self.state.is_ok() {
			self.state = self.serializer.serialize_entry(field.name(), &value)
		}
	}
}
pub struct WriteAdaptor<'a> {
	fmt_write: &'a mut dyn fmt::Write,
}
impl<'a> WriteAdaptor<'a> {
	pub fn new(fmt_write: &'a mut dyn fmt::Write) -> Self {
		Self { fmt_write }
	}
}
impl io::Write for WriteAdaptor<'_> {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let s = std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

		self.fmt_write.write_str(s).map_err(io::Error::other)?;

		Ok(s.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}
pub struct StringWriteAdaptor<'a> {
	s: &'a mut String,
}
impl<'a> StringWriteAdaptor<'a> {
	pub fn new(s: &'a mut String) -> Self {
		Self { s }
	}
}
impl io::Write for StringWriteAdaptor<'_> {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let s = std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

		self.s.push_str(s);

		Ok(s.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}
impl<S, N> FormatEvent<S, N> for IstioJsonFormat
where
	S: Subscriber + for<'lookup> LookupSpan<'lookup>,
	N: for<'writer> FormatFields<'writer> + 'static,
{
	fn format_event(
		&self,
		ctx: &FmtContext<'_, S, N>,
		mut writer: Writer<'_>,
		event: &Event<'_>,
	) -> fmt::Result
	where
		S: Subscriber + for<'a> LookupSpan<'a>,
	{
		let meta = event.normalized_metadata();
		let meta = meta.as_ref().unwrap_or_else(|| event.metadata());
		let mut write = || {
			let mut timestamp = String::with_capacity(28);
			let mut w = Writer::new(&mut timestamp);
			SystemTime.format_time(&mut w)?;
			let mut sx = serde_json::Serializer::new(WriteAdaptor::new(&mut writer));
			let mut serializer = sx.serialize_map(event.fields().try_len().ok())?;
			serializer.serialize_entry("level", &meta.level().as_str().to_ascii_lowercase())?;
			serializer.serialize_entry("time", &timestamp)?;
			serializer.serialize_entry("scope", meta.target())?;
			let mut v = JsonVisitory {
				serializer,
				state: Ok(()),
			};
			event.record(&mut v);

			let mut serializer = v.done()?;
			if let Some(scope) = ctx.event_scope() {
				for span in scope.from_root() {
					let ext = span.extensions();
					if let Some(fields) = &ext.get::<FormattedFields<N>>() {
						let json = serde_json::from_str::<serde_json::Value>(fields)?;
						serializer.serialize_entry(span.metadata().name(), &json)?;
					}
				}
			};
			SerializeMap::end(serializer)?;
			Ok::<(), anyhow::Error>(())
		};
		write().map_err(|_| fmt::Error)?;
		writeln!(writer)
	}
}

// Copied from tracing_subscriber json
impl<'a> FormatFields<'a> for IstioJsonFormat {
	/// Format the provided `fields` to the provided `writer`, returning a result.
	fn format_fields<R: RecordFields>(&self, mut writer: Writer<'_>, fields: R) -> fmt::Result {
		use tracing_subscriber::field::VisitOutput;
		let mut v = JsonVisitor::new(&mut writer);
		fields.record(&mut v);
		v.finish()
	}

	fn add_fields(
		&self,
		_current: &'a mut FormattedFields<Self>,
		_fields: &Record<'_>,
	) -> fmt::Result {
		// We could implement this but tracing doesn't give us an easy or efficient way to do so.
		// for not just disallow it.
		debug_assert!(false, "add_fields is inefficient and should not be used");
		Ok(())
	}
}

/// Mod testing gives access to a test logger, which stores logs in memory for querying.
/// Inspired by https://github.com/dbrgn/tracing-test
pub mod testing {
	use std::collections::HashMap;
	use std::fmt::{Display, Formatter};
	use std::io;
	use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

	use once_cell::sync::Lazy;
	use serde_json::Value;
	use tracing_subscriber::fmt;
	use tracing_subscriber::fmt::writer::Tee;
	use tracing_subscriber::layer::SubscriberExt;
	use tracing_subscriber::util::SubscriberInitExt;

	use crate::telemetry::{
		APPLICATION_START_TIME, IstioJsonFormat, NON_BLOCKING, fmt_layer, nonblocking,
	};

	#[derive(Debug)]
	pub enum LogError {
		// Wanted to equal the value, its missing
		Missing(String),
		// Want to be absent but it is present
		Present(String),
		// Mismatch: want, got
		Mismatch(String, String),
	}

	impl Display for LogError {
		fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
			match self {
				LogError::Missing(_v) => {
					write!(f, "missing")
				},
				LogError::Present(v) => {
					write!(f, "{v:?} found unexpectedly")
				},
				LogError::Mismatch(want, got) => {
					write!(f, "{want:?} != {got:?}")
				},
			}
		}
	}

	/// assert_contains asserts the logs contain a line with the matching keys.
	/// Common keys to match one are "target" and "message"; most of the rest are custom.
	pub fn find(want: &[(&str, &str)]) -> Vec<Value> {
		let want: HashMap<&str, &str> = HashMap::from_iter(want.iter().cloned());
		let logs = {
			let b = global_buf();
			let buf = b.lock().unwrap();
			std::str::from_utf8(&buf)
				.expect("Logs contain invalid UTF8")
				.to_string()
		};
		let found: Vec<Value> = logs
			.lines()
			.map(|line| serde_json::from_str::<serde_json::Value>(line).expect("log must be valid json"))
			.flat_map(|log| {
				for (k, v) in &want {
					let Some(have) = log.get(k) else {
						if !v.is_empty() {
							// Wanted value, found none
							return None;
						}
						continue;
					};
					let have = match have {
						Value::Number(n) => format!("{n}"),
						Value::String(v) => v.clone(),
						_ => panic!("find currently only supports string/number values"),
					};
					if v.is_empty() {
						// Wanted NO value, found something
						return None;
					}
					// TODO fuzzy match
					if *v != have {
						// Wanted value, but it mismatched
						return None;
					}
				}
				Some(log)
			})
			.collect();

		found
	}

	/// MockWriter will store written logs
	#[derive(Debug, Clone)]
	pub struct MockWriter {
		buf: Arc<Mutex<Vec<u8>>>,
	}

	impl MockWriter {
		pub fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
			Self { buf }
		}

		fn buf(&self) -> io::Result<MutexGuard<Vec<u8>>> {
			self
				.buf
				.lock()
				.map_err(|_| io::Error::from(io::ErrorKind::Other))
		}
	}

	impl io::Write for MockWriter {
		fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
			let mut target = self.buf()?;
			target.write(buf)
		}

		fn flush(&mut self) -> io::Result<()> {
			self.buf()?.flush()
		}
	}

	impl fmt::MakeWriter<'_> for MockWriter {
		type Writer = Self;

		fn make_writer(&self) -> Self::Writer {
			MockWriter::new(self.buf.clone())
		}
	}

	// Global buffer to store logs in
	fn global_buf() -> Arc<Mutex<Vec<u8>>> {
		static GLOBAL_BUF: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
		GLOBAL_BUF
			.get_or_init(|| Arc::new(Mutex::new(vec![])))
			.clone()
	}
	static TRACING: Lazy<()> = Lazy::new(setup_test_logging_internal);

	pub fn setup_test_logging() {
		Lazy::force(&TRACING);
	}

	pub fn setup_test_logging_internal() {
		Lazy::force(&APPLICATION_START_TIME);
		let mock_writer = MockWriter::new(global_buf());
		let (non_blocking, _guard) = nonblocking::NonBlockingBuilder::default()
			.lossy(false)
			.buffered_lines_limit(1)
			.finish(Tee::new(std::io::stdout(), mock_writer.clone()));
		let _ = NON_BLOCKING.set((non_blocking.clone(), true));
		// Ensure we do not close until the program ends
		Box::leak(Box::new(_guard));
		let layer: fmt::Layer<_, _, _, _> = fmt::layer()
			.event_format(IstioJsonFormat())
			.fmt_fields(IstioJsonFormat())
			.with_writer(mock_writer);
		tracing_subscriber::registry()
			.with(fmt_layer(non_blocking, true))
			.with(layer)
			.init();
	}
}
