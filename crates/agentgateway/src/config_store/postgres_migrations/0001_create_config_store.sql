CREATE TABLE IF NOT EXISTS agw_config_meta (
	id INTEGER PRIMARY KEY CHECK (id = 1),
	generation BIGINT NOT NULL
);

INSERT INTO agw_config_meta (id, generation) VALUES (1, 1) ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS agw_config_resources (
	kind TEXT NOT NULL,
	id TEXT NOT NULL,
	value_json JSONB NOT NULL,
	revision BIGINT NOT NULL DEFAULT 1,
	created_at TIMESTAMPTZ NOT NULL,
	updated_at TIMESTAMPTZ NOT NULL,
	deleted_at TIMESTAMPTZ,
	PRIMARY KEY (kind, id)
);

CREATE INDEX IF NOT EXISTS idx_agw_config_resources_kind_updated
	ON agw_config_resources(kind, updated_at);
