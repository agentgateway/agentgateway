CREATE TABLE IF NOT EXISTS agw_config_resources (
	kind TEXT NOT NULL,
	id TEXT NOT NULL,
	value_json TEXT NOT NULL CHECK (json_valid(value_json)),
	revision INTEGER NOT NULL DEFAULT 1,
	created_at TEXT NOT NULL,
	updated_at TEXT NOT NULL,
	deleted_at TEXT,
	PRIMARY KEY (kind, id)
);

CREATE INDEX IF NOT EXISTS idx_agw_config_resources_kind_updated
	ON agw_config_resources(kind, updated_at);

CREATE TABLE IF NOT EXISTS agw_config_meta (
	id INTEGER PRIMARY KEY CHECK (id = 1),
	generation INTEGER NOT NULL
);

INSERT OR IGNORE INTO agw_config_meta (id, generation) VALUES (1, 1);
