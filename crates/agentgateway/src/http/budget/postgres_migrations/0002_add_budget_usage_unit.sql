-- Adds the unit column to tables created before it existed.
-- `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` takes an ACCESS EXCLUSIVE lock even when the column
-- exists, which would block running proxies' budget traffic on the first upgrade of a database
-- that already has it, so check the catalog first and only alter when the column is missing.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_attribute
        WHERE attrelid = 'budget_usage'::regclass AND attname = 'unit' AND NOT attisdropped
    ) THEN
        -- IF NOT EXISTS because releases without migrations add the column without this lock, so
        -- one of them may add it between the check above and this statement.
        ALTER TABLE budget_usage ADD COLUMN IF NOT EXISTS unit TEXT;
    END IF;
END
$$;
