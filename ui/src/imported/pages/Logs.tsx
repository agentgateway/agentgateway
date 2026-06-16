import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp, ChevronDown, Download, RefreshCw } from "lucide-react";
import { getLog, searchLogs, streamLogs, tokenUsage } from "../api/logsApi";
import { useGatewayConfig } from "../_adapters/hooks";
import { Dropdown, Field, FieldGroup, JsonBlock, PageHeader, Panel, StatusBanner, Tooltip, formatDate, formatNumber, formatRelativeTime } from "../components/Primitives";
import { llmModelOptions } from "../llmModelOptions";
import type { LogEntry, SearchLogsResponse, TokenUsageGroup } from "../types";

export function LogsPage() {
  const config = useGatewayConfig();
  const models = useMemo(() => llmModelOptions(config.data?.llm), [config.data]);
  const [model, setModel] = useState("");
  const [status, setStatus] = useState("");
  const [stream, setStream] = useState(false);
  const [response, setResponse] = useState<SearchLogsResponse>({ logs: [] });
  const [usage, setUsage] = useState<TokenUsageGroup[]>([]);
  const [tokenSpendLogs, setTokenSpendLogs] = useState<LogEntry[]>([]);
  const [tokenSpendTotal, setTokenSpendTotal] = useState(0);
  const [requestTotal, setRequestTotal] = useState(0);
  const [expanded, setExpanded] = useState<LogEntry | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const filters = useMemo(() => ({
    requestModel: model ? [model] : [],
    httpStatus: status ? [Number(status)] : [],
  }), [model, status]);

  async function load() {
    setError(null);
    try {
      const timeRange = last24Hours();
      const [logs, analytics, spendLogs, spendAnalytics] = await Promise.all([
        searchLogs({ limit: 100, filters, includeAttributes: true }),
        tokenUsage({ groupBy: [{ field: "requestModel" }] }).catch(() => ({ groups: [] })),
        searchLogs({ limit: 1000, filters, timeRange }).catch(() => ({ logs: [] })),
        tokenUsage({ filters, timeRange }).catch(() => ({ groups: [] })),
      ]);
      setResponse(logs);
      setUsage(analytics.groups);
      setTokenSpendLogs(spendLogs.logs);
      setTokenSpendTotal(spendAnalytics.groups.reduce((sum, item) => sum + item.totalTokens, 0));
      setRequestTotal(spendAnalytics.groups.reduce((sum, item) => sum + item.requests, 0));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load logs");
    }
  }

  useEffect(() => {
    void load();
  }, [filters]);

  useEffect(() => {
    abortRef.current?.abort();
    if (!stream) return;
    const controller = new AbortController();
    abortRef.current = controller;
    void (async () => {
      try {
        for await (const event of streamLogs({ limit: 100, filters }, controller.signal)) {
          setResponse((current) => ({ ...current, logs: [event.entry, ...current.logs].slice(0, 200) }));
        }
      } catch (err) {
        if (!controller.signal.aborted) setError(err instanceof Error ? err.message : "Log stream failed");
      }
    })();
    return () => controller.abort();
  }, [stream, filters]);

  async function expand(entry: LogEntry) {
    if (expandedId === entry.id) {
      setExpandedId(null);
      setExpanded(null);
      return;
    }
    setExpandedId(entry.id);
    setExpanded(entry);
    try {
      const detail = await getLog(entry.id);
      setExpanded(detail.log ?? entry);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load log detail");
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Monitoring"
        description="Inspect one row per LLM call, stream new records, and expand full payload details."
        actions={<button className="button" type="button" onClick={load}><RefreshCw size={16} />Refresh</button>}
      />
      {error ? <StatusBanner state="bad" title="Logs API error">{error}</StatusBanner> : null}
      <Panel>
        <div className="filter-bar">
          <FieldGroup label="Model">
            <Dropdown
              ariaLabel="Model"
              value={model}
              searchable
              options={[
                { value: "", label: "All models" },
                ...models.map((item) => ({
                  value: item.name,
                  label: item.label,
                  icon: item.icon,
                  searchText: item.searchText,
                })),
              ]}
              onChange={setModel}
            />
          </FieldGroup>
          <Field label="HTTP status">
            <input value={status} onChange={(event) => setStatus(event.target.value)} placeholder="500" />
          </Field>
          <label className="toggle-row">
            <input type="checkbox" checked={stream} onChange={(event) => setStream(event.target.checked)} />
            Stream
          </label>
        </div>
      </Panel>

      <div className="stat-grid compact">
        <UsageStat title="Requests" value={usage.reduce((sum, item) => sum + item.requests, 0)} />
        <UsageStat title="Input tokens" value={usage.reduce((sum, item) => sum + item.inputTokens, 0)} />
        <UsageStat title="Output tokens" value={usage.reduce((sum, item) => sum + item.outputTokens, 0)} />
        <UsageStat title="Total tokens" value={usage.reduce((sum, item) => sum + item.totalTokens, 0)} />
      </div>

      <div className="logs-analytics-row">
        <Panel className="log-analytics-card">
          <TokenSpendChart logs={tokenSpendLogs} total={tokenSpendTotal} />
        </Panel>
        <Panel className="log-analytics-card">
          <RequestHealthChart logs={tokenSpendLogs} total={requestTotal} />
        </Panel>
      </div>

      <Panel>
        <div className="table-wrap">
          <table className="logs-table">
            <thead>
              <tr>
                <th>Completed</th>
                <th>Status</th>
                <th>Provider</th>
                <th>Model</th>
                <th>Duration</th>
                <th>Tokens</th>
                <th>Trace</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {response.logs.map((entry) => {
                const isExpanded = expandedId === entry.id;
                const detail = isExpanded ? (expanded ?? entry) : entry;
                return (
                  <Fragment key={entry.id}>
                    <tr key={entry.id}>
                      <td>
                        <div className="date-cell">
                          <span>{formatDate(entry.completedAt)}</span>
                          <small>{formatRelativeTime(entry.completedAt)}</small>
                        </div>
                      </td>
                      <td><span className={entry.error || (entry.httpStatus ?? 0) >= 400 ? "badge bad" : "badge ok"}>{entry.httpStatus ?? "n/a"}</span></td>
                      <td>{entry.genAi.providerName ?? "n/a"}</td>
                      <td>{entry.genAi.requestModel ?? "n/a"}</td>
                      <td>{entry.durationMs} ms</td>
                      <td><TokenSummary entry={entry} /></td>
                      <td className="mono">{entry.traceId?.slice(0, 12) ?? "n/a"}</td>
                      <td className="row-actions">
                        <Tooltip content={isExpanded ? "Collapse log" : "Expand log"}>
                          <button
                            className={isExpanded ? "icon-button expanded" : "icon-button"}
                            type="button"
                            aria-label={isExpanded ? "Collapse log" : "Expand log"}
                            onClick={() => void expand(entry)}
                          >
                            <ChevronDown size={16} />
                          </button>
                        </Tooltip>
                      </td>
                    </tr>
                    {isExpanded ? (
                      <tr key={`${entry.id}-detail`} className="expanded-row">
                        <td colSpan={8}>
                          <div className="expanded-log">
                            <div className="editor-title">
                              <div>
                                <h3>Log detail</h3>
                                <p className="mono">{detail.id}</p>
                              </div>
                              <button className="button" type="button" onClick={() => downloadJson(detail)}>
                                <Download size={16} />
                                JSON
                              </button>
                            </div>
                            <JsonBlock value={detail} />
                          </div>
                        </td>
                      </tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      </Panel>
    </div>
  );
}

function UsageStat(props: { title: string; value: number }) {
  return (
    <div className="stat">
      <span>{props.title}</span>
      <strong>{formatNumber(props.value)}</strong>
    </div>
  );
}

function TokenSpendChart(props: { logs: LogEntry[]; total: number }) {
  const buckets = hourlyTokenBuckets(props.logs);
  const max = Math.max(...buckets.map((bucket) => bucket.total), 1);
  return (
    <div className="token-spend-row">
      <div className="token-spend-copy">
        <span>24h token spend</span>
        <strong>{formatNumber(props.total)}</strong>
      </div>
      <div className="token-spend-chart" aria-label="Token spend over the past 24 hours">
        {buckets.map((bucket) => (
          <span className="token-bar-wrap" key={bucket.start}>
            <span
              className={bucket.total > 0 ? "token-bar" : "token-bar empty"}
              style={{ height: `${Math.max(4, Math.round((bucket.total / max) * 42))}px` }}
            />
            <span className="token-bar-tooltip">
              {bucket.label}
              {"\n"}
              tokens: {formatNumber(bucket.total)}
            </span>
          </span>
        ))}
      </div>
    </div>
  );
}

function RequestHealthChart(props: { logs: LogEntry[]; total: number }) {
  const buckets = hourlyRequestBuckets(props.logs);
  const max = Math.max(...buckets.map((bucket) => bucket.total), 1);
  const errors = buckets.reduce((sum, bucket) => sum + bucket.errors, 0);
  const errorRate = props.total > 0 ? `${Math.round((errors / props.total) * 100)}%` : "0%";
  return (
    <div className="token-spend-row">
      <div className="token-spend-copy">
        <span>24h request health</span>
        <strong>{formatNumber(props.total)}</strong>
      </div>
      <div className="analytics-subtle">{formatNumber(errors)} errors · {errorRate}</div>
      <div className="token-spend-chart" aria-label="Request volume over the past 24 hours">
        {buckets.map((bucket) => {
          const height = Math.max(4, Math.round((bucket.total / max) * 42));
          const successful = Math.max(0, bucket.total - bucket.errors);
          const errorHeight = bucket.total > 0 ? Math.max(bucket.errors > 0 ? 3 : 0, Math.round((bucket.errors / bucket.total) * height)) : 0;
          const successHeight = bucket.total > 0 ? Math.max(successful > 0 ? 3 : 0, height - errorHeight) : 0;
          return (
            <span className="token-bar-wrap" key={bucket.start}>
              {bucket.total === 0 ? (
                <span className="token-bar empty" style={{ height: "4px" }} />
              ) : (
                <span className="request-health-bar" style={{ height: `${height}px` }}>
                  {bucket.errors > 0 ? <span className="request-health-segment error" style={{ height: `${errorHeight}px` }} /> : null}
                  {successful > 0 ? <span className="request-health-segment ok" style={{ height: `${successHeight}px` }} /> : null}
                </span>
              )}
              <span className="token-bar-tooltip">
                {bucket.label}
                {"\n"}
                requests: {formatNumber(bucket.total)}
                {"\n"}
                errors: {formatNumber(bucket.errors)}
              </span>
            </span>
          );
        })}
      </div>
    </div>
  );
}

function hourlyTokenBuckets(logs: LogEntry[]) {
  const now = Date.now();
  const hourMs = 60 * 60 * 1000;
  const start = now - 24 * hourMs;
  const buckets = Array.from({ length: 24 }, (_, index) => {
    const bucketStart = start + index * hourMs;
    return {
      start: String(bucketStart),
      total: 0,
      label: new Intl.DateTimeFormat(undefined, { hour: "numeric" }).format(new Date(bucketStart)),
    };
  });

  for (const entry of logs) {
    const completed = new Date(entry.completedAt).getTime();
    if (!Number.isFinite(completed) || completed < start || completed > now) continue;
    const index = Math.min(23, Math.max(0, Math.floor((completed - start) / hourMs)));
    buckets[index].total += entry.usage.totalTokens ?? (entry.usage.inputTokens ?? 0) + (entry.usage.outputTokens ?? 0);
  }
  return buckets;
}

function hourlyRequestBuckets(logs: LogEntry[]) {
  const now = Date.now();
  const hourMs = 60 * 60 * 1000;
  const start = now - 24 * hourMs;
  const buckets = Array.from({ length: 24 }, (_, index) => {
    const bucketStart = start + index * hourMs;
    return {
      start: String(bucketStart),
      total: 0,
      errors: 0,
      label: new Intl.DateTimeFormat(undefined, { hour: "numeric" }).format(new Date(bucketStart)),
    };
  });

  for (const entry of logs) {
    const completed = new Date(entry.completedAt).getTime();
    if (!Number.isFinite(completed) || completed < start || completed > now) continue;
    const index = Math.min(23, Math.max(0, Math.floor((completed - start) / hourMs)));
    buckets[index].total += 1;
    if (entry.error || (entry.httpStatus ?? 0) >= 400) buckets[index].errors += 1;
  }
  return buckets;
}

function last24Hours() {
  const now = new Date();
  return {
    from: new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString(),
    to: now.toISOString(),
  };
}

function downloadJson(value: unknown) {
  const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "agentgateway-log.json";
  anchor.click();
  URL.revokeObjectURL(url);
}

function TokenSummary(props: { entry: LogEntry }) {
  const input = props.entry.usage.inputTokens;
  const output = props.entry.usage.outputTokens;
  const cache = attributeNumber(props.entry.attributes, ["cacheTokens", "cachedTokens", "cache_tokens", "cached_tokens"]);
  const total = props.entry.usage.totalTokens;
  const detail = [
    `in: ${formatNumber(input)}`,
    `out: ${formatNumber(output)}`,
    `cache: ${formatNumber(cache)}`,
    `total: ${formatNumber(total)}`,
  ].join("\n");

  return (
    <span className="token-summary">
      <span><ArrowDown size={14} />{formatNumber(input)}</span>
      <span><ArrowUp size={14} />{formatNumber(output)}</span>
      <span className="token-tooltip" aria-hidden="true">{detail}</span>
    </span>
  );
}

function attributeNumber(value: unknown, keys: string[]) {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  for (const key of keys) {
    const direct = record[key];
    if (typeof direct === "number") return direct;
    if (typeof direct === "string" && Number.isFinite(Number(direct))) return Number(direct);
  }
  return undefined;
}
