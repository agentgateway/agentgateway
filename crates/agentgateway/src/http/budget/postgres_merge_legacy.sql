-- TODO: temporary migration from key-hash budget ids. Adds the usage of a legacy row to the current
-- row, but only when it belongs to the current window and unit, with the same semantics as the
-- regular upsert.
INSERT INTO budget_usage (budget_id, window_start, window_end, unit, used_amount, updated_at)
SELECT $1, window_start, window_end, unit, used_amount, updated_at
FROM budget_usage
WHERE budget_id = $2 AND window_start = $3 AND window_end = $4 AND unit = $5
ON CONFLICT(budget_id) DO UPDATE SET
    window_start = excluded.window_start,
    window_end = excluded.window_end,
    unit = excluded.unit,
    used_amount = CASE
        WHEN excluded.window_start = budget_usage.window_start
          AND excluded.window_end = budget_usage.window_end
          AND excluded.unit = budget_usage.unit
        THEN budget_usage.used_amount + excluded.used_amount
        ELSE excluded.used_amount
    END,
    updated_at = GREATEST(budget_usage.updated_at, excluded.updated_at)
WHERE excluded.window_end >= budget_usage.window_end
   OR excluded.window_end - excluded.window_start <> budget_usage.window_end - budget_usage.window_start;
