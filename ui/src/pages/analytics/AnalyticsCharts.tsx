import {
  Bar,
  BarChart as RechartsBarChart,
  CartesianGrid,
  Cell,
  LabelList,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  XAxis,
  YAxis,
} from "recharts";
import { formatNumber } from "../../components/Primitives";
import type {
  AnalyticsGroup,
  AnalyticsTimeBucket,
  LogFilters,
  TimeRange,
} from "../../types";

export type AnalyticsDimension =
  | "model"
  | "user"
  | "group"
  | "provider"
  | "userAgent";
export type AnalyticsMeasure = "tokens" | "cost" | "requests";

export const ANALYTICS_DIMENSIONS: Array<{
  value: AnalyticsDimension;
  label: string;
  filterLabel: string;
}> = [
  { value: "model", label: "Model", filterLabel: "Models" },
  { value: "user", label: "User", filterLabel: "Users" },
  { value: "group", label: "Group", filterLabel: "Groups" },
  { value: "provider", label: "Provider", filterLabel: "Providers" },
  { value: "userAgent", label: "User agent", filterLabel: "User agents" },
];

export function AnalyticsTimelineChart(props: {
  data: AnalyticsTimelineRow[];
  series: Array<{ key: string; label: string; color: string }>;
  measure: AnalyticsMeasure;
}) {
  const xTicks = analyticsAxisTicks(props.data);
  return (
    <div className="analytics-chart-frame">
      {props.data.length ? null : (
        <div className="activity-empty">
          No analytics in the selected window.
        </div>
      )}
      <ResponsiveContainer width="100%" height="100%">
        <RechartsBarChart
          data={props.data}
          margin={{ top: 12, right: 8, bottom: 0, left: 0 }}
        >
          <CartesianGrid
            stroke="var(--line)"
            strokeDasharray="4 4"
            vertical={false}
          />
          <XAxis
            axisLine={false}
            dataKey="startMs"
            interval={0}
            padding={{ left: 12, right: 12 }}
            ticks={xTicks}
            tickFormatter={(value) =>
              formatTimelineAxisTick(Number(value), props.data)
            }
            tick={{ fill: "var(--muted)", fontSize: 12, fontWeight: 560 }}
            tickMargin={8}
            tickLine={false}
          />
          <YAxis
            allowDecimals={props.measure !== "cost"}
            axisLine={false}
            domain={props.measure === "cost" ? [0, costAxisMax] : undefined}
            tick={{ fill: "var(--muted)", fontSize: 12, fontWeight: 520 }}
            tickFormatter={(value) => formatAxisNumber(value, props.measure)}
            tickLine={false}
            width={54}
          />
          <RechartsTooltip
            content={
              <AnalyticsTimelineTooltip
                measure={props.measure}
                series={props.series}
              />
            }
            cursor={{
              fill: "color-mix(in srgb, var(--primary) 8%, transparent)",
            }}
          />
          {props.series.map((item) => (
            <Bar
              dataKey={item.key}
              fill={item.color}
              isAnimationActive={false}
              key={item.key}
              stackId="analytics"
            />
          ))}
        </RechartsBarChart>
      </ResponsiveContainer>
      <div className="dense-chart-legend">
        {props.series.slice(0, 10).map((item) => (
          <span key={item.key}>
            <i style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}

export function AnalyticsBreakdownChart(props: {
  data: AnalyticsBreakdownRow[];
  measure: AnalyticsMeasure;
}) {
  const contentHeight = Math.max(180, props.data.length * 34 + 40);
  const viewportHeight = Math.min(520, contentHeight);
  return (
    <div className="analytics-chart-frame compact horizontal breakdown">
      {props.data.length ? null : (
        <div className="activity-empty">
          No analytics in the selected window.
        </div>
      )}
      <div
        className="analytics-breakdown-scroll"
        style={{ maxHeight: viewportHeight }}
      >
        <div
          className="analytics-breakdown-chart"
          style={{ height: contentHeight }}
        >
          <ResponsiveContainer width="100%" height="100%">
            <RechartsBarChart
              data={props.data}
              layout="vertical"
              margin={{ top: 8, right: 76, bottom: 8, left: 0 }}
            >
              <CartesianGrid
                stroke="var(--line)"
                strokeDasharray="4 4"
                vertical={false}
              />
              <XAxis hide type="number" />
              <YAxis
                axisLine={false}
                dataKey="name"
                interval={0}
                tick={<BreakdownAxisTick />}
                tickLine={false}
                type="category"
                width={210}
              />
              <RechartsTooltip
                content={<AnalyticsBreakdownTooltip measure={props.measure} />}
                cursor={{
                  fill: "color-mix(in srgb, var(--primary) 8%, transparent)",
                }}
              />
              <Bar barSize={18} dataKey="value" isAnimationActive={false}>
                {props.data.map((item) => (
                  <Cell fill={item.color} key={item.id} />
                ))}
                <LabelList
                  dataKey="value"
                  formatter={(value: unknown) =>
                    formatMeasureValue(Number(value) || 0, props.measure)
                  }
                  position="right"
                  style={{
                    fill: "var(--muted)",
                    fontFamily: "var(--mono)",
                    fontSize: 12,
                    fontWeight: 450,
                  }}
                />
              </Bar>
            </RechartsBarChart>
          </ResponsiveContainer>
        </div>
      </div>
    </div>
  );
}

function BreakdownAxisTick(props: {
  x?: number;
  y?: number;
  payload?: { value?: unknown };
}) {
  const x = Number(props.x ?? 0);
  const y = Number(props.y ?? 0);
  const label = truncateBreakdownLabel(props.payload?.value);
  return (
    <text
      className="analytics-breakdown-axis-label"
      x={x - 8}
      y={y}
      dominantBaseline="middle"
      textAnchor="end"
    >
      <title>{String(props.payload?.value ?? "")}</title>
      {label}
    </text>
  );
}

function AnalyticsTimelineTooltip(props: {
  active?: boolean;
  label?: string;
  payload?: Array<{
    dataKey?: string | number;
    value?: number;
    payload?: AnalyticsTimelineRow;
  }>;
  series: Array<{ key: string; label: string; color: string }>;
  measure: AnalyticsMeasure;
}) {
  if (!props.active || !props.payload?.length) return null;
  const label =
    props.payload.find((item) => item.payload?.tooltipLabel)?.payload
      ?.tooltipLabel ?? props.label;
  const rows = props.payload
    .map((item) => {
      const key = String(item.dataKey ?? "");
      const series = props.series.find((candidate) => candidate.key === key);
      return series && item.value ? { ...series, value: item.value } : null;
    })
    .filter(
      (
        item,
      ): item is { key: string; label: string; color: string; value: number } =>
        Boolean(item),
    )
    .sort((a, b) => b.value - a.value);
  if (!rows.length) return null;
  return (
    <div className="chart-tooltip">
      <strong>{label}</strong>
      {rows.map((row) => (
        <span key={row.key}>
          <i style={{ background: row.color }} />
          {row.label}
          <code>{formatMeasureValue(row.value, props.measure)}</code>
        </span>
      ))}
    </div>
  );
}

function AnalyticsBreakdownTooltip(props: {
  active?: boolean;
  payload?: Array<{ payload?: AnalyticsBreakdownRow }>;
  measure: AnalyticsMeasure;
}) {
  const item = props.payload?.[0]?.payload;
  if (!props.active || !item) return null;
  return (
    <div className="chart-tooltip compact">
      <strong>{item.name}</strong>
      <span>
        <i style={{ background: item.color }} />
        {measureLabel(props.measure)}
        <code>{formatMeasureValue(item.value ?? 0, props.measure)}</code>
      </span>
      <span>
        <i style={{ background: "#64748b" }} />
        Tokens<code>{formatNumber(item.tokens ?? 0)}</code>
      </span>
      <span>
        <i style={{ background: "#64748b" }} />
        Calls<code>{formatNumber(item.requests ?? 0)}</code>
      </span>
    </div>
  );
}

export function isAnalyticsDimension(
  value: string,
): value is AnalyticsDimension {
  return (
    value === "model" ||
    value === "user" ||
    value === "group" ||
    value === "provider" ||
    value === "userAgent"
  );
}

export function analyticsGroupBy(dimension: AnalyticsDimension) {
  if (dimension === "model") return { field: "requestModel" as const };
  if (dimension === "provider") return { field: "provider" as const };
  if (dimension === "group")
    return { field: "attributes" as const, key: "agentgateway.group" };
  if (dimension === "userAgent")
    return { field: "attributes" as const, key: "user_agent.name" };
  return { field: "attributes" as const, key: "agentgateway.user" };
}

export function analyticsLogFilters(
  filters: Record<AnalyticsDimension, string[]>,
): LogFilters {
  const next: LogFilters = {};
  if (filters.model.length) next.requestModel = filters.model;
  if (filters.provider.length) next.provider = filters.provider;
  const attributes: Record<string, string[]> = {};
  if (filters.user.length) attributes["agentgateway.user"] = filters.user;
  if (filters.group.length) attributes["agentgateway.group"] = filters.group;
  if (filters.userAgent.length)
    attributes["user_agent.name"] = filters.userAgent;
  if (Object.keys(attributes).length) next.attributes = attributes;
  return next;
}

export function analyticsFilterOptions(
  usage: AnalyticsGroup[],
  dimensions: AnalyticsDimension[] = ANALYTICS_DIMENSIONS.map(
    (item) => item.value,
  ),
): Record<AnalyticsDimension, string[]> {
  return {
    model: dimensions.includes("model")
      ? sortedUnique(
          usage
            .map((item) => analyticsGroupValue(item.group, "model"))
            .filter(isNonEmptyString),
        )
      : [],
    provider: dimensions.includes("provider")
      ? sortedUnique(
          usage
            .map((item) => analyticsGroupValue(item.group, "provider"))
            .filter(isNonEmptyString),
        )
      : [],
    user: dimensions.includes("user")
      ? sortedUnique(
          usage
            .map((item) => analyticsGroupValue(item.group, "user"))
            .filter(isNonEmptyString),
        )
      : [],
    group: dimensions.includes("group")
      ? sortedUnique(
          usage
            .map((item) => analyticsGroupValue(item.group, "group"))
            .filter(isNonEmptyString),
        )
      : [],
    userAgent: dimensions.includes("userAgent")
      ? sortedUnique(
          usage
            .map((item) => analyticsGroupValue(item.group, "userAgent"))
            .filter(isNonEmptyString),
        )
      : [],
  };
}

export function analyticsFilterOptionsFromResponse(
  options: Record<string, string[]> | null | undefined,
  dimensions: AnalyticsDimension[] = ANALYTICS_DIMENSIONS.map(
    (item) => item.value,
  ),
) {
  if (!options) return null;
  return {
    model: dimensions.includes("model")
      ? sortedUnique(options.requestModel ?? options.model ?? [])
      : [],
    provider: dimensions.includes("provider")
      ? sortedUnique(options.provider ?? options.providerName ?? [])
      : [],
    user: dimensions.includes("user")
      ? sortedUnique(
          options["agentgateway.user"] ??
            options.user ??
            options["attributes.user"] ??
            [],
        )
      : [],
    group: dimensions.includes("group")
      ? sortedUnique(
          options["agentgateway.group"] ??
            options.group ??
            options["attributes.group"] ??
            [],
        )
      : [],
    userAgent: dimensions.includes("userAgent")
      ? sortedUnique(
          options["user_agent.name"] ??
            options.userAgent ??
            options["attributes.userAgent"] ??
            [],
        )
      : [],
  } satisfies Record<AnalyticsDimension, string[]>;
}

export function analyticsFiltersKey(
  filters: Record<AnalyticsDimension, string[]>,
) {
  return JSON.stringify({
    model: filters.model,
    provider: filters.provider,
    user: filters.user,
    group: filters.group,
    userAgent: filters.userAgent,
  });
}

export function emptyAnalyticsFilterOptions(): Record<
  AnalyticsDimension,
  string[]
> {
  return { model: [], provider: [], user: [], group: [], userAgent: [] };
}

export function mergeAnalyticsFilterOptions(
  current: Record<AnalyticsDimension, string[]>,
  discovered: Record<AnalyticsDimension, string[]>,
  activeFilters: Record<AnalyticsDimension, string[]>,
) {
  const next = emptyAnalyticsFilterOptions();
  for (const dimension of ANALYTICS_DIMENSIONS.map((item) => item.value)) {
    next[dimension] = sortedUnique([
      ...current[dimension],
      ...discovered[dimension],
      ...activeFilters[dimension],
    ]);
  }
  return next;
}

export type AnalyticsTimelineRow = {
  name: string;
  tooltipLabel: string;
  startMs: number;
  requests: number;
  tokens: number;
  cost: number;
} & Record<string, string | number>;

const ANALYTICS_CHART_COLORS = [
  "#2563eb",
  "#7c3aed",
  "#059669",
  "#db2777",
  "#d97706",
  "#0891b2",
  "#4f46e5",
  "#65a30d",
  "#be123c",
  "#0f766e",
];
const PROVIDER_COLORS: Record<string, string> = {
  anthropic: "#7c3aed",
  bedrock: "#d97706",
  custom: "#64748b",
  google: "#059669",
  openai: "#2563eb",
};

export function analyticsTimelineData(
  buckets: AnalyticsTimeBucket[],
  timeRange: TimeRange,
  bucketSeconds: number,
  dimensions: AnalyticsDimension[],
  measure: AnalyticsMeasure,
) {
  const now = Date.now();
  const start = timeRange.from
    ? new Date(timeRange.from).getTime()
    : now - 24 * 60 * 60 * 1000;
  const end = timeRange.to ? new Date(timeRange.to).getTime() : now;
  const safeStart = Number.isFinite(start) ? start : now - 24 * 60 * 60 * 1000;
  const safeEnd = Number.isFinite(end) && end > safeStart ? end : now;
  const rangeMs = safeEnd - safeStart;
  const bucketMs = Math.max(1, bucketSeconds * 1000);
  const bucketCount = Math.max(1, Math.ceil(rangeMs / bucketMs));
  const dayMs = 24 * 60 * 60 * 1000;
  const axisFormatter = new Intl.DateTimeFormat(
    undefined,
    bucketMs >= dayMs
      ? { month: "short", day: "numeric" }
      : rangeMs > 48 * 60 * 60 * 1000
        ? { month: "short", day: "numeric", hour: "numeric" }
        : {
            hour: "numeric",
            minute: rangeMs <= 6 * 60 * 60 * 1000 ? "2-digit" : undefined,
          },
  );
  const tooltipFormatter = new Intl.DateTimeFormat(
    undefined,
    bucketMs >= dayMs
      ? { month: "short", day: "numeric" }
      : rangeMs > 48 * 60 * 60 * 1000
        ? { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }
        : { hour: "numeric", minute: "2-digit" },
  );
  const rows: AnalyticsTimelineRow[] = Array.from(
    { length: bucketCount },
    (_, index) => {
      const bucketStart = safeStart + index * bucketMs;
      return {
        name: axisFormatter.format(new Date(bucketStart)),
        tooltipLabel: tooltipFormatter.format(new Date(bucketStart)),
        startMs: bucketStart,
        requests: 0,
        tokens: 0,
        cost: 0,
      };
    },
  );
  const totals = new Map<
    string,
    {
      label: string;
      requests: number;
      tokens: number;
      cost: number;
      value: number;
    }
  >();

  for (const bucket of buckets) {
    const bucketStart = new Date(bucket.start).getTime();
    if (
      !Number.isFinite(bucketStart) ||
      bucketStart < safeStart ||
      bucketStart > safeEnd
    )
      continue;
    const index = Math.min(
      bucketCount - 1,
      Math.max(0, Math.floor((bucketStart - safeStart) / bucketMs)),
    );
    const values = Object.fromEntries(
      dimensions.map((dimension) => [
        dimension,
        analyticsGroupValue(bucket.group, dimension),
      ]),
    );
    if (dimensions.length > 0 && !Object.values(values).some(Boolean)) continue;
    const label = analyticsDisplayName(values);
    const key = `series_${stableSeriesKey(label)}`;
    const tokens = bucket.totalTokens;
    const requests = bucket.requests;
    const cost = analyticsBucketCost(bucket);
    const value = analyticsMeasureValue({ tokens, requests, cost }, measure);
    rows[index].requests += bucket.requests;
    rows[index].tokens += tokens;
    rows[index].cost += cost;
    rows[index][key] = Number(rows[index][key] ?? 0) + value;
    const existing = totals.get(key) ?? {
      label,
      requests: 0,
      tokens: 0,
      cost: 0,
      value: 0,
    };
    existing.requests += bucket.requests;
    existing.tokens += tokens;
    existing.cost += cost;
    existing.value += value;
    totals.set(key, existing);
  }

  const series = [...totals.entries()]
    .sort(
      (a, b) =>
        b[1].value - a[1].value ||
        b[1].tokens - a[1].tokens ||
        b[1].requests - a[1].requests,
    )
    .slice(0, ANALYTICS_CHART_COLORS.length)
    .map(([key, value], index) => ({
      key,
      label: value.label,
      color: analyticsGroupColor(value.label, dimensions, index),
    }));
  return { data: rows, series };
}

export type AnalyticsBreakdownRow = {
  id: string;
  name: string;
  color: string;
  value: number;
  tokens: number;
  requests: number;
  cost: number;
};

export function analyticsBreakdownData(
  usage: AnalyticsGroup[],
  dimensions: AnalyticsDimension[],
  measure: AnalyticsMeasure,
): AnalyticsBreakdownRow[] {
  const merged = new Map<
    string,
    { name: string; tokens: number; requests: number; cost: number }
  >();
  for (const item of usage) {
    const values = Object.fromEntries(
      dimensions.map((dimension) => [
        dimension,
        analyticsGroupValue(item.group, dimension),
      ]),
    );
    if (dimensions.length > 0 && !Object.values(values).some(Boolean)) continue;
    const name = analyticsDisplayName(values);
    const current = merged.get(name) ?? {
      name,
      tokens: 0,
      requests: 0,
      cost: 0,
    };
    current.tokens += item.totalTokens;
    current.requests += item.requests;
    current.cost += analyticsUsageCost(item);
    merged.set(name, current);
  }
  return [...merged.values()]
    .map((item) => ({ ...item, value: analyticsMeasureValue(item, measure) }))
    .filter((item) => item.value > 0 || item.tokens > 0 || item.requests > 0)
    .sort(
      (a, b) =>
        b.value - a.value || b.tokens - a.tokens || b.requests - a.requests,
    )
    .map((item, index) => ({
      ...item,
      id: `breakdown_${stableSeriesKey(item.name)}`,
      color: analyticsGroupColor(item.name, dimensions, index),
    }))
    .slice(0, 30);
}

function analyticsAxisTicks(data: AnalyticsTimelineRow[]) {
  if (data.length <= 8) return data.map((item) => item.startMs);
  const tickCount = 7;
  const indexes = new Set<number>();
  for (let index = 0; index < tickCount; index += 1) {
    indexes.add(Math.round((index * (data.length - 1)) / (tickCount - 1)));
  }
  return [...indexes].sort((a, b) => a - b).map((index) => data[index].startMs);
}

function formatTimelineAxisTick(value: number, data: AnalyticsTimelineRow[]) {
  return data.find((item) => item.startMs === value)?.name ?? "";
}

function analyticsGroupValue(
  group: Record<string, unknown>,
  dimension: AnalyticsDimension,
) {
  if (dimension === "model")
    return stringFromUnknown(group.requestModel ?? group.model);
  if (dimension === "provider")
    return stringFromUnknown(group.provider ?? group.providerName);
  if (dimension === "userAgent") {
    return stringFromUnknown(
      group["user_agent.name"] ??
        group.userAgent ??
        group["attributes.userAgent"] ??
        group["attributes:userAgent"] ??
        nestedAttribute(group, "user_agent.name") ??
        nestedAttribute(group, "userAgent"),
    );
  }
  if (dimension === "group") {
    return stringFromUnknown(
      group["agentgateway.group"] ??
        group.group ??
        nestedAttribute(group, "agentgateway.group") ??
        nestedAttribute(group, "group"),
    );
  }
  return stringFromUnknown(
    group["agentgateway.user"] ??
      group.user ??
      group["attributes.user"] ??
      group["attributes:user"] ??
      nestedAttribute(group, "agentgateway.user") ??
      nestedAttribute(group, "user"),
  );
}

function analyticsDisplayName(
  values: Partial<Record<AnalyticsDimension, string>>,
) {
  if (!Object.values(values).some(Boolean)) return "Total";
  const model = values.model || "";
  const provider = values.provider || "";
  const user = values.user || "";
  const group = values.group || "";
  const userAgent = values.userAgent || "";
  const base = model && provider ? `${provider}/${model}` : model || provider;
  const suffix = [group, user, userAgent].filter(Boolean).join(" · ");
  if (base && suffix) return `${base} · ${suffix}`;
  return base || suffix || "unknown";
}

function providerColor(provider: string | undefined, fallbackIndex = 0) {
  if (!provider)
    return ANALYTICS_CHART_COLORS[
      fallbackIndex % ANALYTICS_CHART_COLORS.length
    ];
  return (
    PROVIDER_COLORS[provider.toLowerCase()] ??
    ANALYTICS_CHART_COLORS[fallbackIndex % ANALYTICS_CHART_COLORS.length]
  );
}

function analyticsGroupColor(
  label: string,
  dimensions: AnalyticsDimension[],
  index: number,
) {
  if (dimensions.length === 1 && dimensions[0] === "provider")
    return providerColor(label, index);
  return ANALYTICS_CHART_COLORS[index % ANALYTICS_CHART_COLORS.length];
}

function truncateBreakdownLabel(value: unknown) {
  const text = String(value ?? "");
  return text.length > 30 ? `${text.slice(0, 29)}…` : text;
}

function formatAxisNumber(
  value: unknown,
  measure: AnalyticsMeasure = "tokens",
) {
  const number = Number(value);
  if (!Number.isFinite(number)) return "";
  if (measure === "cost") return formatAxisCost(number);
  if (Math.abs(number) >= 1_000_000)
    return `${(number / 1_000_000).toFixed(number >= 10_000_000 ? 0 : 1)}M`;
  if (Math.abs(number) >= 1_000)
    return `${(number / 1_000).toFixed(number >= 10_000 ? 0 : 1)}k`;
  return formatNumber(number);
}

function costAxisMax(dataMax: unknown) {
  const max = Number(dataMax);
  if (!Number.isFinite(max) || max <= 0) return 1;
  if (max >= 1) return Math.ceil(max);
  return Math.ceil(max * 100) / 100;
}

function formatAxisCost(value: number) {
  if (Math.abs(value) >= 1 && Number.isInteger(value))
    return `$${value.toFixed(0)}`;
  return `$${value.toFixed(2)}`;
}

function measureLabel(measure: AnalyticsMeasure) {
  if (measure === "requests") return "Requests";
  if (measure === "cost") return "Cost";
  return "Tokens";
}

function formatMeasureValue(value: number, measure: AnalyticsMeasure) {
  if (measure === "cost") return `$${value.toFixed(2)}`;
  return formatNumber(value);
}

function analyticsMeasureValue(
  value: { tokens: number; requests: number; cost: number },
  measure: AnalyticsMeasure,
) {
  if (measure === "requests") return value.requests;
  if (measure === "cost") return value.cost;
  return value.tokens;
}

function analyticsBucketCost(bucket: AnalyticsTimeBucket) {
  return numberFromRecord(bucket, [
    "cost",
    "totalCost",
    "estimatedCost",
    "usdCost",
  ]);
}

function analyticsUsageCost(item: AnalyticsGroup) {
  return numberFromRecord(item, [
    "cost",
    "totalCost",
    "estimatedCost",
    "usdCost",
  ]);
}

function numberFromRecord(value: unknown, keys: string[]) {
  if (!value || typeof value !== "object") return 0;
  const record = value as Record<string, unknown>;
  for (const key of keys) {
    const number = Number(record[key]);
    if (Number.isFinite(number)) return number;
  }
  return 0;
}

function stableSeriesKey(value: string) {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash.toString(36);
}

function nestedAttribute(record: Record<string, unknown>, key: string) {
  if (key in record) return record[key];
  const metadata = record.metadata;
  if (metadata && typeof metadata === "object" && key in metadata)
    return (metadata as Record<string, unknown>)[key];
  const apiKey = record.apiKey;
  if (apiKey && typeof apiKey === "object" && key in apiKey)
    return (apiKey as Record<string, unknown>)[key];
  return undefined;
}

function stringFromUnknown(value: unknown) {
  if (typeof value === "string" && value.trim()) return value.trim();
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return "";
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function sortedUnique(values: string[]) {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))].sort(
    (a, b) => a.localeCompare(b),
  );
}
