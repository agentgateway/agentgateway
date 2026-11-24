CREATE TABLE agw_config_resources_with_uid (
	uid TEXT NOT NULL UNIQUE DEFAULT (
		lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2)
			|| '-' || substr('89ab', (random() & 3) + 1, 1) || substr(hex(randomblob(2)), 2)
			|| '-' || hex(randomblob(6)))
	),
	kind TEXT NOT NULL,
	id TEXT NOT NULL,
	value_json TEXT NOT NULL CHECK (json_valid(value_json)),
	revision INTEGER NOT NULL DEFAULT 1,
	created_at TEXT NOT NULL,
	updated_at TEXT NOT NULL,
	deleted_at TEXT,
	PRIMARY KEY (kind, id)
);

INSERT INTO agw_config_resources_with_uid
	(kind, id, value_json, revision, created_at, updated_at, deleted_at)
SELECT kind, id, value_json, revision, created_at, updated_at, deleted_at
FROM agw_config_resources;

DROP TABLE agw_config_resources;
ALTER TABLE agw_config_resources_with_uid RENAME TO agw_config_resources;
CREATE INDEX idx_agw_config_resources_kind_updated ON agw_config_resources(kind, updated_at);
