ALTER TABLE request_logs ADD COLUMN cache_read_input_tokens INTEGER;
ALTER TABLE request_logs ADD COLUMN cache_creation_input_tokens INTEGER;
ALTER TABLE request_logs ADD COLUMN reasoning_tokens INTEGER;
ALTER TABLE request_logs ADD COLUMN time_to_first_token_ms REAL;
ALTER TABLE request_logs ADD COLUMN time_per_output_token_ms REAL;
UPDATE request_logs SET
	cache_read_input_tokens = CASE WHEN json_type(attributes_json, '$."gen_ai.usage.cache_read.input_tokens"') IN ('integer', 'real') THEN json_extract(attributes_json, '$."gen_ai.usage.cache_read.input_tokens"') END,
	cache_creation_input_tokens = CASE WHEN json_type(attributes_json, '$."gen_ai.usage.cache_creation.input_tokens"') IN ('integer', 'real') THEN json_extract(attributes_json, '$."gen_ai.usage.cache_creation.input_tokens"') END,
	reasoning_tokens = CASE WHEN json_type(attributes_json, '$."gen_ai.usage.reasoning_tokens"') IN ('integer', 'real') THEN json_extract(attributes_json, '$."gen_ai.usage.reasoning_tokens"') END,
	time_to_first_token_ms = CASE WHEN json_type(attributes_json, '$."agw.ai.time_to_first_token"') IN ('integer', 'real') THEN json_extract(attributes_json, '$."agw.ai.time_to_first_token"') * 1000 END,
	time_per_output_token_ms = CASE WHEN json_type(attributes_json, '$."agw.ai.time_per_output_token"') IN ('integer', 'real') THEN json_extract(attributes_json, '$."agw.ai.time_per_output_token"') * 1000 END
WHERE gen_ai_provider_name IS NOT NULL;
