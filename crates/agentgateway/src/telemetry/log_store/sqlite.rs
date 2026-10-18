use serde_json::Value;
use sqlx::types::Json;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, query_builder};

use super::{
	AnalyticsGroup, AnalyticsSummaryRequest, AnalyticsSummaryResponse, AnalyticsTimeBucket,
	GenAiEntry, GetRequest, GetResponse, GroupBy, GroupByField, LogEntry, LogFilters, PayloadEntry,
	SearchRequest, SearchResponse, StoredRequestLog, StoredRequestLogPayload, TailRequest,
	TailResponse, TimeRange, TurnEntry, UsageEntry, analytics_window, attr_filter_values,
	decode_cursor, encode_cursor, escape_like, limit, promoted_attribute_column, prompt_preview,
	search_term, turn_kind,
};

pub struct SqliteLogStore {
	pool: SqlitePool,
}

const ANALYTICS_FILTER_OPTION_LIMIT: i64 = 500;
// Leave room below SQLite's 32,766-parameter limit for two more log fields or one payload field.
const LOG_INSERT_CHUNK_SIZE: usize = 1_400;
const PAYLOAD_INSERT_CHUNK_SIZE: usize = 8_000;
const INSERT_LOG_PREFIX: &str = r#"
INSERT INTO request_logs (
	id, started_at, completed_at, duration_ms, trace_id, span_id, http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, agentgateway_user, agentgateway_group,
	user_agent_name, has_payload, attributes_json
) 
"#;

const INSERT_PAYLOAD_PREFIX: &str = r#"
INSERT INTO request_log_payloads (log_id, request_prompt_json, response_completion_json)
"#;

fn push_request_log_row(
	row: &mut query_builder::Separated<'_, Sqlite, &'static str>,
	record: &StoredRequestLog,
) {
	row
		.push_bind(&record.id)
		.push_bind(record.started_at)
		.push_bind(record.completed_at)
		.push_bind(record.duration_ms)
		.push_bind(&record.trace_id)
		.push_bind(&record.span_id)
		.push_bind(record.http_status)
		.push_bind(&record.error)
		.push_bind(&record.gen_ai_operation_name)
		.push_bind(&record.gen_ai_provider_name)
		.push_bind(&record.gen_ai_request_model)
		.push_bind(&record.gen_ai_response_model)
		.push_bind(record.input_tokens)
		.push_bind(record.output_tokens)
		.push_bind(record.total_tokens)
		.push_bind(record.cost)
		.push_bind(&record.agentgateway_user)
		.push_bind(&record.agentgateway_group)
		.push_bind(&record.user_agent_name)
		.push_bind(record.has_payload)
		.push_bind(record.attributes_json.as_ref());
}

fn push_request_log_payload_row(
	row: &mut query_builder::Separated<'_, Sqlite, &'static str>,
	record: &StoredRequestLog,
	payload: &StoredRequestLogPayload,
) {
	row
		.push_bind(&record.id)
		.push_bind(payload.request_prompt_json.as_ref().map(Json))
		.push_bind(payload.response_completion_json.as_ref().map(Json));
}

impl SqliteLogStore {
	pub async fn from_pool(pool: SqlitePool) -> anyhow::Result<Self> {
		sqlx::raw_sql(SCHEMA).execute(&pool).await?;
		Ok(Self { pool })
	}

	pub async fn insert_batch(&self, records: &[StoredRequestLog]) -> anyhow::Result<()> {
		if records.is_empty() {
			return Ok(());
		}
		let mut tx = self.pool.begin().await?;
		for chunk in records.chunks(LOG_INSERT_CHUNK_SIZE) {
			let mut logs = QueryBuilder::<Sqlite>::new(INSERT_LOG_PREFIX);
			logs.push_values(chunk, |mut row, record| {
				push_request_log_row(&mut row, record);
			});
			logs.build().execute(&mut *tx).await?;
		}

		let payload_records = records
			.iter()
			.filter_map(|record| record.payload.as_ref().map(|payload| (record, payload)))
			.collect::<Vec<_>>();
		for chunk in payload_records.chunks(PAYLOAD_INSERT_CHUNK_SIZE) {
			let mut payloads = QueryBuilder::<Sqlite>::new(INSERT_PAYLOAD_PREFIX);
			payloads.push_values(chunk, |mut row, (record, payload)| {
				push_request_log_payload_row(&mut row, record, payload);
			});
			payloads.build().execute(&mut *tx).await?;
		}
		tx.commit().await?;
		Ok(())
	}

	pub async fn search(&self, request: SearchRequest) -> anyhow::Result<SearchResponse> {
		let limit = limit(request.limit);
		let mut qb = QueryBuilder::<Sqlite>::new(format!("{SELECT_LOGS} WHERE 1=1"));
		push_filters(&mut qb, request.time_range.as_ref(), &request.filters);
		if let Some(cursor) = request.cursor.as_deref() {
			let (completed_at, id) = decode_cursor(cursor)?;
			qb.push(" AND (completed_at, id) < (")
				.push_bind(completed_at)
				.push(", ")
				.push_bind(id)
				.push(")");
		}
		qb.push(" ORDER BY completed_at DESC, id DESC LIMIT ");
		qb.push_bind(limit + 1);
		let rows = qb.build().fetch_all(&self.pool).await?;
		let mut logs = rows
			.into_iter()
			.map(|row| row_to_log(row, request.include_attributes, false))
			.collect::<Result<Vec<_>, _>>()?;
		let next_cursor = if logs.len() > limit as usize {
			let _ = logs.pop();
			logs
				.last()
				.map(|log| encode_cursor(log.completed_at, &log.id))
		} else {
			None
		};
		Ok(SearchResponse { logs, next_cursor })
	}

	pub async fn get(&self, request: GetRequest) -> anyhow::Result<GetResponse> {
		let row = sqlx::query(SELECT_LOG_BY_ID)
			.bind(request.id)
			.fetch_optional(&self.pool)
			.await?;
		let log = row
			.map(|row| row_to_log(row, true, request.include_payload))
			.transpose()?;
		Ok(GetResponse { log })
	}

	pub async fn analytics_summary(
		&self,
		request: AnalyticsSummaryRequest,
	) -> anyhow::Result<AnalyticsSummaryResponse> {
		let (time_range, from, _to, bucket_seconds) = analytics_window(
			request.time_range,
			request.bucket_count,
			request.bucket_seconds,
		);
		let mut qb = QueryBuilder::<Sqlite>::new("SELECT CAST((unixepoch(completed_at) - ");
		qb.push_bind(from.timestamp());
		qb.push(") / ");
		qb.push_bind(bucket_seconds);
		qb.push(" AS INTEGER) AS bucket_index");
		if !request.group_by.is_empty() {
			qb.push(", ");
			push_group_select(&mut qb, &request.group_by);
		}
		qb.push(", COUNT(*) AS requests, COALESCE(SUM(total_tokens), 0) AS total_tokens, COALESCE(SUM(cost), 0.0) AS cost FROM request_logs WHERE 1=1");
		push_filters(&mut qb, Some(&time_range), &request.filters);
		qb.push(" GROUP BY bucket_index");
		if !request.group_by.is_empty() {
			for idx in 0..request.group_by.len() {
				qb.push(format!(", g{idx}"));
			}
		}
		qb.push(" ORDER BY bucket_index ASC");
		let rows = qb.build().fetch_all(&self.pool).await?;
		let buckets = rows
			.into_iter()
			.map(|row| row_to_analytics_bucket(row, from, bucket_seconds, &request.group_by))
			.collect::<Result<Vec<_>, _>>()?;
		let groups = groups_from_buckets(&buckets);
		let filter_options = self.analytics_filter_options(&time_range).await?;
		Ok(AnalyticsSummaryResponse {
			time_range,
			bucket_seconds,
			buckets,
			groups,
			filter_options,
		})
	}

	async fn analytics_filter_options(
		&self,
		time_range: &TimeRange,
	) -> anyhow::Result<std::collections::BTreeMap<String, Vec<String>>> {
		let mut options = analytics_filter_option_map();
		self
			.push_distinct_column_options(
				&mut options,
				"requestModel",
				"gen_ai_request_model",
				time_range,
			)
			.await?;
		self
			.push_distinct_column_options(&mut options, "provider", "gen_ai_provider_name", time_range)
			.await?;
		self
			.push_distinct_attribute_options(&mut options, "agentgateway.user", time_range)
			.await?;
		self
			.push_distinct_attribute_options(&mut options, "agentgateway.group", time_range)
			.await?;
		self
			.push_distinct_attribute_options(&mut options, "user_agent.name", time_range)
			.await?;
		for values in options.values_mut() {
			values.sort();
		}
		Ok(options)
	}

	async fn push_distinct_column_options(
		&self,
		options: &mut std::collections::BTreeMap<String, Vec<String>>,
		key: &str,
		column: &'static str,
		time_range: &TimeRange,
	) -> anyhow::Result<()> {
		let mut qb = QueryBuilder::<Sqlite>::new("SELECT DISTINCT ");
		qb.push(column);
		qb.push(" AS value FROM request_logs");
		if let Some(index) = distinct_option_index(column) {
			qb.push(" INDEXED BY ");
			qb.push(index);
		}
		qb.push(" WHERE ");
		qb.push(column);
		qb.push(" IS NOT NULL AND ");
		qb.push(column);
		qb.push(" != ''");
		push_filters(&mut qb, Some(time_range), &LogFilters::default());
		qb.push(" ORDER BY value LIMIT ");
		qb.push_bind(ANALYTICS_FILTER_OPTION_LIMIT);
		for row in qb.build().fetch_all(&self.pool).await? {
			push_filter_option(options, key, row.try_get::<Option<String>, _>("value")?);
		}
		Ok(())
	}

	async fn push_distinct_attribute_options(
		&self,
		options: &mut std::collections::BTreeMap<String, Vec<String>>,
		key: &str,
		time_range: &TimeRange,
	) -> anyhow::Result<()> {
		if let Some(column) = promoted_attribute_column(key) {
			return self
				.push_distinct_column_options(options, key, column, time_range)
				.await;
		}
		let mut qb = QueryBuilder::<Sqlite>::new("SELECT DISTINCT CAST(");
		push_attr_extract_query(&mut qb, key);
		qb.push(" AS TEXT) AS value FROM request_logs WHERE 1=1");
		push_filters(&mut qb, Some(time_range), &LogFilters::default());
		qb.push(" ORDER BY value LIMIT ");
		qb.push_bind(ANALYTICS_FILTER_OPTION_LIMIT);
		for row in qb.build().fetch_all(&self.pool).await? {
			push_filter_option(options, key, row.try_get::<Option<String>, _>("value")?);
		}
		Ok(())
	}

	pub async fn tail(&self, request: TailRequest) -> anyhow::Result<TailResponse> {
		let limit = limit(request.limit);
		let mut qb = QueryBuilder::<Sqlite>::new(format!("{SELECT_LOGS} WHERE 1=1"));
		push_filters(&mut qb, None, &request.filters);
		if let Some(cursor) = request.cursor.as_deref() {
			let (completed_at, id) = decode_cursor(cursor)?;
			qb.push(" AND (completed_at > ");
			qb.push_bind(completed_at);
			qb.push(" OR (completed_at = ");
			qb.push_bind(completed_at);
			qb.push(" AND id > ");
			qb.push_bind(id);
			qb.push("))");
		}
		qb.push(" ORDER BY completed_at ASC, id ASC LIMIT ");
		qb.push_bind(limit);
		let rows = qb.build().fetch_all(&self.pool).await?;
		let logs = rows
			.into_iter()
			.map(|row| row_to_log(row, request.include_attributes, false))
			.collect::<Result<Vec<_>, _>>()?;
		let next_cursor = logs
			.last()
			.map(|log| encode_cursor(log.completed_at, &log.id));
		Ok(TailResponse { logs, next_cursor })
	}
}

fn groups_from_buckets(buckets: &[AnalyticsTimeBucket]) -> Vec<AnalyticsGroup> {
	let mut groups = std::collections::BTreeMap::<String, AnalyticsGroup>::new();
	for bucket in buckets {
		let key = serde_json::to_string(&bucket.group).unwrap_or_default();
		let group = groups.entry(key).or_insert_with(|| AnalyticsGroup {
			group: bucket.group.clone(),
			requests: 0,
			total_tokens: 0,
			cost: 0.0,
		});
		group.requests += bucket.requests;
		group.total_tokens += bucket.total_tokens;
		group.cost += bucket.cost;
	}
	groups.into_values().collect()
}

fn analytics_filter_option_map() -> std::collections::BTreeMap<String, Vec<String>> {
	[
		("requestModel".to_string(), Vec::new()),
		("provider".to_string(), Vec::new()),
		("agentgateway.user".to_string(), Vec::new()),
		("agentgateway.group".to_string(), Vec::new()),
		("user_agent.name".to_string(), Vec::new()),
	]
	.into()
}

fn distinct_option_index(column: &str) -> Option<&'static str> {
	match column {
		"gen_ai_provider_name" => Some("idx_request_logs_gen_ai_completed_at"),
		"gen_ai_request_model" => Some("idx_request_logs_request_model_completed_at"),
		"agentgateway_user" => Some("idx_request_logs_user_completed_at"),
		"agentgateway_group" => Some("idx_request_logs_group_completed_at"),
		"user_agent_name" => Some("idx_request_logs_user_agent_completed_at"),
		_ => None,
	}
}

fn push_filter_option(
	options: &mut std::collections::BTreeMap<String, Vec<String>>,
	key: &str,
	value: Option<String>,
) {
	let Some(value) = value
		.map(|value| value.trim().to_string())
		.filter(|value| !value.is_empty())
	else {
		return;
	};
	let values = options.entry(key.to_string()).or_default();
	if !values.contains(&value) {
		values.push(value);
	}
}

fn push_filters(
	qb: &mut QueryBuilder<Sqlite>,
	time_range: Option<&TimeRange>,
	filters: &LogFilters,
) {
	if let Some(from) = time_range.and_then(|r| r.from) {
		qb.push(" AND completed_at >= ");
		qb.push_bind(from);
	}
	if let Some(to) = time_range.and_then(|r| r.to) {
		qb.push(" AND completed_at < ");
		qb.push_bind(to);
	}
	push_in(qb, "http_status", &filters.http_status);
	push_in(qb, "gen_ai_provider_name", &filters.provider);
	push_in(qb, "gen_ai_request_model", &filters.request_model);
	push_in(qb, "gen_ai_response_model", &filters.response_model);
	if let Some(trace_id) = &filters.trace_id {
		qb.push(" AND trace_id = ");
		qb.push_bind(trace_id);
	}
	if let Some(has_payload) = filters.has_payload {
		qb.push(" AND has_payload = ");
		qb.push_bind(has_payload);
	}
	if let Some(search) = search_term(filters) {
		let pattern = format!("%{}%", escape_like(search));
		qb.push(" AND (agentgateway_user LIKE ");
		qb.push_bind(pattern.clone());
		qb.push(" ESCAPE '\\' OR agentgateway_group LIKE ");
		qb.push_bind(pattern);
		qb.push(" ESCAPE '\\')");
	}
	for (key, value) in &filters.attributes {
		let Some(values) = attr_filter_values(value) else {
			qb.push(" AND 1=0");
			continue;
		};
		if values.is_empty() {
			qb.push(" AND 1=0");
			continue;
		}
		if let Some(column) = promoted_attribute_column(key) {
			push_in(qb, column, &values);
			continue;
		}
		qb.push(" AND CAST(");
		push_attr_extract_query(qb, key);
		qb.push(" AS TEXT) IN (");
		let mut separated = qb.separated(", ");
		for value in values {
			separated.push_bind(sqlite_attr_value(value));
		}
		separated.push_unseparated(")");
	}
}

fn push_in<T>(qb: &mut QueryBuilder<Sqlite>, column: &str, values: &[T])
where
	T: for<'q> sqlx::Encode<'q, Sqlite> + sqlx::Type<Sqlite> + Send + Sync,
{
	if values.is_empty() {
		return;
	}
	qb.push(" AND ");
	qb.push(column);
	qb.push(" IN (");
	let mut separated = qb.separated(", ");
	for value in values {
		separated.push_bind(value);
	}
	separated.push_unseparated(")");
}

fn push_group_select(qb: &mut QueryBuilder<Sqlite>, group_by: &[GroupBy]) {
	for (idx, group) in group_by.iter().enumerate() {
		if idx > 0 {
			qb.push(", ");
		}
		match group.field {
			GroupByField::Provider => {
				qb.push(format!("gen_ai_provider_name AS g{idx}"));
			},
			GroupByField::RequestModel => {
				qb.push(format!("gen_ai_request_model AS g{idx}"));
			},
			GroupByField::ResponseModel => {
				qb.push(format!("gen_ai_response_model AS g{idx}"));
			},
			GroupByField::HttpStatus => {
				qb.push(format!("CAST(http_status AS TEXT) AS g{idx}"));
			},
			GroupByField::Attributes => {
				if let Some(column) = group.key.as_deref().and_then(promoted_attribute_column) {
					qb.push(format!("{column} AS g{idx}"));
				} else {
					qb.push("CAST(");
					push_attr_extract_query(qb, group.key.as_deref().unwrap_or_default());
					qb.push(format!(" AS TEXT) AS g{idx}"));
				}
			},
		};
	}
}

fn row_to_analytics_bucket(
	row: sqlx::sqlite::SqliteRow,
	from: chrono::DateTime<chrono::Utc>,
	bucket_seconds: i64,
	group_by: &[GroupBy],
) -> anyhow::Result<AnalyticsTimeBucket> {
	let bucket_index: i64 = row.try_get("bucket_index")?;
	let mut group = std::collections::BTreeMap::new();
	for (idx, spec) in group_by.iter().enumerate() {
		let value: Option<String> = row.try_get(format!("g{idx}").as_str())?;
		group.insert(
			group_key(spec),
			value.map(Value::String).unwrap_or(Value::Null),
		);
	}
	Ok(AnalyticsTimeBucket {
		start: from + chrono::Duration::seconds(bucket_index * bucket_seconds),
		group,
		requests: row.try_get("requests")?,
		total_tokens: row.try_get("total_tokens")?,
		cost: row.try_get("cost")?,
	})
}

fn row_to_log(
	row: sqlx::sqlite::SqliteRow,
	include_attributes: bool,
	include_payload: bool,
) -> anyhow::Result<LogEntry> {
	let attributes: Json<Value> = row.try_get("attributes_json")?;
	let request_prompt: Option<Json<Value>> = row.try_get("request_prompt_json")?;
	let response_completion: Option<Json<Value>> = row.try_get("response_completion_json")?;
	let prompt_preview = prompt_preview(request_prompt.as_ref().map(|value| &value.0));
	let turn = TurnEntry {
		input: turn_kind(request_prompt.as_ref().map(|value| &value.0)),
		output: turn_kind(response_completion.as_ref().map(|value| &value.0)),
	};
	let payload = if include_payload {
		Some(PayloadEntry {
			request_prompt: request_prompt.map(|v| v.0),
			response_completion: response_completion.map(|v| v.0),
		})
	} else {
		None
	};
	Ok(LogEntry {
		id: row.try_get("id")?,
		started_at: row.try_get("started_at")?,
		completed_at: row.try_get("completed_at")?,
		duration_ms: row.try_get("duration_ms")?,
		trace_id: row.try_get("trace_id")?,
		span_id: row.try_get("span_id")?,
		http_status: row.try_get("http_status")?,
		error: row.try_get("error")?,
		prompt_preview,
		turn,
		gen_ai: GenAiEntry {
			operation_name: row.try_get("gen_ai_operation_name")?,
			provider_name: row.try_get("gen_ai_provider_name")?,
			request_model: row.try_get("gen_ai_request_model")?,
			response_model: row.try_get("gen_ai_response_model")?,
		},
		usage: UsageEntry {
			input_tokens: row.try_get("input_tokens")?,
			output_tokens: row.try_get("output_tokens")?,
			total_tokens: row.try_get("total_tokens")?,
		},
		cost: row.try_get("cost")?,
		has_payload: row.try_get("has_payload")?,
		attributes: include_attributes.then_some(attributes.0),
		payload,
	})
}

fn group_key(group: &GroupBy) -> String {
	match group.field {
		GroupByField::Provider => "provider".to_string(),
		GroupByField::RequestModel => "requestModel".to_string(),
		GroupByField::ResponseModel => "responseModel".to_string(),
		GroupByField::HttpStatus => "httpStatus".to_string(),
		GroupByField::Attributes => group
			.key
			.clone()
			.unwrap_or_else(|| "attributes".to_string()),
	}
}

fn json_path(key: &str) -> String {
	format!(
		"$.{}",
		key
			.split('.')
			.map(|part| format!("\"{part}\""))
			.collect::<Vec<_>>()
			.join(".")
	)
}

fn flat_json_path(key: &str) -> String {
	format!("$.\"{}\"", key.replace('"', "\\\""))
}

fn push_attr_extract_query(qb: &mut QueryBuilder<Sqlite>, key: &str) {
	qb.push("COALESCE(json_extract(attributes_json, ");
	qb.push_bind(flat_json_path(key));
	qb.push("), json_extract(attributes_json, ");
	qb.push_bind(json_path(key));
	qb.push("))");
}

fn sqlite_attr_value(value: String) -> String {
	match value.as_str() {
		"true" => "1".to_string(),
		"false" => "0".to_string(),
		_ => value,
	}
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS request_logs (
	id TEXT PRIMARY KEY,
	started_at TEXT NOT NULL,
	completed_at TEXT NOT NULL,
	duration_ms INTEGER NOT NULL,
	trace_id TEXT,
	span_id TEXT,
	http_status INTEGER,
	error TEXT,
	gen_ai_operation_name TEXT,
	gen_ai_provider_name TEXT,
	gen_ai_request_model TEXT,
	gen_ai_response_model TEXT,
	input_tokens INTEGER,
	output_tokens INTEGER,
	total_tokens INTEGER,
	cost REAL,
	agentgateway_user TEXT,
	agentgateway_group TEXT,
	user_agent_name TEXT,
	has_payload INTEGER NOT NULL,
	attributes_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS request_log_payloads (
	log_id TEXT PRIMARY KEY REFERENCES request_logs(id) ON DELETE CASCADE,
	request_prompt_json TEXT,
	response_completion_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_request_logs_completed_at ON request_logs(completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_usage_completed_at ON request_logs(completed_at DESC, total_tokens, cost);
CREATE INDEX IF NOT EXISTS idx_request_logs_http_status_completed_at ON request_logs(http_status, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_gen_ai_completed_at ON request_logs(gen_ai_provider_name, gen_ai_request_model, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_request_model_completed_at ON request_logs(gen_ai_request_model, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_user_completed_at ON request_logs(agentgateway_user, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_group_completed_at ON request_logs(agentgateway_group, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_user_agent_completed_at ON request_logs(user_agent_name, completed_at DESC, id DESC);
"#;

const SELECT_LOGS: &str = r#"
SELECT id, started_at, completed_at, duration_ms, trace_id, span_id, http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, has_payload, attributes_json,
	request_prompt_json, response_completion_json
FROM request_logs
LEFT JOIN request_log_payloads ON request_logs.id = request_log_payloads.log_id
"#;

const SELECT_LOG_BY_ID: &str = r#"
SELECT id, started_at, completed_at, duration_ms, trace_id, span_id, http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, has_payload, attributes_json,
	request_prompt_json, response_completion_json
FROM request_logs
LEFT JOIN request_log_payloads ON request_logs.id = request_log_payloads.log_id
WHERE request_logs.id = ?
"#;

#[cfg(test)]
mod tests {
	use chrono::DateTime;

	use super::*;

	/// A log row carrying only the identity fields, which are what the search filter reads.
	fn log_with_identity(id: &str, user: Option<&str>, group: Option<&str>) -> StoredRequestLog {
		StoredRequestLog {
			id: id.to_string(),
			started_at: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp"),
			completed_at: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp"),
			duration_ms: 1,
			trace_id: None,
			span_id: None,
			http_status: None,
			error: None,
			gen_ai_operation_name: None,
			gen_ai_provider_name: None,
			gen_ai_request_model: None,
			gen_ai_response_model: None,
			input_tokens: None,
			output_tokens: None,
			total_tokens: None,
			cost: None,
			agentgateway_user: user.map(str::to_string),
			agentgateway_group: group.map(str::to_string),
			user_agent_name: None,
			has_payload: false,
			attributes_json: "{}".into(),
			payload: None,
		}
	}

	async fn search_ids(store: &SqliteLogStore, search: Option<&str>) -> Vec<String> {
		let response = store
			.search(SearchRequest {
				limit: None,
				cursor: None,
				time_range: None,
				filters: LogFilters {
					search: search.map(str::to_string),
					..LogFilters::default()
				},
				include_attributes: false,
			})
			.await
			.expect("search request logs");
		let mut ids = response
			.logs
			.into_iter()
			.map(|log| log.id)
			.collect::<Vec<_>>();
		ids.sort();
		ids
	}

	#[tokio::test]
	async fn search_matches_partial_user_and_group_names() {
		let store = SqliteLogStore::from_pool(
			SqlitePool::connect("sqlite::memory:")
				.await
				.expect("open in-memory sqlite"),
		)
		.await
		.expect("create log store");

		store
			.insert_batch(&[
				log_with_identity("alice", Some("alice@example.com"), Some("platform")),
				log_with_identity("bob", Some("bob@example.com"), Some("data-eng")),
				log_with_identity("lower", Some("josé@example.com"), Some("equipo")),
				log_with_identity("upper", Some("JOSÉ@example.com"), Some("equipo")),
				log_with_identity("anon", None, None),
			])
			.await
			.expect("insert logs");

		// A user is found by any part of the name, not only the exact value.
		assert_eq!(search_ids(&store, Some("alice")).await, vec!["alice"]);
		assert_eq!(
			search_ids(&store, Some("example.com")).await,
			vec![
				"alice".to_string(),
				"bob".to_string(),
				"lower".to_string(),
				"upper".to_string()
			]
		);
		// The same text also matches the group column.
		assert_eq!(search_ids(&store, Some("data")).await, vec!["bob"]);
		// SQLite `LIKE` compares ASCII case-insensitively.
		assert_eq!(search_ids(&store, Some("ALICE")).await, vec!["alice"]);
		// Beyond ASCII it does not fold, so the case has to match the stored value. Postgres folds
		// with `ILIKE` here, which is why the SQL differs between the two backends.
		assert_eq!(search_ids(&store, Some("josé")).await, vec!["lower"]);
		assert_eq!(search_ids(&store, Some("JOSÉ")).await, vec!["upper"]);
		// A term that matches nothing returns nothing, and no term returns everything.
		assert!(search_ids(&store, Some("nobody")).await.is_empty());
		assert_eq!(search_ids(&store, None).await.len(), 5);
		// Rows without an identity never match a search.
		assert_eq!(search_ids(&store, Some("anon")).await, Vec::<String>::new());
	}

	#[test]
	fn like_pattern_escapes_wildcards_the_user_typed() {
		assert_eq!(escape_like("alice"), "alice");
		assert_eq!(escape_like("50%"), "50\\%");
		assert_eq!(escape_like("a_b"), "a\\_b");
		assert_eq!(escape_like("a\\b"), "a\\\\b");
	}

	/// The escaping only matters if the stored rows are compared the same way, so this drives the
	/// search for a term that contains each wildcard and expects only the literal match.
	#[tokio::test]
	async fn search_treats_wildcards_in_the_term_literally() {
		let store = SqliteLogStore::from_pool(
			SqlitePool::connect("sqlite::memory:")
				.await
				.expect("open in-memory sqlite"),
		)
		.await
		.expect("create log store");

		store
			.insert_batch(&[
				log_with_identity("percent", Some("50%off"), Some("growth")),
				log_with_identity("underscore", Some("group_a"), Some("growth")),
				log_with_identity("decoy", Some("groupXa"), Some("growth")),
				log_with_identity("backslash", Some("team\\alpha"), Some("growth")),
				log_with_identity("backslash-decoy", Some("teamXalpha"), Some("growth")),
			])
			.await
			.expect("insert logs");

		// Without `ESCAPE`, `_` would match any single character and would also select the decoy.
		assert_eq!(
			search_ids(&store, Some("group_a")).await,
			vec!["underscore"]
		);
		// `%` behaves the same way.
		assert_eq!(search_ids(&store, Some("50%")).await, vec!["percent"]);
		// A backslash is the escape character itself and is still matched literally.
		assert_eq!(
			search_ids(&store, Some("team\\alpha")).await,
			vec!["backslash"]
		);
		assert!(search_ids(&store, Some("teamXalpha")).await == vec!["backslash-decoy"]);
	}

	#[test]
	fn search_term_is_trimmed_and_empty_values_are_ignored() {
		let with = |search: Option<&str>| LogFilters {
			search: search.map(str::to_string),
			..LogFilters::default()
		};

		assert_eq!(search_term(&with(None)), None);
		assert_eq!(search_term(&with(Some(""))), None);
		assert_eq!(search_term(&with(Some("   "))), None);
		assert_eq!(search_term(&with(Some("  alice  "))), Some("alice"));
	}
}
