use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use std::{env, thread};

use chrono::{DateTime, Duration, Utc};
use crossbeam::channel::{Receiver, Sender, TryRecvError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::{apply, schema};

mod postgres;
mod sqlite;

static REQUEST_LOG_STORE: OnceLock<RequestLogStore> = OnceLock::new();
static REQUEST_LOG_STORE_BACKLOG: AtomicUsize = AtomicUsize::new(0);

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub struct Config {
	/// Connection URL for the request log database. A postgres:// or postgresql:// URL uses Postgres; any other value is treated as a SQLite database.
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	pub url: String,
	/// Maximum number of connections to open in this database's connection pool. Defaults to 5.
	/// When the request log and config stores have matching database settings, they share one pool
	/// with this limit.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(range(min = 1)))]
	pub max_connections: Option<u32>,
}

#[derive(Clone)]
pub struct RequestLogStore {
	tx: Sender<LogStoreMsg>,
}

impl RequestLogStore {
	pub(super) fn emit(&self, record: PendingRequestLog) {
		REQUEST_LOG_STORE_BACKLOG.fetch_add(1, Ordering::Relaxed);
		if let Err(err) = self.tx.send(LogStoreMsg::Record(record)) {
			REQUEST_LOG_STORE_BACKLOG.fetch_sub(1, Ordering::Relaxed);
			warn!(target: "request", ?err, "failed to enqueue request log database record");
		}
	}

	async fn request<T: Send + 'static>(
		&self,
		msg: impl FnOnce(QueryResponse<T>) -> LogStoreMsg,
	) -> anyhow::Result<T> {
		let (tx, rx) = oneshot::channel();
		self
			.tx
			.send(msg(tx))
			.map_err(|_| anyhow::anyhow!("request log database worker is stopped"))?;
		rx.await
			.map_err(|_| anyhow::anyhow!("request log database worker is stopped"))?
	}
}

pub async fn setup(cfg: &Config) -> anyhow::Result<RequestLogStoreGuard> {
	setup_with_pool(cfg, None).await
}

pub async fn setup_with_pool(
	cfg: &Config,
	pool: Option<crate::database::DatabasePool>,
) -> anyhow::Result<RequestLogStoreGuard> {
	let (tx, rx) = crossbeam::channel::unbounded();
	let (ready_tx, ready_rx) = oneshot::channel();
	let writer = LogStoreWorker::new(rx, cfg.clone(), pool, ready_tx)
		.worker_thread("request-log-db-writer".to_string())?;
	ready_rx
		.await
		.map_err(|_| anyhow::anyhow!("request log database worker stopped during startup"))??;
	let store = RequestLogStore { tx: tx.clone() };
	let _ = REQUEST_LOG_STORE.set(store);
	Ok(RequestLogStoreGuard {
		tx,
		writer: Some(writer),
	})
}

pub(super) fn emit(record: PendingRequestLog) {
	if let Some(store) = REQUEST_LOG_STORE.get() {
		store.emit(record);
	}
}

pub fn enabled() -> bool {
	REQUEST_LOG_STORE.get().is_some()
}

pub async fn search(request: SearchRequest) -> anyhow::Result<SearchResponse> {
	let store = REQUEST_LOG_STORE
		.get()
		.ok_or_else(|| anyhow::anyhow!("request log database is not configured"))?;
	store
		.request(|tx| LogStoreMsg::Search { request, tx })
		.await
}

pub async fn get(request: GetRequest) -> anyhow::Result<GetResponse> {
	let store = REQUEST_LOG_STORE
		.get()
		.ok_or_else(|| anyhow::anyhow!("request log database is not configured"))?;
	store.request(|tx| LogStoreMsg::Get { request, tx }).await
}

pub async fn analytics_summary(
	request: AnalyticsSummaryRequest,
) -> anyhow::Result<AnalyticsSummaryResponse> {
	let store = REQUEST_LOG_STORE
		.get()
		.ok_or_else(|| anyhow::anyhow!("request log database is not configured"))?;
	store
		.request(|tx| LogStoreMsg::AnalyticsSummary { request, tx })
		.await
}

pub async fn tail(request: TailRequest) -> anyhow::Result<TailResponse> {
	let store = REQUEST_LOG_STORE
		.get()
		.ok_or_else(|| anyhow::anyhow!("request log database is not configured"))?;
	store.request(|tx| LogStoreMsg::Tail { request, tx }).await
}

pub struct RequestLogStoreGuard {
	tx: Sender<LogStoreMsg>,
	writer: Option<thread::JoinHandle<()>>,
}

impl RequestLogStoreGuard {
	pub async fn shutdown_and_wait(mut self) {
		let _ = self.tx.send(LogStoreMsg::Shutdown);
		if let Some(writer) = self.writer.take() {
			match tokio::task::spawn_blocking(move || writer.join()).await {
				Ok(Ok(())) => {},
				Ok(Err(_)) => {
					warn!(target: "request", "request log database writer panicked");
				},
				Err(err) => {
					warn!(target: "request", ?err, "failed to join request log database writer");
				},
			};
		}
	}
}

impl Drop for RequestLogStoreGuard {
	fn drop(&mut self) {
		let _ = self.tx.send(LogStoreMsg::Shutdown);
	}
}

// Testing shows this to achieve a pretty good throughput for both postgres and sqlite
const DEFAULT_LOG_STORE_BATCH_SIZE: usize = 16384;
const LOG_STORE_BATCH_SIZE_ENV: &str = "REQUEST_LOG_STORE_BATCH_SIZE";

#[allow(clippy::large_enum_variant)] // The StoredRequestLog, which is used 99.9% of the time, is the large one
enum LogStoreMsg {
	Record(PendingRequestLog),
	Search {
		request: SearchRequest,
		tx: QueryResponse<SearchResponse>,
	},
	Get {
		request: GetRequest,
		tx: QueryResponse<GetResponse>,
	},
	AnalyticsSummary {
		request: AnalyticsSummaryRequest,
		tx: QueryResponse<AnalyticsSummaryResponse>,
	},
	Tail {
		request: TailRequest,
		tx: QueryResponse<TailResponse>,
	},
	Shutdown,
}

type QueryResponse<T> = oneshot::Sender<anyhow::Result<T>>;

struct LogStoreWorker {
	receiver: Receiver<LogStoreMsg>,
	cfg: Config,
	pool: Option<crate::database::DatabasePool>,
	ready_tx: Option<oneshot::Sender<anyhow::Result<()>>>,
}

impl LogStoreWorker {
	fn new(
		receiver: Receiver<LogStoreMsg>,
		cfg: Config,
		pool: Option<crate::database::DatabasePool>,
		ready_tx: oneshot::Sender<anyhow::Result<()>>,
	) -> Self {
		Self {
			receiver,
			cfg,
			pool,
			ready_tx: Some(ready_tx),
		}
	}

	fn worker_thread(self, name: String) -> anyhow::Result<thread::JoinHandle<()>> {
		thread::Builder::new()
			.name(name)
			.spawn(move || self.work())
			.map_err(Into::into)
	}

	fn work(mut self) {
		let runtime = match tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
		{
			Ok(runtime) => runtime,
			Err(err) => {
				warn!(target: "request", ?err, "failed to start request log database worker runtime");
				self.notify_ready(Err(anyhow::Error::new(err)));
				return;
			},
		};
		runtime.block_on(self.work_async());
		debug!(target: "request", "request log database writer stopped");
	}

	fn notify_ready(&mut self, result: anyhow::Result<()>) {
		if let Some(tx) = self.ready_tx.take() {
			let _ = tx.send(result);
		}
	}

	async fn work_async(mut self) {
		let batch_size = match log_store_batch_size() {
			Ok(batch_size) => batch_size,
			Err(err) => {
				self.notify_ready(Err(err));
				return;
			},
		};
		let backend = match Backend::connect(&self.cfg, self.pool.take()).await {
			Ok(backend) => backend,
			Err(err) => {
				self.notify_ready(Err(err));
				return;
			},
		};
		self.notify_ready(Ok(()));
		let mut batch = Vec::with_capacity(batch_size);
		loop {
			match self.receiver.recv() {
				Ok(msg) => {
					if process_log_store_msg(&backend, &mut batch, msg).await {
						self.drain_available_messages(&backend, &mut batch).await;
						break;
					}
				},
				Err(_) => {
					self.drain_available_messages(&backend, &mut batch).await;
					break;
				},
			}

			let mut shutdown = false;
			while batch.len() < batch.capacity() {
				match self.receiver.try_recv() {
					Ok(msg) => {
						if process_log_store_msg(&backend, &mut batch, msg).await {
							shutdown = true;
							break;
						}
					},
					Err(TryRecvError::Disconnected) => {
						shutdown = true;
						break;
					},
					Err(TryRecvError::Empty) => break,
				}
			}
			flush_log_store_batch(&backend, &mut batch).await;
			if shutdown {
				self.drain_available_messages(&backend, &mut batch).await;
				break;
			}
		}
	}

	async fn drain_available_messages(&self, backend: &Backend, batch: &mut Vec<StoredRequestLog>) {
		loop {
			match self.receiver.try_recv() {
				Ok(msg) => {
					let _ = process_log_store_msg(backend, batch, msg).await;
				},
				Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
			}
			if batch.len() == batch.capacity() {
				flush_log_store_batch(backend, batch).await;
			}
		}
		flush_log_store_batch(backend, batch).await;
	}
}

fn log_store_batch_size() -> anyhow::Result<usize> {
	let Ok(value) = env::var(LOG_STORE_BATCH_SIZE_ENV) else {
		return Ok(DEFAULT_LOG_STORE_BATCH_SIZE);
	};
	let batch_size = value.parse::<usize>().map_err(|err| {
		anyhow::anyhow!(
			"invalid env var {LOG_STORE_BATCH_SIZE_ENV}={value} ({})",
			err
		)
	})?;
	if batch_size == 0 {
		anyhow::bail!("invalid env var {LOG_STORE_BATCH_SIZE_ENV}=0 (must be greater than 0)");
	}
	Ok(batch_size)
}

async fn process_log_store_msg(
	backend: &Backend,
	batch: &mut Vec<StoredRequestLog>,
	msg: LogStoreMsg,
) -> bool {
	match msg {
		LogStoreMsg::Record(pending) => {
			let mut record = pending.record;
			record.payload = super::log::database_llm_payload(
				pending.llm_mode,
				pending.input_messages.as_deref().map(Vec::as_slice),
				pending.llm_response.as_ref(),
			);
			record.has_payload = record.payload.is_some();
			batch.push(record);
			false
		},
		LogStoreMsg::Search { request, tx } => {
			flush_log_store_batch(backend, batch).await;
			let _ = tx.send(backend.search(request).await);
			false
		},
		LogStoreMsg::Get { request, tx } => {
			flush_log_store_batch(backend, batch).await;
			let _ = tx.send(backend.get(request).await);
			false
		},
		LogStoreMsg::AnalyticsSummary { request, tx } => {
			flush_log_store_batch(backend, batch).await;
			let _ = tx.send(backend.analytics_summary(request).await);
			false
		},
		LogStoreMsg::Tail { request, tx } => {
			flush_log_store_batch(backend, batch).await;
			let _ = tx.send(backend.tail(request).await);
			false
		},
		LogStoreMsg::Shutdown => true,
	}
}

async fn flush_log_store_batch(backend: &Backend, batch: &mut Vec<StoredRequestLog>) {
	if batch.is_empty() {
		return;
	}
	let t0 = Instant::now();
	let count = batch.len();
	if let Err(err) = backend.insert_batch(batch).await {
		warn!(target: "request", ?err, count, "failed to persist request log batch");
	}
	let backlog = REQUEST_LOG_STORE_BACKLOG.fetch_sub(count, Ordering::Relaxed) - count;
	let latency = t0.elapsed();
	let throughput = count as f64 / latency.as_secs_f64();
	debug!(
		count,
		backlog,
		?latency,
		throughput,
		"flushed request log database batch"
	);
	batch.clear();
}

// Keep captured content in its original form until it reaches the database worker.
// Normalization and JSON serialization can be expensive for large prompts and completions.
pub(super) struct PendingRequestLog {
	pub record: StoredRequestLog,
	pub llm_mode: Option<crate::types::frontend::DatabaseLlmMode>,
	pub input_messages: Option<Arc<Vec<agent_llm::types::NormalizedMessage>>>,
	pub llm_response: Option<crate::cel::LLMContext>,
}

#[derive(Clone, Debug)]
pub struct StoredRequestLog {
	pub id: String,
	pub started_at: DateTime<Utc>,
	pub completed_at: DateTime<Utc>,
	pub duration_ms: i64,
	pub trace_id: Option<String>,
	pub span_id: Option<String>,
	pub http_status: Option<i64>,
	pub error: Option<String>,
	pub gen_ai_operation_name: Option<String>,
	pub gen_ai_provider_name: Option<String>,
	pub gen_ai_request_model: Option<String>,
	pub gen_ai_response_model: Option<String>,
	pub input_tokens: Option<i64>,
	pub output_tokens: Option<i64>,
	pub total_tokens: Option<i64>,
	pub cache_read_input_tokens: Option<i64>,
	pub cache_creation_input_tokens: Option<i64>,
	pub reasoning_tokens: Option<i64>,
	pub time_to_first_token_ms: Option<f64>,
	pub time_per_output_token_ms: Option<f64>,
	pub cost: Option<f64>,
	pub agentgateway_user: Option<String>,
	pub agentgateway_group: Option<String>,
	pub user_agent_name: Option<String>,
	pub has_payload: bool,
	pub attributes_json: Box<str>,
	pub payload: Option<StoredRequestLogPayload>,
}

#[derive(Clone, Debug)]
pub struct StoredRequestLogPayload {
	pub request_prompt_json: Option<Value>,
	pub response_completion_json: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimeRange {
	pub from: Option<DateTime<Utc>>,
	pub to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogFilters {
	#[serde(default)]
	pub http_status: Vec<i64>,
	#[serde(default)]
	pub provider: Vec<String>,
	#[serde(default)]
	pub request_model: Vec<String>,
	#[serde(default)]
	pub response_model: Vec<String>,
	#[serde(default)]
	pub trace_id: Option<String>,
	#[serde(default)]
	pub has_payload: Option<bool>,
	#[serde(default)]
	pub attributes: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
	#[serde(default)]
	pub limit: Option<i64>,
	#[serde(default)]
	pub cursor: Option<String>,
	#[serde(default)]
	pub time_range: Option<TimeRange>,
	#[serde(default)]
	pub filters: LogFilters,
	#[serde(default)]
	pub include_attributes: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRequest {
	pub id: String,
	#[serde(default)]
	pub include_payload: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyticsSummaryRequest {
	#[serde(default)]
	pub time_range: Option<TimeRange>,
	#[serde(default)]
	pub filters: LogFilters,
	#[serde(default)]
	pub group_by: Vec<GroupBy>,
	#[serde(default)]
	pub bucket_count: Option<i64>,
	#[serde(default)]
	pub bucket_seconds: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TailRequest {
	#[serde(default)]
	pub limit: Option<i64>,
	#[serde(default)]
	pub cursor: Option<String>,
	#[serde(default)]
	pub filters: LogFilters,
	#[serde(default)]
	pub include_attributes: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupBy {
	pub field: GroupByField,
	#[serde(default)]
	pub key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GroupByField {
	Provider,
	RequestModel,
	ResponseModel,
	HttpStatus,
	Attributes,
	/// Time to first token histogram; the group value is the bucket's lower bound in milliseconds.
	/// Requests without a first token (non-streaming) group under null.
	Ttft,
	/// Output tokens per second histogram; the group value is the bucket's lower bound.
	OutputTokensPerSecond,
}

pub(crate) const MIGRATIONS_TABLE: &str = "_agentgateway_request_log_migrations";

pub(crate) const TTFT_BOUNDS_MS: &[i64] = &[
	0, 100, 250, 500, 1000, 2000, 5000, 10_000, 30_000, 60_000, 120_000, 300_000,
];
pub(crate) const OUTPUT_TOKENS_PER_SECOND_BOUNDS: &[i64] = &[
	0, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000, 25_000,
];

/// A `CASE` labelling each row with the lower bound of the bucket `value` falls in, or NULL when
/// `value` is NULL.
pub(crate) fn histogram_bucket_sql(value: &str, bounds: &[i64]) -> String {
	let mut sql = format!("CASE WHEN {value} IS NULL THEN NULL");
	for pair in bounds.windows(2) {
		sql.push_str(&format!(" WHEN {value} < {} THEN '{}'", pair[1], pair[0]));
	}
	let last = bounds.last().copied().unwrap_or_default();
	sql.push_str(&format!(" ELSE '{last}' END"));
	sql
}

/// The `AnalyticsTotals` select list, valid in both SQLite and Postgres. Aliases must match the
/// struct's field names.
pub(crate) const ANALYTICS_TOTALS_SELECT: &str = "\
	COUNT(*) AS requests, \
	CAST(COALESCE(SUM(total_tokens), 0) AS BIGINT) AS total_tokens, \
	CAST(COALESCE(SUM(cost), 0) AS DOUBLE PRECISION) AS cost, \
	CAST(COALESCE(SUM(input_tokens), 0) AS BIGINT) AS input_tokens, \
	CAST(COALESCE(SUM(output_tokens), 0) AS BIGINT) AS output_tokens, \
	CAST(COALESCE(SUM(CASE WHEN http_status >= 400 OR error IS NOT NULL THEN 1 ELSE 0 END), 0) AS BIGINT) AS errors, \
	CAST(COALESCE(SUM(duration_ms), 0) AS BIGINT) AS duration_ms, \
	COUNT(time_to_first_token_ms) AS ttft_requests, \
	CAST(COALESCE(SUM(time_to_first_token_ms), 0) AS DOUBLE PRECISION) AS ttft_ms, \
	CAST(COALESCE(MAX(time_to_first_token_ms), 0) AS DOUBLE PRECISION) AS max_ttft_ms, \
	CAST(COALESCE(SUM(CASE WHEN time_per_output_token_ms > 0 THEN output_tokens END), 0) AS BIGINT) AS generation_tokens, \
	CAST(COALESCE(SUM(CASE WHEN time_per_output_token_ms > 0 THEN time_per_output_token_ms * output_tokens END), 0) AS DOUBLE PRECISION) AS generation_ms, \
	CAST(COALESCE(SUM(cache_read_input_tokens), 0) AS BIGINT) AS cache_read_tokens, \
	CAST(COALESCE(SUM(cache_creation_input_tokens), 0) AS BIGINT) AS cache_write_tokens, \
	CAST(COALESCE(SUM(reasoning_tokens), 0) AS BIGINT) AS reasoning_tokens";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
	pub logs: Vec<LogEntry>,
	pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetResponse {
	pub log: Option<LogEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSummaryResponse {
	pub time_range: TimeRange,
	pub bucket_seconds: i64,
	pub buckets: Vec<AnalyticsTimeBucket>,
	pub groups: Vec<AnalyticsGroup>,
	pub filter_options: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailResponse {
	pub logs: Vec<LogEntry>,
	pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailEvent {
	pub entry: LogEntry,
	pub cursor: String,
}

#[derive(Clone, Debug, Default, Serialize, sqlx_macros::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTotals {
	pub requests: i64,
	pub total_tokens: i64,
	pub cost: f64,
	pub input_tokens: i64,
	pub output_tokens: i64,
	/// Requests with an HTTP status >= 400 or a recorded error.
	pub errors: i64,
	/// Sum of request durations; divide by `requests` for the mean.
	pub duration_ms: i64,
	/// Requests with a recorded time to first token (streaming responses).
	pub ttft_requests: i64,
	/// Sum of time to first token; divide by `ttft_requests` for the mean.
	pub ttft_ms: f64,
	pub max_ttft_ms: f64,
	/// Output tokens of requests with a recorded per-token time.
	pub generation_tokens: i64,
	/// Time spent generating those tokens; `generation_tokens` over this is the decode rate.
	pub generation_ms: f64,
	/// Input tokens served from the provider's prompt cache (included in `input_tokens`).
	pub cache_read_tokens: i64,
	/// Input tokens written to the provider's prompt cache (included in `input_tokens`).
	pub cache_write_tokens: i64,
	/// Reasoning tokens (included in `output_tokens`).
	pub reasoning_tokens: i64,
}

impl AnalyticsTotals {
	fn add(&mut self, other: &AnalyticsTotals) {
		// Destructure so a new field fails to compile until it is merged here.
		let AnalyticsTotals {
			requests,
			total_tokens,
			cost,
			input_tokens,
			output_tokens,
			errors,
			duration_ms,
			ttft_requests,
			ttft_ms,
			max_ttft_ms,
			generation_tokens,
			generation_ms,
			cache_read_tokens,
			cache_write_tokens,
			reasoning_tokens,
		} = other;
		self.requests += requests;
		self.total_tokens += total_tokens;
		self.cost += cost;
		self.input_tokens += input_tokens;
		self.output_tokens += output_tokens;
		self.errors += errors;
		self.duration_ms += duration_ms;
		self.ttft_requests += ttft_requests;
		self.ttft_ms += ttft_ms;
		self.max_ttft_ms = self.max_ttft_ms.max(*max_ttft_ms);
		self.generation_tokens += generation_tokens;
		self.generation_ms += generation_ms;
		self.cache_read_tokens += cache_read_tokens;
		self.cache_write_tokens += cache_write_tokens;
		self.reasoning_tokens += reasoning_tokens;
	}
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsGroup {
	pub group: BTreeMap<String, Value>,
	#[serde(flatten)]
	pub totals: AnalyticsTotals,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTimeBucket {
	pub start: DateTime<Utc>,
	pub group: BTreeMap<String, Value>,
	#[serde(flatten)]
	pub totals: AnalyticsTotals,
}

pub(crate) fn groups_from_buckets(buckets: &[AnalyticsTimeBucket]) -> Vec<AnalyticsGroup> {
	let mut groups = BTreeMap::<String, AnalyticsGroup>::new();
	for bucket in buckets {
		let key = serde_json::to_string(&bucket.group).unwrap_or_default();
		groups
			.entry(key)
			.or_insert_with(|| AnalyticsGroup {
				group: bucket.group.clone(),
				totals: AnalyticsTotals::default(),
			})
			.totals
			.add(&bucket.totals);
	}
	groups.into_values().collect()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
	pub id: String,
	pub started_at: DateTime<Utc>,
	pub completed_at: DateTime<Utc>,
	pub duration_ms: i64,
	pub trace_id: Option<String>,
	pub span_id: Option<String>,
	pub http_status: Option<i64>,
	pub error: Option<String>,
	pub prompt_preview: Option<String>,
	pub turn: TurnEntry,
	pub gen_ai: GenAiEntry,
	pub usage: UsageEntry,
	pub cost: Option<f64>,
	pub has_payload: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub attributes: Option<Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub payload: Option<PayloadEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEntry {
	pub input: Option<TurnKind>,
	pub output: Option<TurnKind>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnKind {
	User,
	Assistant,
	ToolCall,
	ToolResult,
}

fn prompt_preview(prompt: Option<&Value>) -> Option<String> {
	let messages = prompt?.as_array()?;
	messages.iter().rev().find_map(|message| {
		if message.get("role").and_then(Value::as_str) != Some("user") {
			return None;
		}
		let text = message
			.get("parts")?
			.as_array()?
			.iter()
			.filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
			.filter_map(|part| part.get("text").and_then(Value::as_str))
			.collect::<Vec<_>>()
			.join("\n");
		let text = text.trim();
		if text.is_empty() {
			None
		} else if text.chars().nth(180).is_some() {
			Some(format!("{}...", text.chars().take(177).collect::<String>()))
		} else {
			Some(text.to_string())
		}
	})
}

fn turn_kind(messages: Option<&Value>) -> Option<TurnKind> {
	// Classify the final meaningful normalized part; reasoning and system text are not turns.
	messages?.as_array()?.iter().rev().find_map(|message| {
		let role = message.get("role").and_then(Value::as_str)?;
		message
			.get("parts")?
			.as_array()?
			.iter()
			.rev()
			.find_map(|part| match part.get("type").and_then(Value::as_str)? {
				"toolCall" => Some(TurnKind::ToolCall),
				"toolResult" => Some(TurnKind::ToolResult),
				"text"
					if part
						.get("text")
						.and_then(Value::as_str)
						.is_some_and(|text| !text.trim().is_empty()) =>
				{
					match role {
						"user" => Some(TurnKind::User),
						"assistant" => Some(TurnKind::Assistant),
						_ => None,
					}
				},
				_ => None,
			})
	})
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenAiEntry {
	pub operation_name: Option<String>,
	pub provider_name: Option<String>,
	pub request_model: Option<String>,
	pub response_model: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
	pub input_tokens: Option<i64>,
	pub output_tokens: Option<i64>,
	pub total_tokens: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadEntry {
	pub request_prompt: Option<Value>,
	pub response_completion: Option<Value>,
}

pub(crate) fn limit(limit: Option<i64>) -> i64 {
	limit.unwrap_or(100).clamp(1, 500)
}

pub(crate) fn encode_cursor(completed_at: DateTime<Utc>, id: &str) -> String {
	format!("{}|{}", completed_at.to_rfc3339(), id)
}

pub(crate) fn decode_cursor(cursor: &str) -> anyhow::Result<(DateTime<Utc>, String)> {
	let (completed_at, id) = cursor
		.split_once('|')
		.ok_or_else(|| anyhow::anyhow!("invalid cursor"))?;
	Ok((completed_at.parse::<DateTime<Utc>>()?, id.to_string()))
}

pub(crate) fn attr_value(value: &Value) -> Option<String> {
	match value {
		Value::Null => None,
		Value::Bool(value) => Some(value.to_string()),
		Value::Number(value) => Some(value.to_string()),
		Value::String(value) => Some(value.clone()),
		Value::Array(_) | Value::Object(_) => None,
	}
}

pub(crate) fn attr_filter_values(value: &Value) -> Option<Vec<String>> {
	match value {
		Value::Array(values) => values.iter().map(attr_value).collect(),
		_ => attr_value(value).map(|value| vec![value]),
	}
}

pub(crate) fn promoted_attribute_column(key: &str) -> Option<&'static str> {
	match key {
		"agentgateway.user" => Some("agentgateway_user"),
		"agentgateway.group" => Some("agentgateway_group"),
		"user_agent.name" => Some("user_agent_name"),
		_ => None,
	}
}

pub(crate) fn analytics_window(
	time_range: Option<TimeRange>,
	bucket_count: Option<i64>,
	bucket_seconds: Option<i64>,
) -> (TimeRange, DateTime<Utc>, DateTime<Utc>, i64) {
	let now = Utc::now();
	let from = time_range.as_ref().and_then(|range| range.from);
	let to = time_range
		.as_ref()
		.and_then(|range| range.to)
		.unwrap_or(now);
	let from = from.unwrap_or_else(|| to - Duration::hours(24));
	let to = if to > from {
		to
	} else {
		from + Duration::hours(24)
	};
	let span_seconds = (to - from).num_seconds().max(1);
	let bucket_seconds = bucket_seconds
		.map(|seconds| seconds.clamp(1, span_seconds))
		.unwrap_or_else(|| {
			let bucket_count = bucket_count.unwrap_or(96).clamp(1, 500);
			((span_seconds + bucket_count - 1) / bucket_count).max(1)
		});
	(
		TimeRange {
			from: Some(from),
			to: Some(to),
		},
		from,
		to,
		bucket_seconds,
	)
}

enum Backend {
	Sqlite(sqlite::SqliteLogStore),
	Postgres(postgres::PostgresLogStore),
}

impl Backend {
	async fn connect(
		cfg: &Config,
		pool: Option<crate::database::DatabasePool>,
	) -> anyhow::Result<Self> {
		let pool = match pool {
			Some(pool) => pool,
			None => {
				crate::database::DatabasePool::connect_with_max_connections(&cfg.url, cfg.max_connections)
					.await?
			},
		};
		match pool {
			crate::database::DatabasePool::Sqlite(pool) => {
				Ok(Self::Sqlite(sqlite::SqliteLogStore::from_pool(pool).await?))
			},
			crate::database::DatabasePool::Postgres(pool) => Ok(Self::Postgres(
				postgres::PostgresLogStore::from_pool(pool).await?,
			)),
		}
	}

	async fn insert_batch(&self, records: &[StoredRequestLog]) -> anyhow::Result<()> {
		match self {
			Self::Sqlite(store) => store.insert_batch(records).await,
			Self::Postgres(store) => store.insert_batch(records).await,
		}
	}

	async fn search(&self, request: SearchRequest) -> anyhow::Result<SearchResponse> {
		match self {
			Self::Sqlite(store) => store.search(request).await,
			Self::Postgres(store) => store.search(request).await,
		}
	}

	async fn get(&self, request: GetRequest) -> anyhow::Result<GetResponse> {
		match self {
			Self::Sqlite(store) => store.get(request).await,
			Self::Postgres(store) => store.get(request).await,
		}
	}

	async fn analytics_summary(
		&self,
		request: AnalyticsSummaryRequest,
	) -> anyhow::Result<AnalyticsSummaryResponse> {
		match self {
			Self::Sqlite(store) => store.analytics_summary(request).await,
			Self::Postgres(store) => store.analytics_summary(request).await,
		}
	}

	async fn tail(&self, request: TailRequest) -> anyhow::Result<TailResponse> {
		match self {
			Self::Sqlite(store) => store.tail(request).await,
			Self::Postgres(store) => store.tail(request).await,
		}
	}
}
