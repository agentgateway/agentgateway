UPDATE request_logs SET
	cache_read_input_tokens = CASE WHEN jsonb_typeof(attributes_json -> 'gen_ai.usage.cache_read.input_tokens') = 'number' THEN LEAST((attributes_json ->> 'gen_ai.usage.cache_read.input_tokens')::NUMERIC, 9223372036854775807)::BIGINT END,
	cache_creation_input_tokens = CASE WHEN jsonb_typeof(attributes_json -> 'gen_ai.usage.cache_creation.input_tokens') = 'number' THEN LEAST((attributes_json ->> 'gen_ai.usage.cache_creation.input_tokens')::NUMERIC, 9223372036854775807)::BIGINT END,
	reasoning_tokens = CASE WHEN jsonb_typeof(attributes_json -> 'gen_ai.usage.reasoning_tokens') = 'number' THEN LEAST((attributes_json ->> 'gen_ai.usage.reasoning_tokens')::NUMERIC, 9223372036854775807)::BIGINT END,
	time_to_first_token_ms = CASE WHEN jsonb_typeof(attributes_json -> 'agw.ai.time_to_first_token') = 'number' THEN (attributes_json ->> 'agw.ai.time_to_first_token')::DOUBLE PRECISION * 1000 END,
	time_per_output_token_ms = CASE WHEN jsonb_typeof(attributes_json -> 'agw.ai.time_per_output_token') = 'number' THEN (attributes_json ->> 'agw.ai.time_per_output_token')::DOUBLE PRECISION * 1000 END
WHERE gen_ai_provider_name IS NOT NULL;
