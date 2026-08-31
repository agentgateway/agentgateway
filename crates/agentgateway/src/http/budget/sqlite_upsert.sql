INSERT INTO budget_usage (budget_id, window_start, window_end, unit, used_amount, updated_at)
VALUES (?, ?, ?, ?, ?, ?)
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
    updated_at = MAX(budget_usage.updated_at, excluded.updated_at)
WHERE excluded.window_end >= budget_usage.window_end;
