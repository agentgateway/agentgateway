ALTER TABLE request_logs
	ADD COLUMN IF NOT EXISTS cache_read_input_tokens BIGINT,
	ADD COLUMN IF NOT EXISTS cache_creation_input_tokens BIGINT,
	ADD COLUMN IF NOT EXISTS reasoning_tokens BIGINT,
	ADD COLUMN IF NOT EXISTS time_to_first_token_ms DOUBLE PRECISION,
	ADD COLUMN IF NOT EXISTS time_per_output_token_ms DOUBLE PRECISION;
