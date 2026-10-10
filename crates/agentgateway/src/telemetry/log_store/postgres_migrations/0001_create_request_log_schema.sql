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
