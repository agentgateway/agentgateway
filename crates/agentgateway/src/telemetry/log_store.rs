use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
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
pub struct Config {
	/// Connection URL for the request log database. A postgres:// or postgresql:// URL uses Postgres; any other value is treated as a SQLite database.
	pub url: String,
}

#[derive(Clone)]
pub struct RequestLogStore {
	tx: Sender<LogStoreMsg>,
}

impl RequestLogStore {
	pub fn emit(&self, record: StoredRequestLog) {
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

pub fn emit(record: StoredRequestLog) {
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
	Record(StoredRequestLog),
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
		LogStoreMsg::Record(record) => {
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
}

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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsGroup {
	pub group: BTreeMap<String, Value>,
	pub requests: i64,
	pub total_tokens: i64,
	pub cost: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTimeBucket {
	pub start: DateTime<Utc>,
	pub group: BTreeMap<String, Value>,
	pub requests: i64,
	pub total_tokens: i64,
	pub cost: f64,
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
			None => crate::database::DatabasePool::connect(&cfg.url).await?,
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
