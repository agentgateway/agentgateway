use anyhow::Context;
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool, query_builder};

use super::{
	ANALYTICS_TOTALS_SELECT, AnalyticsSummaryRequest, AnalyticsSummaryResponse, AnalyticsTimeBucket,
	AnalyticsTotals, GenAiEntry, GetRequest, GetResponse, GroupBy, GroupByField, LogEntry,
	LogFilters, MIGRATIONS_TABLE, OUTPUT_TOKENS_PER_SECOND_BOUNDS, PayloadEntry, SearchRequest,
	SearchResponse, StoredRequestLog, StoredRequestLogPayload, TTFT_BOUNDS_MS, TailRequest,
	TailResponse, TimeRange, TurnEntry, UsageEntry, analytics_window, attr_filter_values,
	decode_cursor, encode_cursor, groups_from_buckets, histogram_bucket_sql, limit,
	promoted_attribute_column, prompt_preview, turn_kind,
};

pub struct SqliteLogStore {
	pool: SqlitePool,
}

const ANALYTICS_FILTER_OPTION_LIMIT: i64 = 500;
// Leave room below SQLite's 32,766-parameter limit for two more log fields or one payload field.
const LOG_INSERT_CHUNK_SIZE: usize = 1_150;
const PAYLOAD_INSERT_CHUNK_SIZE: usize = 8_000;
const INSERT_LOG_PREFIX: &str = r#"
INSERT INTO request_logs (
	id, started_at, completed_at, duration_ms, trace_id, span_id, http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cache_read_input_tokens, cache_creation_input_tokens,
	reasoning_tokens, time_to_first_token_ms, time_per_output_token_ms, cost, agentgateway_user,
	agentgateway_group, user_agent_name, has_payload, attributes_json
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
		.push_bind(record.cache_read_input_tokens)
		.push_bind(record.cache_creation_input_tokens)
		.push_bind(record.reasoning_tokens)
		.push_bind(record.time_to_first_token_ms)
		.push_bind(record.time_per_output_token_ms)
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
		let started = std::time::Instant::now();
		tracing::info!("initializing request log database schema");
		let mut migrator = sqlx_macros::migrate!("./src/telemetry/log_store/sqlite_migrations");
		migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
		migrator
			.run(&pool)
			.await
			.context("failed to migrate request log database schema")?;
		tracing::info!(elapsed = ?started.elapsed(), "request log database schema is ready");
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
		qb.push(", ");
		qb.push(ANALYTICS_TOTALS_SELECT);
		qb.push(" FROM request_logs WHERE 1=1");
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

/// Databases created before the analytics columns existed get them added and backfilled from
/// `attributes_json`.
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
			GroupByField::Ttft => {
				let bucket = histogram_bucket_sql("time_to_first_token_ms", TTFT_BOUNDS_MS);
				qb.push(format!("{bucket} AS g{idx}"));
			},
			GroupByField::OutputTokensPerSecond => {
				let bucket = histogram_bucket_sql(
					"1000.0 / NULLIF(time_per_output_token_ms, 0)",
					OUTPUT_TOKENS_PER_SECOND_BOUNDS,
				);
				qb.push(format!("{bucket} AS g{idx}"));
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
		totals: AnalyticsTotals::from_row(&row)?,
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
		GroupByField::Ttft => "ttft".to_string(),
		GroupByField::OutputTokensPerSecond => "outputTokensPerSecond".to_string(),
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
	use chrono::{TimeZone, Utc};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;

	use super::*;

	#[tokio::test]
	async fn analytics_totals_from_migrated_and_new_rows() -> anyhow::Result<()> {
		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await?;
		// A row written before the analytics columns existed; only attributes_json has the values.
		sqlx::raw_sql(include_str!(
			"sqlite_migrations/0001_create_request_log_schema.sql"
		))
		.execute(&pool)
		.await?;
		sqlx::raw_sql(
			r#"
INSERT INTO request_logs VALUES (
	'old', '2026-10-01T00:00:00+00:00', '2026-10-01T00:00:01+00:00', 1000, NULL, NULL, 200, NULL,
	'chat', 'openai', 'gpt', 'gpt', 1000, 10, 1010, 0.5, NULL, NULL, NULL, 0,
	'{"gen_ai.usage.cache_read.input_tokens":800,"agw.ai.time_to_first_token":0.3,"agw.ai.time_per_output_token":0.01}'
);
"#,
		)
		.execute(&pool)
		.await?;
		let store = SqliteLogStore::from_pool(pool).await?;
		let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 2).unwrap();
		store
			.insert_batch(&[StoredRequestLog {
				id: "new".into(),
				started_at: at,
				completed_at: at,
				duration_ms: 2000,
				trace_id: None,
				span_id: None,
				http_status: Some(500),
				error: None,
				gen_ai_operation_name: Some("chat".into()),
				gen_ai_provider_name: Some("openai".into()),
				gen_ai_request_model: Some("gpt".into()),
				gen_ai_response_model: Some("gpt".into()),
				input_tokens: Some(100),
				output_tokens: Some(50),
				total_tokens: Some(150),
				cache_read_input_tokens: None,
				cache_creation_input_tokens: Some(40),
				reasoning_tokens: Some(20),
				time_to_first_token_ms: Some(1500.0),
				time_per_output_token_ms: Some(20.0),
				cost: Some(0.25),
				agentgateway_user: None,
				agentgateway_group: None,
				user_agent_name: None,
				has_payload: false,
				attributes_json: "{}".into(),
				payload: None,
			}])
			.await?;

		let request = serde_json::from_value(json!({
			"timeRange": { "from": "2026-10-01T00:00:00Z", "to": "2026-10-02T00:00:00Z" },
			"groupBy": [{ "field": "ttft" }, { "field": "outputTokensPerSecond" }],
			"bucketCount": 1,
		}))?;
		let groups = serde_json::to_value(store.analytics_summary(request).await?.groups)?;
		assert_eq!(
			groups,
			json!([
				{
					"group": { "ttft": "250", "outputTokensPerSecond": "100" },
					"requests": 1, "totalTokens": 1010, "cost": 0.5, "inputTokens": 1000,
					"outputTokens": 10, "errors": 0, "durationMs": 1000, "ttftRequests": 1,
					"ttftMs": 300.0, "maxTtftMs": 300.0, "generationTokens": 10, "generationMs": 100.0,
					"cacheReadTokens": 800, "cacheWriteTokens": 0, "reasoningTokens": 0,
				},
				{
					"group": { "ttft": "1000", "outputTokensPerSecond": "50" },
					"requests": 1, "totalTokens": 150, "cost": 0.25, "inputTokens": 100,
					"outputTokens": 50, "errors": 1, "durationMs": 2000, "ttftRequests": 1,
					"ttftMs": 1500.0, "maxTtftMs": 1500.0, "generationTokens": 50, "generationMs": 1000.0,
					"cacheReadTokens": 0, "cacheWriteTokens": 40, "reasoningTokens": 20,
				},
			])
		);
		Ok(())
	}
}
