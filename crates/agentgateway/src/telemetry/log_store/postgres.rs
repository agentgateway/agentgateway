use anyhow::Context;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};

use super::{
	AnalyticsGroup, AnalyticsSummaryRequest, AnalyticsSummaryResponse, AnalyticsTimeBucket,
	GenAiEntry, GetRequest, GetResponse, GroupBy, GroupByField, INSERT_LOG_PREFIX,
	INSERT_PAYLOAD_PREFIX, LogEntry, LogFilters, PayloadEntry, SearchRequest, SearchResponse,
	StoredRequestLog, TailRequest, TailResponse, TimeRange, UsageEntry, analytics_window,
	attr_filter_values, decode_cursor, encode_cursor, limit, promoted_attribute_column,
};

pub struct PostgresLogStore {
	pool: PgPool,
}

const ANALYTICS_FILTER_OPTION_LIMIT: i64 = 500;

impl PostgresLogStore {
	pub async fn connect(url: &str) -> anyhow::Result<Self> {
		let pool = PgPoolOptions::new()
			.max_connections(5)
			.connect(url)
			.await
			.context("failed to connect request log postgres database")?;
		sqlx::raw_sql(SCHEMA).execute(&pool).await?;
		Ok(Self { pool })
	}

	pub async fn insert_batch(&self, records: &[StoredRequestLog]) -> anyhow::Result<()> {
		if records.is_empty() {
			return Ok(());
		}
		let mut tx = self.pool.begin().await?;
		let mut logs = QueryBuilder::<Postgres>::new(INSERT_LOG_PREFIX);
		logs.push_values(records, |mut row, record| {
			push_request_log_row!(row, record);
		});
		logs.build().execute(&mut *tx).await?;

		if records.iter().any(|record| record.payload.is_some()) {
			let mut payloads = QueryBuilder::<Postgres>::new(INSERT_PAYLOAD_PREFIX);
			payloads.push_values(
				records
					.iter()
					.filter_map(|record| record.payload.as_ref().map(|payload| (record, payload))),
				|mut row, (record, payload)| {
					push_request_log_payload_row!(row, record, payload);
				},
			);
			payloads.build().execute(&mut *tx).await?;
		}
		tx.commit().await?;
		Ok(())
	}

	pub async fn search(&self, request: SearchRequest) -> anyhow::Result<SearchResponse> {
		let limit = limit(request.limit);
		let mut qb = QueryBuilder::<Postgres>::new(format!("{SELECT_LOGS} WHERE 1=1"));
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
		let row = if request.include_payload {
			sqlx::query(SELECT_LOG_WITH_PAYLOAD_BY_ID)
				.bind(request.id)
				.fetch_optional(&self.pool)
				.await?
		} else {
			sqlx::query(SELECT_LOG_BY_ID)
				.bind(request.id)
				.fetch_optional(&self.pool)
				.await?
		};
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
		let mut qb =
			QueryBuilder::<Postgres>::new("SELECT CAST(FLOOR((EXTRACT(EPOCH FROM completed_at) - ");
		qb.push_bind(from.timestamp() as f64);
		qb.push(") / ");
		qb.push_bind(bucket_seconds as f64);
		qb.push(") AS BIGINT) AS bucket_index");
		if !request.group_by.is_empty() {
			qb.push(", ");
			push_group_select(&mut qb, &request.group_by);
		}
		qb.push(", COUNT(*) AS requests, COALESCE(SUM(total_tokens), 0)::BIGINT AS total_tokens, COALESCE(SUM(cost), 0.0)::DOUBLE PRECISION AS cost FROM request_logs WHERE 1=1");
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
		let mut qb = QueryBuilder::<Postgres>::new("SELECT DISTINCT ");
		qb.push(column);
		qb.push(" AS value FROM request_logs WHERE ");
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
		let mut qb = QueryBuilder::<Postgres>::new("SELECT DISTINCT attributes_json ->> ");
		qb.push_bind(key);
		qb.push(" AS value FROM request_logs WHERE 1=1");
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
		let mut qb = QueryBuilder::<Postgres>::new(format!("{SELECT_LOGS} WHERE 1=1"));
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
	qb: &mut QueryBuilder<Postgres>,
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
		qb.push(" AND attributes_json ->> ");
		qb.push_bind(key);
		qb.push(" IN (");
		let mut separated = qb.separated(", ");
		for value in values {
			separated.push_bind(value);
		}
		separated.push_unseparated(")");
	}
}

fn push_in<T>(qb: &mut QueryBuilder<Postgres>, column: &str, values: &[T])
where
	T: for<'q> sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres> + Send + Sync,
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

fn push_group_select(qb: &mut QueryBuilder<Postgres>, group_by: &[GroupBy]) {
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
				qb.push(format!("http_status::TEXT AS g{idx}"));
			},
			GroupByField::Attributes => {
				if let Some(column) = group.key.as_deref().and_then(promoted_attribute_column) {
					qb.push(format!("{column} AS g{idx}"));
				} else {
					qb.push("attributes_json ->> ");
					qb.push_bind(group.key.as_deref().unwrap_or_default());
					qb.push(format!(" AS g{idx}"));
				}
			},
		};
	}
}

fn row_to_analytics_bucket(
	row: sqlx::postgres::PgRow,
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
	row: sqlx::postgres::PgRow,
	include_attributes: bool,
	include_payload: bool,
) -> anyhow::Result<LogEntry> {
	let attributes: Json<Value> = row.try_get("attributes_json")?;
	let payload = if include_payload {
		let request_prompt: Option<Json<Value>> = row.try_get("request_prompt_json")?;
		let response_completion: Option<Json<Value>> = row.try_get("response_completion_json")?;
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

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS request_logs (
	id TEXT PRIMARY KEY,
	started_at TIMESTAMPTZ NOT NULL,
	completed_at TIMESTAMPTZ NOT NULL,
	duration_ms BIGINT NOT NULL,
	trace_id TEXT,
	span_id TEXT,
	http_status INTEGER,
	error TEXT,
	gen_ai_operation_name TEXT,
	gen_ai_provider_name TEXT,
	gen_ai_request_model TEXT,
	gen_ai_response_model TEXT,
	input_tokens BIGINT,
	output_tokens BIGINT,
	total_tokens BIGINT,
	cost DOUBLE PRECISION,
	agentgateway_user TEXT,
	agentgateway_group TEXT,
	user_agent_name TEXT,
	has_payload BOOLEAN NOT NULL,
	attributes_json JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS request_log_payloads (
	log_id TEXT PRIMARY KEY REFERENCES request_logs(id) ON DELETE CASCADE,
	request_prompt_json JSONB,
	response_completion_json JSONB
);

CREATE INDEX IF NOT EXISTS idx_request_logs_completed_at ON request_logs(completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_http_status_completed_at ON request_logs(http_status, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_gen_ai_completed_at ON request_logs(gen_ai_provider_name, gen_ai_request_model, completed_at DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_request_model_completed_at ON request_logs(gen_ai_request_model, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_user_completed_at ON request_logs(agentgateway_user, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_group_completed_at ON request_logs(agentgateway_group, completed_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_logs_user_agent_completed_at ON request_logs(user_agent_name, completed_at DESC, id DESC);
"#;

const SELECT_LOGS: &str = r#"
SELECT id, started_at, completed_at, duration_ms, trace_id, span_id, http_status::BIGINT AS http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, has_payload, attributes_json
FROM request_logs
"#;

const SELECT_LOG_BY_ID: &str = r#"
SELECT id, started_at, completed_at, duration_ms, trace_id, span_id, http_status::BIGINT AS http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, has_payload, attributes_json
FROM request_logs
WHERE request_logs.id = $1
"#;

const SELECT_LOG_WITH_PAYLOAD_BY_ID: &str = r#"
SELECT request_logs.id, started_at, completed_at, duration_ms, trace_id, span_id, http_status::BIGINT AS http_status, error,
	gen_ai_operation_name, gen_ai_provider_name, gen_ai_request_model, gen_ai_response_model,
	input_tokens, output_tokens, total_tokens, cost, has_payload, attributes_json,
	request_prompt_json, response_completion_json
FROM request_logs
LEFT JOIN request_log_payloads ON request_logs.id = request_log_payloads.log_id
WHERE request_logs.id = $1
"#;
