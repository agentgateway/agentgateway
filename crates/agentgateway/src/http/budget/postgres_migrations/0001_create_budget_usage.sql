CREATE TABLE IF NOT EXISTS budget_usage (
    budget_id TEXT PRIMARY KEY,
    window_start BIGINT NOT NULL,
    window_end BIGINT NOT NULL,
    unit TEXT NOT NULL,
    used_amount BIGINT NOT NULL DEFAULT 0 CHECK (used_amount >= 0),
    updated_at BIGINT NOT NULL
);
