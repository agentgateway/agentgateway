CREATE TABLE IF NOT EXISTS budget_usage (
    budget_id TEXT PRIMARY KEY,
    window_start INTEGER NOT NULL,
    window_end INTEGER NOT NULL,
    unit TEXT NOT NULL,
    used_amount INTEGER NOT NULL DEFAULT 0 CHECK (used_amount >= 0),
    updated_at INTEGER NOT NULL
);
