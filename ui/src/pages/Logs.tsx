import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Bot,
  Braces,
  Check,
  ChevronDown,
  ChevronRight,
  Clock3,
  Copy,
  Download,
  RefreshCw,
  Route,
  Save,
  Settings,
  User,
} from "lucide-react";
import {
  analyticsSummary,
  getLog,
  searchLogs,
  streamLogs,
} from "../api/logsApi";
import {
  DateRangePicker,
  bucketCountForRange,
  bucketSecondsForRange,
  isPresetRange,
  logTimeRangeLabel,
  logTimeRangeToApi,
  toDateTimeLocal,
  type LogTimeRange,
} from "../components/DateRangePicker";
import { EnumSelector } from "../components/EnumSelector";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { MultiCheckboxDropdown } from "../components/MultiCheckboxDropdown";
import {
  promptCompletionLoggingEnabled,
  setPromptCompletionLogging,
  setUiLogAttributeExpressions,
  uiLogAttributeExpressions,
} from "../config";
import { queryParam, useStickyQueryParam } from "../drawerRouteState";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import {
  Drawer,
  EmptyState,
  FieldGroup,
  JsonBlock,
  PageHeader,
  Panel,
  StatusBanner,
  formatDate,
  formatNumber,
  formatRelativeTime,
  useDismissiblePopover,
} from "../components/Primitives";
import { llmModelOptions } from "../llmModelOptions";
import {
  ANALYTICS_DIMENSIONS,
  AnalyticsBreakdownChart,
  AnalyticsTimelineChart,
  analyticsBreakdownData,
  analyticsFilterOptions,
  analyticsFilterOptionsFromResponse,
  analyticsFiltersKey,
  analyticsGroupBy,
  analyticsLogFilters,
  analyticsTimelineData,
  emptyAnalyticsFilterOptions,
  isAnalyticsDimension,
  mergeAnalyticsFilterOptions,
  type AnalyticsDimension,
  type AnalyticsMeasure,
} from "./analytics/AnalyticsCharts";
import type {
  AnalyticsGroup,
  AnalyticsTimeBucket,
  LogEntry,
  SearchLogsResponse,
  TimeRange,
} from "../types";

export function LogsPage() {
  const navigate = useNavigate({ from: "/llm/logs" });
  const config = useGatewayConfig();
  const updateConfig = useUpdateConfig();
  const models = useMemo(
    () => llmModelOptions(config.data?.llm),
    [config.data],
  );
  const promptLoggingEnabled = promptCompletionLoggingEnabled(config.data);
  const [settings, setSettings] = useStickyQueryParam("settings");
  const [linkedLogId, setLinkedLogId] = useState(() => queryParam("log"));
  const [logFilters, setLogFilters] = useState<
    Record<AnalyticsDimension, string[]>
  >(emptyAnalyticsFilterOptions);
  const [filterOptionMap, setFilterOptionMap] = useState<
    Record<AnalyticsDimension, string[]>
  >(emptyAnalyticsFilterOptions);
  const [status, setStatus] = useState("");
  const [stream, setStream] = useState(false);
  const [response, setResponse] = useState<SearchLogsResponse>({ logs: [] });
  const [expanded, setExpanded] = useState<LogEntry | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [expandedLoading, setExpandedLoading] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const loadSeqRef = useRef(0);
  const detailSeqRef = useRef(0);
  const detailLoadingTimerRef = useRef<number | null>(null);
  const filterOptionsSeqRef = useRef(0);
  const logFiltersKey = analyticsFiltersKey(logFilters);
  const filters = useMemo(
    () => ({
      ...analyticsLogFilters(logFilters),
      httpStatus: status ? [Number(status)] : [],
    }),
    [logFiltersKey, status],
  );
  const visibleLogs = useMemo(() => {
    if (
      !expanded ||
      !expandedId ||
      response.logs.some((entry) => entry.id === expandedId)
    )
      return response.logs;
    if (!isLlmLogEntry(expanded)) return response.logs;
    return [expanded, ...response.logs];
  }, [expanded, expandedId, response.logs]);

  async function load() {
    const loadSeq = loadSeqRef.current + 1;
    loadSeqRef.current = loadSeq;
    setError(null);
    setLoading(true);
    try {
      const logs = await searchLogs({
        limit: 100,
        filters,
        includeAttributes: true,
      });
      if (loadSeq !== loadSeqRef.current) return;
      setResponse(llmLogsResponse(logs));
    } catch (err) {
      if (loadSeq !== loadSeqRef.current) return;
      setError(err instanceof Error ? err.message : "Failed to load logs");
    } finally {
      if (loadSeq === loadSeqRef.current) setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, [filters]);

  useEffect(() => {
    const loadSeq = filterOptionsSeqRef.current + 1;
    filterOptionsSeqRef.current = loadSeq;
    void (async () => {
      try {
        const analytics = await analyticsSummary({
          filters: analyticsLogFilters(logFilters),
          groupBy: [],
          bucketCount: 1,
        });
        if (loadSeq !== filterOptionsSeqRef.current) return;
        const discoveredOptions =
          analyticsFilterOptionsFromResponse(analytics.filterOptions) ??
          analyticsFilterOptions(analytics.groups);
        setFilterOptionMap((current) =>
          mergeAnalyticsFilterOptions(current, discoveredOptions, logFilters),
        );
      } catch {
        if (loadSeq !== filterOptionsSeqRef.current) return;
        setFilterOptionMap((current) =>
          mergeAnalyticsFilterOptions(
            current,
            {
              ...emptyAnalyticsFilterOptions(),
              model: models.map((item) => item.name),
            },
            logFilters,
          ),
        );
      }
    })();
  }, [logFiltersKey, models]);

  useEffect(() => {
    abortRef.current?.abort();
    if (!stream) return;
    const controller = new AbortController();
    abortRef.current = controller;
    void (async () => {
      try {
        for await (const event of streamLogs(
          { limit: 100, filters },
          controller.signal,
        )) {
          if (!isLlmLogEntry(event.entry)) continue;
          setResponse((current) => ({
            ...current,
            logs: [event.entry, ...current.logs].slice(0, 200),
          }));
        }
      } catch (err) {
        if (!controller.signal.aborted)
          setError(err instanceof Error ? err.message : "Log stream failed");
      }
    })();
    return () => controller.abort();
  }, [stream, filters]);

  useEffect(() => {
    function syncLinkedLogId() {
      setLinkedLogId(queryParam("log"));
    }
    window.addEventListener("popstate", syncLinkedLogId);
    return () => window.removeEventListener("popstate", syncLinkedLogId);
  }, []);

  function setLinkedLogQuery(next: string | null) {
    setLinkedLogId(next);
    void navigate({
      to: "/llm/logs",
      replace: true,
      resetScroll: false,
      search: (previous) => {
        const search = { ...(previous as Record<string, unknown>) };
        if (next) search.log = next;
        else delete search.log;
        return search;
      },
    });
  }

  useEffect(() => {
    if (!linkedLogId) {
      setExpandedId(null);
      setExpanded(null);
      stopDeferredExpandedLoading();
      return;
    }
    if (linkedLogId === expandedId) return;
    const fallback = response.logs.find((entry) => entry.id === linkedLogId);
    void loadExpandedLog(linkedLogId, fallback);
  }, [linkedLogId]);

  async function loadExpandedLog(logId: string, fallback?: LogEntry) {
    const detailSeq = detailSeqRef.current + 1;
    detailSeqRef.current = detailSeq;
    setExpandedId(logId);
    setExpanded(fallback ?? null);
    startDeferredExpandedLoading();
    try {
      const detail = await getLog(logId);
      if (detailSeq !== detailSeqRef.current) return;
      const nextLog = detail.log ?? fallback ?? null;
      setExpanded(nextLog && isLlmLogEntry(nextLog) ? nextLog : null);
    } catch (err) {
      if (detailSeq !== detailSeqRef.current) return;
      setError(
        err instanceof Error ? err.message : "Failed to load log detail",
      );
    } finally {
      if (detailSeq === detailSeqRef.current) stopDeferredExpandedLoading();
    }
  }

  async function expand(entry: LogEntry) {
    if (expandedId === entry.id) {
      setLinkedLogQuery(null);
      setExpandedId(null);
      setExpanded(null);
      stopDeferredExpandedLoading();
      return;
    }
    setLinkedLogQuery(entry.id);
    await loadExpandedLog(entry.id, entry);
  }

  function startDeferredExpandedLoading() {
    stopDeferredExpandedLoading();
    detailLoadingTimerRef.current = window.setTimeout(() => {
      detailLoadingTimerRef.current = null;
      setExpandedLoading(true);
    }, 180);
  }

  function stopDeferredExpandedLoading() {
    if (detailLoadingTimerRef.current != null) {
      window.clearTimeout(detailLoadingTimerRef.current);
      detailLoadingTimerRef.current = null;
    }
    setExpandedLoading(false);
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Logs"
        description="Inspect recent LLM calls and request/response payloads."
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() => setSettings("logs")}
            >
              <Settings size={16} />
              Settings
            </button>
            <button
              className="button"
              type="button"
              disabled={loading}
              onClick={load}
            >
              <RefreshCw size={16} />
              Refresh
            </button>
          </div>
        }
      />
      {error ? (
        <StatusBanner state="bad" title="Logs API error">
          {error}
        </StatusBanner>
      ) : null}
      {updateConfig.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {updateConfig.error.message}
        </StatusBanner>
      ) : null}
      <Panel>
        <div className="logs-filter-bar">
          {ANALYTICS_DIMENSIONS.map((meta) => {
            const dimension = meta.value;
            return (
              <MultiCheckboxDropdown
                kind="filter"
                key={dimension}
                label={meta.filterLabel}
                options={filterOptionMap[dimension].map((value) => ({
                  value,
                  label: value,
                }))}
                values={logFilters[dimension]}
                placeholder={`All ${meta.filterLabel.toLowerCase()}`}
                allLabel={`All ${meta.filterLabel.toLowerCase()}`}
                onChange={(values) =>
                  setLogFilters((current) => ({
                    ...current,
                    [dimension]: values,
                  }))
                }
              />
            );
          })}
          <MultiCheckboxDropdown
            kind="filter"
            label="HTTP status"
            options={[
              { value: "200", label: "200 OK" },
              { value: "400", label: "400 Bad request" },
              { value: "401", label: "401 Unauthorized" },
              { value: "403", label: "403 Forbidden" },
              { value: "404", label: "404 Not found" },
              { value: "429", label: "429 Rate limited" },
              { value: "500", label: "500 Server error" },
            ]}
            values={status ? [status] : []}
            placeholder="Any status"
            allLabel="Any status"
            onChange={(values) => setStatus(values.at(-1) ?? "")}
          />
          <label className="toggle-row logs-stream-toggle">
            <input
              type="checkbox"
              checked={stream}
              onChange={(event) => setStream(event.target.checked)}
            />
            Stream
          </label>
          {hasAnalyticsFilters(logFilters) || status ? (
            <button
              className="button"
              type="button"
              onClick={() => {
                setLogFilters(emptyAnalyticsFilterOptions());
                setStatus("");
              }}
            >
              Clear filters
            </button>
          ) : null}
        </div>
      </Panel>

      <Panel className="logs-results-panel">
        <div className="logs-section-header">
          <div>
            <h3>Recent calls</h3>
            <p>
              {loading
                ? "Refreshing..."
                : `${formatNumber(visibleLogs.length)} rows`}
            </p>
          </div>
          {stream ? <span className="badge ok">streaming</span> : null}
        </div>
        <div className="log-call-list">
          {visibleLogs.length === 0 ? (
            <EmptyState
              title={loading ? "Loading logs" : "No log entries"}
              description={
                loading
                  ? "Fetching recent LLM calls."
                  : "No LLM calls match the current filters."
              }
            />
          ) : null}
          {visibleLogs.map((entry) => (
            <LogCallRow
              key={entry.id}
              entry={entry}
              detail={expandedId === entry.id ? (expanded ?? entry) : entry}
              expanded={expandedId === entry.id}
              loading={expandedId === entry.id && expandedLoading}
              onToggle={() => void expand(entry)}
              onOpenSettings={() => setSettings("logs")}
            />
          ))}
        </div>
      </Panel>

      {settings === "logs" ? (
        <LogsSettingsDrawer
          enabled={promptLoggingEnabled}
          attributes={uiLogAttributeExpressions(config.data)}
          saving={updateConfig.isPending}
          saveError={updateConfig.isError ? updateConfig.error.message : null}
          onClose={() => setSettings(null, "replace")}
          onSave={(values) =>
            updateConfig.mutate(
              (next) => {
                setPromptCompletionLogging(next, values.enabled);
                setUiLogAttributeExpressions(next, values.attributes);
              },
              {
                onSuccess: () => setSettings(null, "replace"),
              },
            )
          }
        />
      ) : null}
    </div>
  );
}

function LogsSettingsDrawer(props: {
  enabled: boolean;
  attributes: { user: string; group: string };
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onSave: (values: {
    enabled: boolean;
    attributes: { user: string; group: string };
  }) => void;
}) {
  const [enabled, setEnabled] = useState(props.enabled);
  const [userExpression, setUserExpression] = useState(props.attributes.user);
  const [groupExpression, setGroupExpression] = useState(
    props.attributes.group,
  );
  useEffect(() => setEnabled(props.enabled), [props.enabled]);
  useEffect(() => {
    setUserExpression(props.attributes.user);
    setGroupExpression(props.attributes.group);
  }, [props.attributes.group, props.attributes.user]);
  return (
    <Drawer
      title="Log settings"
      onClose={props.onClose}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={props.saving}
            onClick={() =>
              props.onSave({
                enabled,
                attributes: { user: userExpression, group: groupExpression },
              })
            }
          >
            <Save size={16} />
            Save settings
          </button>
        </div>
      }
    >
      <label className="config-option-row">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(event) => setEnabled(event.target.checked)}
        />
        <span>
          <strong>Include prompts and completions in logs</strong>
          <small>
            Adds `gen_ai.prompt` and `gen_ai.completion` attributes to access
            logs.
          </small>
        </span>
      </label>
      <section className="policy-form-section log-attribute-settings">
        <div className="policy-form-section-header">
          <span className="policy-form-section-icon compact">
            <User size={16} />
          </span>
          <div>
            <h4>Request log identity</h4>
            <p>
              Optional CEL expressions for populating user and group attributes
              in database logs. If not set a default will be used.
            </p>
          </div>
        </div>
        <div className="policy-form-section-body">
          <FieldGroup
            label="User attribute"
            tooltip="CEL expression used to populate the agentgateway.user request log attribute."
          >
            <MiniMonacoEditor
              className="micro"
              language="cel"
              value={userExpression}
              onChange={setUserExpression}
              placeholder="apiKey.user"
            />
          </FieldGroup>
          <FieldGroup
            label="Group attribute"
            tooltip="CEL expression used to populate the agentgateway.group request log attribute."
          >
            <MiniMonacoEditor
              className="micro"
              language="cel"
              value={groupExpression}
              onChange={setGroupExpression}
              placeholder="apiKey.group"
            />
          </FieldGroup>
        </div>
      </section>
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function isLlmLogEntry(entry: LogEntry | null | undefined) {
  return Boolean(entry?.genAi?.providerName);
}

function llmLogsResponse(response: SearchLogsResponse): SearchLogsResponse {
  return {
    ...response,
    logs: response.logs.filter(isLlmLogEntry),
  };
}

function hasAnalyticsFilters(filters: Record<AnalyticsDimension, string[]>) {
  return ANALYTICS_DIMENSIONS.some((item) => filters[item.value].length > 0);
}

type AnalyticsUrlState = {
  timeRange: LogTimeRange;
  groupBy: AnalyticsDimension[];
  filters: Record<AnalyticsDimension, string[]>;
  metric: AnalyticsMetric;
};

type AnalyticsMetric = AnalyticsMeasure;

function readAnalyticsUrlState(): AnalyticsUrlState {
  const params = new URLSearchParams(window.location.search);
  const groupBy = uniqueAnalyticsDimensions(
    (params.get("groupBy") ?? "")
      .split(",")
      .filter((value): value is AnalyticsDimension =>
        isAnalyticsDimension(value),
      ),
  );
  return {
    timeRange: readAnalyticsTimeRange(params),
    groupBy,
    filters: {
      model: readRepeatedParam(params, "model"),
      user: readRepeatedParam(params, "user"),
      group: readRepeatedParam(params, "group"),
      provider: readRepeatedParam(params, "provider"),
      userAgent: readRepeatedParam(params, "userAgent"),
    },
    metric: readAnalyticsMetric(params),
  };
}

function writeAnalyticsUrlState(state: AnalyticsUrlState) {
  const params = new URLSearchParams();
  if (state.timeRange.mode === "preset") {
    if (state.timeRange.preset !== "24h")
      params.set("range", state.timeRange.preset);
  } else {
    const from = new Date(state.timeRange.fromLocal);
    const to = new Date(state.timeRange.toLocal);
    if (Number.isFinite(from.getTime()) && Number.isFinite(to.getTime())) {
      params.set("from", from.toISOString());
      params.set("to", to.toISOString());
    }
  }
  if (state.groupBy.length > 0) params.set("groupBy", state.groupBy.join(","));
  if (state.metric !== "tokens") params.set("metric", state.metric);
  for (const dimension of ANALYTICS_DIMENSIONS.map((item) => item.value)) {
    for (const value of state.filters[dimension]) {
      params.append(dimension, value);
    }
  }
  const nextSearch = params.toString();
  const nextUrl = `${window.location.pathname}${nextSearch ? `?${nextSearch}` : ""}${window.location.hash}`;
  if (
    nextUrl !==
    `${window.location.pathname}${window.location.search}${window.location.hash}`
  ) {
    window.history.replaceState(null, "", nextUrl);
  }
}

function readAnalyticsMetric(params: URLSearchParams): AnalyticsMetric {
  const value = params.get("metric");
  return value === "cost" || value === "requests" || value === "tokens"
    ? value
    : "tokens";
}

function readAnalyticsTimeRange(params: URLSearchParams): LogTimeRange {
  const range = params.get("range");
  if (range && isPresetRange(range)) return { mode: "preset", preset: range };
  const from = params.get("from");
  const to = params.get("to");
  if (from && to) {
    const fromDate = new Date(from);
    const toDate = new Date(to);
    if (
      Number.isFinite(fromDate.getTime()) &&
      Number.isFinite(toDate.getTime()) &&
      fromDate < toDate
    ) {
      return {
        mode: "absolute",
        fromLocal: toDateTimeLocal(fromDate),
        toLocal: toDateTimeLocal(toDate),
      };
    }
  }
  return { mode: "preset", preset: "24h" };
}

function readRepeatedParam(params: URLSearchParams, key: AnalyticsDimension) {
  return sortedUnique(
    params
      .getAll(key)
      .flatMap((value) => value.split(","))
      .filter(Boolean),
  );
}

function uniqueAnalyticsDimensions(values: AnalyticsDimension[]) {
  return ANALYTICS_DIMENSIONS.map((item) => item.value).filter((dimension) =>
    values.includes(dimension),
  );
}

export function AnalyticsPage() {
  const initialUrlState = useMemo(readAnalyticsUrlState, []);
  const [timeRange, setTimeRange] = useState<LogTimeRange>(
    initialUrlState.timeRange,
  );
  const [groupBy, setGroupBy] = useState<AnalyticsDimension[]>(
    initialUrlState.groupBy,
  );
  const [filters, setFilters] = useState<Record<AnalyticsDimension, string[]>>(
    initialUrlState.filters,
  );
  const [metric, setMetric] = useState<AnalyticsMetric>(initialUrlState.metric);
  const [buckets, setBuckets] = useState<AnalyticsTimeBucket[]>([]);
  const [summaryRange, setSummaryRange] = useState<TimeRange | null>(null);
  const [bucketSeconds, setBucketSeconds] = useState<number | null>(null);
  const [usage, setUsage] = useState<AnalyticsGroup[]>([]);
  const [filterOptionMap, setFilterOptionMap] = useState<
    Record<AnalyticsDimension, string[]>
  >(emptyAnalyticsFilterOptions);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadSeqRef = useRef(0);
  const selectedGroupBy: AnalyticsDimension[] = groupBy;
  const groupByKey = selectedGroupBy.join(",");
  const filtersKey = analyticsFiltersKey(filters);
  const analyticsState = useMemo(
    () => ({
      timeRange,
      groupBy: selectedGroupBy,
      filters,
      metric,
    }),
    [timeRange, groupByKey, filtersKey, metric],
  );

  useEffect(() => {
    writeAnalyticsUrlState(analyticsState);
  }, [analyticsState]);

  useEffect(() => {
    function onPopState() {
      const next = readAnalyticsUrlState();
      setTimeRange(next.timeRange);
      setGroupBy(next.groupBy);
      setFilters(next.filters);
      setMetric(next.metric);
    }
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  async function load() {
    const loadSeq = loadSeqRef.current + 1;
    loadSeqRef.current = loadSeq;
    setError(null);
    setLoading(true);
    try {
      const apiTimeRange = logTimeRangeToApi(timeRange);
      const logFilters = analyticsLogFilters(filters);
      const analytics = await analyticsSummary({
        timeRange: apiTimeRange,
        filters: logFilters,
        groupBy: selectedGroupBy.map(analyticsGroupBy),
        bucketCount: bucketCountForRange(timeRange),
        bucketSeconds: bucketSecondsForRange(timeRange),
      });
      if (loadSeq !== loadSeqRef.current) return;
      setBuckets(analytics.buckets);
      setSummaryRange(analytics.timeRange);
      setBucketSeconds(analytics.bucketSeconds);
      setUsage(analytics.groups);
      const discoveredOptions =
        analyticsFilterOptionsFromResponse(analytics.filterOptions) ??
        analyticsFilterOptions(analytics.groups, selectedGroupBy);
      setFilterOptionMap((current) =>
        mergeAnalyticsFilterOptions(current, discoveredOptions, filters),
      );
    } catch (err) {
      if (loadSeq !== loadSeqRef.current) return;
      setError(err instanceof Error ? err.message : "Failed to load analytics");
    } finally {
      if (loadSeq === loadSeqRef.current) setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, [timeRange, groupByKey, filtersKey]);

  useEffect(() => {
    setFilterOptionMap(emptyAnalyticsFilterOptions());
  }, [timeRange, groupByKey]);

  function updateGroupBy(next: string[]) {
    const normalized = next.filter((value): value is AnalyticsDimension =>
      isAnalyticsDimension(value),
    );
    setGroupBy(normalized);
  }

  function updateFilter(dimension: AnalyticsDimension, values: string[]) {
    setFilters((current) => ({ ...current, [dimension]: values }));
  }

  const requestedRange = useMemo(
    () => logTimeRangeToApi(timeRange),
    [timeRange],
  );
  const effectiveBucketSeconds =
    bucketSeconds ?? bucketSecondsForRange(timeRange);
  const timeline = useMemo(
    () =>
      analyticsTimelineData(
        buckets,
        summaryRange ?? requestedRange,
        effectiveBucketSeconds,
        selectedGroupBy,
        metric,
      ),
    [
      buckets,
      summaryRange?.from,
      summaryRange?.to,
      requestedRange.from,
      requestedRange.to,
      effectiveBucketSeconds,
      selectedGroupBy,
      metric,
    ],
  );
  const chartData = useMemo(
    () => analyticsBreakdownData(usage, selectedGroupBy, metric),
    [usage, selectedGroupBy, metric],
  );
  const totalTokens = usage.reduce((sum, item) => sum + item.totalTokens, 0);
  const totalRequests = usage.reduce((sum, item) => sum + item.requests, 0);
  const totalCost = usage.reduce(
    (sum, item) => sum + analyticsRecordCost(item),
    0,
  );

  return (
    <div className="page-stack">
      <PageHeader
        title="Analytics"
        description="Analyze LLM traffic by model, user, and provider."
        actions={
          <div className="button-row">
            <AnalyticsExportDropdown
              buckets={buckets}
              groupBy={selectedGroupBy}
              disabled={loading || !buckets.length}
            />
            <DateRangePicker value={timeRange} onChange={setTimeRange} />
          </div>
        }
      />
      {error ? (
        <StatusBanner state="bad" title="Analytics API error">
          {error}
        </StatusBanner>
      ) : null}
      <Panel>
        <div className="analytics-controls">
          <div className="analytics-controls-left">
            <MultiCheckboxDropdown
              kind="group"
              label="Group by"
              options={ANALYTICS_DIMENSIONS.map((dimension) => ({
                value: dimension.value,
                label: dimension.label,
              }))}
              values={selectedGroupBy}
              placeholder="Total"
              allLabel="Total"
              onChange={updateGroupBy}
            />
            {ANALYTICS_DIMENSIONS.map((meta) => {
              const dimension = meta.value;
              return (
                <MultiCheckboxDropdown
                  kind="filter"
                  key={dimension}
                  label={meta.filterLabel}
                  options={filterOptionMap[dimension].map((value) => ({
                    value,
                    label: value,
                  }))}
                  values={filters[dimension]}
                  placeholder={`All ${meta.filterLabel.toLowerCase()}`}
                  allLabel={`All ${meta.filterLabel.toLowerCase()}`}
                  onChange={(values) => updateFilter(dimension, values)}
                />
              );
            })}
          </div>
          <div className="analytics-controls-right">
            <FieldGroup label="Measure">
              <EnumSelector
                ariaLabel="Measure"
                value={metric}
                options={[
                  { value: "tokens", label: "Tokens" },
                  { value: "cost", label: "Cost" },
                  { value: "requests", label: "Requests" },
                ]}
                onChange={setMetric}
              />
            </FieldGroup>
          </div>
        </div>
      </Panel>
      <Panel className="monitoring-activity-card">
        <div className="monitoring-card-header">
          <div>
            <h3>Traffic over time</h3>
            <p>
              {loading
                ? "Loading analytics..."
                : `${formatCost(totalCost)} / ${formatNumber(totalTokens)} tokens / ${formatNumber(totalRequests)} calls`}
            </p>
          </div>
          <span className="muted-copy">{logTimeRangeLabel(timeRange)}</span>
        </div>
        <AnalyticsTimelineChart
          data={timeline.data}
          measure={metric}
          series={timeline.series}
        />
      </Panel>
      <Panel className="monitoring-activity-card">
        <div className="monitoring-card-header">
          <div>
            <h3>Breakdown</h3>
            <p>
              {selectedGroupBy.length
                ? `${analyticsMetricLabel(metric)} by ${selectedGroupBy.map((dimension) => ANALYTICS_DIMENSIONS.find((item) => item.value === dimension)?.label.toLowerCase()).join(", ")}`
                : `${analyticsMetricLabel(metric)} total`}
            </p>
          </div>
        </div>
        <AnalyticsBreakdownChart data={chartData} measure={metric} />
      </Panel>
    </div>
  );
}

function sortedUnique(values: string[]) {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))].sort(
    (a, b) => a.localeCompare(b),
  );
}

function AnalyticsExportDropdown(props: {
  buckets: AnalyticsTimeBucket[];
  groupBy: AnalyticsDimension[];
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useDismissiblePopover<HTMLDivElement>(open, () => setOpen(false));
  return (
    <div className="export-dropdown" ref={ref}>
      <button
        className="button"
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={props.disabled}
        onClick={() => setOpen((current) => !current)}
      >
        <Download size={16} />
        Export
        <ChevronDown size={15} />
      </button>
      {open ? (
        <div className="export-dropdown-menu" role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              downloadAnalyticsCsv(props.buckets, props.groupBy);
              setOpen(false);
            }}
          >
            CSV
          </button>
        </div>
      ) : null}
    </div>
  );
}

function downloadAnalyticsCsv(
  buckets: AnalyticsTimeBucket[],
  groupBy: AnalyticsDimension[],
) {
  const headers = [
    "start",
    ...groupBy.map((dimension) => dimension),
    "requests",
    "total_tokens",
    "cost",
  ];
  const rows = buckets.map((bucket) => [
    bucket.start,
    ...groupBy.map((dimension) =>
      analyticsCsvGroupValue(bucket.group, dimension),
    ),
    bucket.requests,
    bucket.totalTokens,
    bucket.cost ?? "",
  ]);
  const csv = [headers, ...rows]
    .map((row) => row.map(csvCell).join(","))
    .join("\n");
  const blob = new Blob([`${csv}\n`], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `agentgateway-analytics-${new Date().toISOString().slice(0, 10)}.csv`;
  anchor.click();
  URL.revokeObjectURL(url);
}

function analyticsCsvGroupValue(
  group: Record<string, unknown>,
  dimension: AnalyticsDimension,
) {
  if (dimension === "model") return group.requestModel ?? group.model ?? "";
  if (dimension === "provider")
    return group.provider ?? group.providerName ?? "";
  if (dimension === "group")
    return group["agentgateway.group"] ?? group.group ?? "";
  if (dimension === "userAgent")
    return group["user_agent.name"] ?? group.userAgent ?? "";
  return group["agentgateway.user"] ?? group.user ?? "";
}

function csvCell(value: unknown) {
  const text = value == null ? "" : String(value);
  return /[",\n\r]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function analyticsMetricLabel(metric: AnalyticsMetric) {
  if (metric === "requests") return "Requests";
  if (metric === "cost") return "Cost";
  return "Tokens";
}

function formatCost(value: number) {
  if (value === 0) return "$0.00";
  if (value >= 10) return `$${value.toFixed(2)}`;
  if (value >= 0.01) return `$${value.toFixed(4)}`;
  // Sub-cent: show 2 significant figures, strip trailing zeros
  return `$${parseFloat(value.toPrecision(2))}`;
}

function analyticsRecordCost(value: unknown) {
  if (!value || typeof value !== "object") return 0;
  const record = value as Record<string, unknown>;
  for (const key of ["cost", "totalCost", "estimatedCost", "usdCost"]) {
    const number = Number(record[key]);
    if (Number.isFinite(number)) return number;
  }
  return 0;
}

type RenderedLogMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  name?: string;
  toolCalls?: Array<{ name: string; arguments?: unknown }>;
};

function originalModelForLog(entry: LogEntry) {
  const attributes =
    entry.attributes && typeof entry.attributes === "object"
      ? (entry.attributes as Record<string, unknown>)
      : {};
  const value = attributes["agw.ai.original_model"] ?? attributes.originalModel;
  return typeof value === "string" && value.trim() ? value : null;
}

function logIdentity(entry: LogEntry) {
  const attributes =
    entry.attributes && typeof entry.attributes === "object"
      ? (entry.attributes as Record<string, unknown>)
      : {};
  const record = entry as unknown as Record<string, unknown>;
  return {
    user: stringValue(
      record.user ?? attributes["agentgateway.user"] ?? attributes.user,
    ),
    group: stringValue(
      record.group ?? attributes["agentgateway.group"] ?? attributes.group,
    ),
  };
}

function stringValue(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function logMessagePreview(entry: LogEntry) {
  const prompt = payloadValue(entry, "gen_ai.prompt", "requestPrompt");
  const messages = messagesFromPrompt(prompt);
  const message =
    findLastMessage(
      messages,
      (item) => item.role === "user" && Boolean(item.content.trim()),
    ) ?? findLastMessage(messages, (item) => Boolean(item.content.trim()));
  const text = message?.content.trim() || entry.error || "";
  return text.length > 180 ? `${text.slice(0, 177)}...` : text;
}

function findLastMessage(
  messages: RenderedLogMessage[],
  predicate: (message: RenderedLogMessage) => boolean,
) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (predicate(messages[index])) return messages[index];
  }
  return null;
}

function LogCallRow(props: {
  entry: LogEntry;
  detail: LogEntry;
  expanded: boolean;
  loading: boolean;
  onToggle: () => void;
  onOpenSettings?: () => void;
}) {
  const originalModel = originalModelForLog(props.entry);
  const identity = logIdentity(props.entry);
  const statusBad = Boolean(
    props.entry.error || (props.entry.httpStatus ?? 0) >= 400,
  );
  const preview = logMessagePreview(props.entry);
  const hasPreview = Boolean(preview);
  return (
    <article
      className={props.expanded ? "log-call-card expanded" : "log-call-card"}
    >
      <button
        className="log-call-summary"
        type="button"
        onClick={props.onToggle}
        aria-expanded={props.expanded}
      >
        <span
          className={statusBad ? "log-status-rail bad" : "log-status-rail ok"}
          aria-hidden="true"
        />
        <span className="log-call-time">
          <span>{formatDate(props.entry.completedAt)}</span>
          <small>{formatRelativeTime(props.entry.completedAt)}</small>
        </span>
        <span className="log-type-chip">
          {(props.entry.genAi.operationName ?? "chat").toUpperCase()}
        </span>
        <span
          className={hasPreview ? "log-call-main" : "log-call-main no-preview"}
        >
          {hasPreview ? (
            <span className="log-call-title-row">
              <span className="log-message-preview">{preview}</span>
              <span className={statusBad ? "badge bad" : "badge ok"}>
                {props.entry.httpStatus ?? "n/a"}
              </span>
            </span>
          ) : null}
          <span className="log-call-subtitle">
            {!hasPreview ? (
              <span className="log-call-inline-status">
                <span className={statusBad ? "badge bad" : "badge ok"}>
                  {props.entry.httpStatus ?? "n/a"}
                </span>
              </span>
            ) : null}
            <span className="log-model-flow">
              {originalModel &&
              originalModel !== props.entry.genAi.requestModel ? (
                <>
                  <strong>{originalModel}</strong>
                  <ArrowRight size={14} />
                </>
              ) : null}
              <strong>
                {props.entry.genAi.requestModel ?? "unknown model"}
              </strong>
              {props.entry.genAi.responseModel &&
              props.entry.genAi.responseModel !==
                props.entry.genAi.requestModel ? (
                <>
                  <ArrowRight size={14} />
                  <strong>{props.entry.genAi.responseModel}</strong>
                </>
              ) : null}
            </span>
            <span className="log-identity-chip">
              provider: {props.entry.genAi.providerName ?? "unknown provider"}
            </span>
            {identity.user ? (
              <span className="log-identity-chip">user: {identity.user}</span>
            ) : null}
            {identity.group ? (
              <span className="log-identity-chip">group: {identity.group}</span>
            ) : null}
          </span>
        </span>
        <span className="log-call-metrics">
          <span>
            <Clock3 size={14} />
            {formatNumber(props.entry.durationMs)} ms
          </span>
          <TokenSummary entry={props.entry} />
          <CostSummary entry={props.entry} />
        </span>
        <ChevronDown className="log-call-chevron" size={18} />
      </button>
      {props.expanded ? (
        <div className="expanded-log">
          <div className="editor-title">
            <div>
              <h3>Request detail</h3>
            </div>
            <button
              className="button"
              type="button"
              onClick={() => downloadJson(props.detail)}
            >
              <Download size={16} />
              JSON
            </button>
          </div>
          {props.loading ? (
            <StatusBanner state="loading" title="Loading log payload" />
          ) : null}
          <LogDetailView
            entry={props.detail}
            onOpenSettings={props.onOpenSettings}
          />
        </div>
      ) : null}
    </article>
  );
}
function LogDetailView(props: {
  entry: LogEntry;
  onOpenSettings?: () => void;
}) {
  const messages = logConversation(props.entry);
  return (
    <div className="log-detail-view">
      <div className="log-debug-grid">
        <LogModelFlow entry={props.entry} />
        <LogTimingPanel entry={props.entry} />
      </div>

      {props.entry.error ? (
        <StatusBanner state="bad" title="Gateway error">
          {props.entry.error}
        </StatusBanner>
      ) : null}

      {messages.length ? (
        <section className="log-conversation">
          <div className="section-heading compact">
            <h3>Conversation</h3>
          </div>
          <div className="mini-chat log-mini-chat">
            {messages.map((message, index) => (
              <LogMessageView
                message={message}
                key={`${message.role}-${index}`}
              />
            ))}
          </div>
        </section>
      ) : (
        <StatusBanner state="info" title="Prompt logging is off">
          Enable "Include prompts and completions in logs" in{" "}
          {props.onOpenSettings ? (
            <button
              className="link-button"
              type="button"
              onClick={props.onOpenSettings}
            >
              log settings
            </button>
          ) : (
            "log settings"
          )}{" "}
          to capture request and response payloads.
        </StatusBanner>
      )}

      <details className="log-raw-details">
        <summary>Raw log JSON</summary>
        <JsonBlock value={props.entry} />
      </details>
    </div>
  );
}

function LogModelFlow(props: { entry: LogEntry }) {
  const original = originalModelForLog(props.entry);
  const identity = logIdentity(props.entry);
  const request = props.entry.genAi.requestModel ?? "n/a";
  const response = props.entry.genAi.responseModel ?? "n/a";
  const hasRouting =
    (original && original !== request) ||
    (response && response !== request && response !== "n/a");
  return (
    <section className="log-debug-panel model-flow">
      <div className="log-debug-panel-title">
        <Route size={15} />
        <h3>Model{hasRouting ? " flow" : ""}</h3>
      </div>
      {hasRouting ? (
        <div className="model-flow-steps">
          <ModelFlowStep label="Client requested" value={original ?? request} />
          <ArrowRight size={15} />
          <ModelFlowStep label="Gateway sent" value={request} />
          <ArrowRight size={15} />
          <ModelFlowStep label="Provider returned" value={response} />
        </div>
      ) : (
        <div className="model-flow-simple">
          <strong>{request}</strong>
          <span>
            {response !== "n/a" && response !== request
              ? `returned as ${response}`
              : "no routing"}
          </span>
        </div>
      )}
      <div className="log-debug-note">
        <span>{props.entry.genAi.providerName ?? "unknown provider"}</span>
        <span>{props.entry.genAi.operationName ?? "unknown operation"}</span>
        {identity.user ? <span>user: {identity.user}</span> : null}
        {identity.group ? <span>group: {identity.group}</span> : null}
      </div>
    </section>
  );
}

function ModelFlowStep(props: { label: string; value: string }) {
  return (
    <div className="model-flow-step">
      <span>{props.label}</span>
      <strong>{props.value}</strong>
    </div>
  );
}

function LogTimingPanel(props: { entry: LogEntry }) {
  const a = props.entry.attributes;
  const inputTokens =
    props.entry.usage.inputTokens ??
    attributeNumber(a, ["gen_ai.usage.input_tokens"]);
  const outputTokens =
    props.entry.usage.outputTokens ??
    attributeNumber(a, ["gen_ai.usage.output_tokens"]);
  const totalTokens =
    props.entry.usage.totalTokens ??
    attributeNumber(a, ["gen_ai.usage.total_tokens"]);
  const cacheReadTokens = attributeNumber(a, [
    "gen_ai.usage.cache_read.input_tokens",
    "cacheTokens",
    "cachedTokens",
    "cache_tokens",
    "cached_tokens",
  ]);
  const cacheWriteTokens = attributeNumber(a, [
    "gen_ai.usage.cache_creation.input_tokens",
  ]);
  const reasoningTokens = attributeNumber(a, ["gen_ai.usage.reasoning_tokens"]);
  const inputAudioTokens = attributeNumber(a, [
    "gen_ai.usage.input_audio_tokens",
  ]);
  const outputAudioTokens = attributeNumber(a, [
    "gen_ai.usage.output_audio_tokens",
  ]);

  const inputCost = attributeNumber(a, ["agw.ai.usage.cost.input"]);
  const outputCost = attributeNumber(a, ["agw.ai.usage.cost.output"]);
  const cacheReadCost = attributeNumber(a, ["agw.ai.usage.cost.cacheRead"]);
  const cacheWriteCost = attributeNumber(a, ["agw.ai.usage.cost.cacheWrite"]);
  const reasoningCost = attributeNumber(a, ["agw.ai.usage.cost.reasoning"]);
  const inputAudioCost = attributeNumber(a, ["agw.ai.usage.cost.inputAudio"]);
  const outputAudioCost = attributeNumber(a, ["agw.ai.usage.cost.outputAudio"]);
  const totalCost =
    props.entry.cost ?? attributeNumber(a, ["agw.ai.usage.cost.total"]);

  const showBar =
    inputTokens != null &&
    outputTokens != null &&
    inputTokens + outputTokens > 0;
  return (
    <section className="log-debug-panel">
      <div className="log-debug-panel-title">
        <Clock3 size={15} />
        <h3>Timing and usage</h3>
      </div>
      {showBar ? (
        <TokenBar
          input={inputTokens!}
          output={outputTokens!}
          cache={cacheReadTokens ?? undefined}
        />
      ) : null}
      <div className="log-fact-list">
        <LogFact
          label="Duration"
          value={`${formatNumber(props.entry.durationMs)} ms`}
        />
        <UsageFact label="Input" tokens={inputTokens} cost={inputCost} />
        <UsageFact label="Output" tokens={outputTokens} cost={outputCost} />
        {cacheReadTokens || cacheReadCost ? (
          <UsageFact
            label="Cache read"
            tokens={cacheReadTokens}
            cost={cacheReadCost}
          />
        ) : null}
        {cacheWriteTokens || cacheWriteCost ? (
          <UsageFact
            label="Cache write"
            tokens={cacheWriteTokens}
            cost={cacheWriteCost}
          />
        ) : null}
        {reasoningTokens || reasoningCost ? (
          <UsageFact
            label="Reasoning"
            tokens={reasoningTokens}
            cost={reasoningCost}
          />
        ) : null}
        {inputAudioTokens || inputAudioCost ? (
          <UsageFact
            label="Audio in"
            tokens={inputAudioTokens}
            cost={inputAudioCost}
          />
        ) : null}
        {outputAudioTokens || outputAudioCost ? (
          <UsageFact
            label="Audio out"
            tokens={outputAudioTokens}
            cost={outputAudioCost}
          />
        ) : null}
        <UsageFact
          label="Total"
          tokens={totalTokens}
          cost={totalCost}
          alwaysShow
        />
      </div>
    </section>
  );
}

function UsageFact(props: {
  label: string;
  tokens?: number | null;
  cost?: number | null;
  alwaysShow?: boolean;
}) {
  if (!props.alwaysShow && !props.tokens && !props.cost) return null;
  const parts: string[] = [];
  if (props.tokens != null) parts.push(`${formatNumber(props.tokens)} tokens`);
  if (props.cost != null && props.cost > 0) parts.push(formatCost(props.cost));
  return (
    <div className="log-fact">
      <span>{props.label}</span>
      <strong>{parts.length ? parts.join(" / ") : "n/a"}</strong>
    </div>
  );
}

function LogFact(props: {
  label: string;
  value: string;
  mono?: boolean;
  copyable?: string;
}) {
  return (
    <div className="log-fact">
      <span>{props.label}</span>
      <strong className={props.mono ? "mono" : undefined}>
        {props.value}
        {props.copyable ? <CopyButton value={props.copyable} /> : null}
      </strong>
    </div>
  );
}

function LogMessageView(props: { message: RenderedLogMessage }) {
  const Icon =
    props.message.role === "assistant"
      ? Bot
      : props.message.role === "tool"
        ? Braces
        : User;
  const isSystem = props.message.role === "system";
  const isLongSystem = isSystem && props.message.content.length > 240;
  const [systemCollapsed, setSystemCollapsed] = useState(isLongSystem);

  return (
    <div className={`chat-message ${props.message.role}`}>
      <div className="chat-avatar">
        <Icon size={16} />
      </div>
      <div className="chat-bubble">
        {props.message.toolCalls?.length ? (
          <div className="tool-call-summary">
            {props.message.content.trim() ? (
              <p>{props.message.content}</p>
            ) : null}
            {props.message.toolCalls.map((call, index) => (
              <ToolCallRow call={call} key={`${call.name}-${index}`} />
            ))}
          </div>
        ) : props.message.role === "tool" ? (
          <div className="tool-call-summary">
            <ToolResultRow
              name={props.message.name}
              content={props.message.content}
            />
          </div>
        ) : isSystem ? (
          <>
            <span
              className={systemCollapsed ? "chat-bubble-collapsed" : undefined}
            >
              {props.message.content}
            </span>
            {isLongSystem ? (
              <button
                className="chat-message-system-toggle"
                type="button"
                onClick={() => setSystemCollapsed((c) => !c)}
              >
                {systemCollapsed ? (
                  <ChevronRight size={12} />
                ) : (
                  <ChevronDown size={12} />
                )}
                {systemCollapsed
                  ? `Show full system prompt (${props.message.content.length} chars)`
                  : "Collapse"}
              </button>
            ) : null}
          </>
        ) : (
          props.message.content
        )}
      </div>
    </div>
  );
}

function ToolCallRow(props: { call: { name: string; arguments?: unknown } }) {
  const [expanded, setExpanded] = useState(false);
  const summary = summarizeLogValue(props.call.arguments);
  const hasArgs =
    props.call.arguments != null &&
    props.call.arguments !== "" &&
    summary !== "{}";
  return (
    <div className="tool-call-row">
      <span className="tool-pill">Tool call</span>
      <strong>{props.call.name}</strong>
      {hasArgs ? (
        <div className="tool-call-args">
          {expanded ? (
            <div className="tool-call-args-block">
              <JsonBlock value={props.call.arguments} />
            </div>
          ) : (
            <span className="tool-call-args-summary">{summary}</span>
          )}
          <button
            className="tool-call-args-toggle"
            type="button"
            onClick={() => setExpanded((e) => !e)}
          >
            {expanded ? "Collapse" : "Expand args"}
          </button>
        </div>
      ) : (
        <small className="tool-call-args-summary">no args</small>
      )}
    </div>
  );
}

function ToolResultRow(props: { name?: string; content: string }) {
  const [expanded, setExpanded] = useState(false);
  const isLong = props.content.length > 120;
  const parsed = (() => {
    try {
      return JSON.parse(props.content);
    } catch {
      return null;
    }
  })();
  return (
    <div className="tool-call-row">
      <span className="tool-pill">Tool result</span>
      <strong>{props.name || "unknown"}</strong>
      <div className="tool-call-args">
        {expanded ? (
          <div className="tool-call-args-block">
            {parsed != null ? (
              <JsonBlock value={parsed} />
            ) : (
              <span
                style={{
                  whiteSpace: "pre-wrap",
                  fontSize: 12,
                  fontFamily: "var(--mono)",
                }}
              >
                {props.content}
              </span>
            )}
          </div>
        ) : (
          <span className="tool-call-args-summary">{props.content}</span>
        )}
        {isLong ? (
          <button
            className="tool-call-args-toggle"
            type="button"
            onClick={() => setExpanded((e) => !e)}
          >
            {expanded ? "Collapse" : "Expand result"}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function logConversation(entry: LogEntry): RenderedLogMessage[] {
  const prompt = payloadValue(entry, "gen_ai.prompt", "requestPrompt");
  const completion = payloadValue(
    entry,
    "gen_ai.completion",
    "responseCompletion",
  );
  return [...messagesFromPrompt(prompt), ...messagesFromCompletion(completion)];
}

function payloadValue(
  entry: LogEntry,
  attributeKey: string,
  payloadKey: "requestPrompt" | "responseCompletion",
) {
  const attribute = attributeValue(entry.attributes, attributeKey);
  if (attribute !== undefined && attribute !== null)
    return parseMaybeJson(attribute);
  return parseMaybeJson(entry.payload?.[payloadKey]);
}

function attributeValue(attributes: unknown, key: string): unknown {
  if (!attributes || typeof attributes !== "object") return undefined;
  const record = attributes as Record<string, unknown>;
  if (key in record) return record[key];
  const parts = key.split(".");
  let current: unknown = record;
  for (const part of parts) {
    if (!current || typeof current !== "object") return undefined;
    current = (current as Record<string, unknown>)[part];
  }
  return current;
}

function parseMaybeJson(value: unknown): unknown {
  if (typeof value !== "string") return value;
  const trimmed = value.trim();
  if (!trimmed) return "";
  if (!/^[{["]/.test(trimmed)) return value;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return value;
  }
}

function messagesFromPrompt(value: unknown): RenderedLogMessage[] {
  if (!value) return [];
  if (Array.isArray(value)) return value.flatMap(messageFromUnknown);
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (Array.isArray(record.messages))
      return record.messages.flatMap(messageFromUnknown);
    if (Array.isArray(record.contents))
      return record.contents.flatMap(messageFromUnknown);
    if (record.prompt !== undefined)
      return [{ role: "user", content: contentText(record.prompt) }];
  }
  return [{ role: "user", content: contentText(value) }];
}

function messagesFromCompletion(value: unknown): RenderedLogMessage[] {
  if (!value) return [];
  if (Array.isArray(value))
    return value.flatMap(messageFromUnknown).map((message) => ({
      ...message,
      role: message.role === "user" ? "assistant" : message.role,
    }));
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (Array.isArray(record.choices)) {
      return record.choices
        .flatMap((choice) => {
          const choiceRecord =
            choice && typeof choice === "object"
              ? (choice as Record<string, unknown>)
              : {};
          return messageFromUnknown(
            choiceRecord.message ?? choiceRecord.delta ?? choiceRecord.text,
          );
        })
        .map((message) => ({
          ...message,
          role: message.role === "user" ? "assistant" : message.role,
        }));
    }
    if (record.message !== undefined)
      return messageFromUnknown(record.message).map((message) => ({
        ...message,
        role: "assistant",
      }));
    if (
      record.content !== undefined ||
      record.text !== undefined ||
      record.tool_calls !== undefined
    ) {
      return messageFromUnknown({ role: "assistant", ...record });
    }
  }
  return [{ role: "assistant", content: contentText(value) }];
}

function messageFromUnknown(value: unknown): RenderedLogMessage[] {
  if (value === undefined || value === null) return [];
  if (typeof value !== "object" || Array.isArray(value))
    return [{ role: "user", content: contentText(value) }];
  const record = value as Record<string, unknown>;
  const role = normalizeRole(record.role);
  const content = contentText(
    record.content ?? record.text ?? record.message ?? "",
  );
  const toolCalls = toolCallsFromUnknown(record.tool_calls ?? record.toolCalls);
  const name = typeof record.name === "string" ? record.name : undefined;
  return [{ role, content, name, toolCalls }];
}

function normalizeRole(value: unknown): RenderedLogMessage["role"] {
  if (value === "system" || value === "assistant" || value === "tool")
    return value;
  return "user";
}

function toolCallsFromUnknown(value: unknown): RenderedLogMessage["toolCalls"] {
  if (!Array.isArray(value)) return undefined;
  const calls = value.flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const record = item as Record<string, unknown>;
    const fn =
      record.function && typeof record.function === "object"
        ? (record.function as Record<string, unknown>)
        : record;
    const name = typeof fn.name === "string" ? fn.name : "unknown";
    return [{ name, arguments: parseMaybeJson(fn.arguments) }];
  });
  return calls.length ? calls : undefined;
}

function contentText(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        if (typeof item === "string") return item;
        if (item && typeof item === "object") {
          const record = item as Record<string, unknown>;
          if (typeof record.text === "string") return record.text;
          if (typeof record.content === "string") return record.content;
        }
        return summarizeLogValue(item);
      })
      .join("\n");
  }
  return summarizeLogValue(value);
}

function summarizeLogValue(value: unknown) {
  if (value === undefined || value === null || value === "") return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function downloadJson(value: unknown) {
  const blob = new Blob([JSON.stringify(value, null, 2)], {
    type: "application/json",
  });
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
  const cache = attributeNumber(props.entry.attributes, [
    "cacheTokens",
    "cachedTokens",
    "cache_tokens",
    "cached_tokens",
  ]);
  const total = props.entry.usage.totalTokens;
  const detail = [
    `in: ${formatNumber(input)}`,
    `out: ${formatNumber(output)}`,
    `cache: ${formatNumber(cache)}`,
    `total: ${formatNumber(total)}`,
  ].join("\n");

  return (
    <span className="token-summary">
      <span>
        <ArrowDown size={14} />
        {input == null ? "—" : formatNumber(input)}
      </span>
      <span>
        <ArrowUp size={14} />
        {output == null ? "—" : formatNumber(output)}
      </span>
      <span className="token-tooltip" aria-hidden="true">
        {detail}
      </span>
    </span>
  );
}

function CostSummary(props: { entry: LogEntry }) {
  if (props.entry.cost == null) return null;
  return (
    <span className="log-cost-summary">{formatCost(props.entry.cost)}</span>
  );
}

function attributeNumber(value: unknown, keys: string[]) {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  for (const key of keys) {
    const direct = record[key];
    if (typeof direct === "number") return direct;
    if (typeof direct === "string" && Number.isFinite(Number(direct)))
      return Number(direct);
  }
  return undefined;
}

function CopyButton(props: { value: string }) {
  const [copied, setCopied] = useState(false);
  function handleCopy() {
    void navigator.clipboard.writeText(props.value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }
  return (
    <button
      className={`copy-button${copied ? " copied" : ""}`}
      type="button"
      aria-label="Copy to clipboard"
      onClick={handleCopy}
    >
      {copied ? <Check size={11} /> : <Copy size={11} />}
    </button>
  );
}

function TokenBar(props: { input: number; output: number; cache?: number }) {
  const total = props.input + props.output + (props.cache ?? 0);
  if (!total) return null;
  const inputPct = (props.input / total) * 100;
  const outputPct = (props.output / total) * 100;
  const cachePct = ((props.cache ?? 0) / total) * 100;
  const title = [
    `in: ${formatNumber(props.input)}`,
    `out: ${formatNumber(props.output)}`,
    props.cache != null ? `cache: ${formatNumber(props.cache)}` : null,
  ]
    .filter(Boolean)
    .join(" / ");
  return (
    <div className="token-bar" title={title}>
      <div className="token-bar-input" style={{ width: `${inputPct}%` }} />
      <div className="token-bar-output" style={{ width: `${outputPct}%` }} />
      {props.cache ? (
        <div className="token-bar-cache" style={{ width: `${cachePct}%` }} />
      ) : null}
    </div>
  );
}
