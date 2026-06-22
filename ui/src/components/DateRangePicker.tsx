import { CalendarDays, ChevronDown } from "lucide-react";
import { useEffect, useState } from "react";
import { FieldGroup, useDismissiblePopover } from "./Primitives";
import type { TimeRange } from "../types";

export type PresetRange = "1h" | "12h" | "24h" | "7d" | "14d" | "30d";
export type LogTimeRange =
  | { mode: "preset"; preset: PresetRange }
  | { mode: "absolute"; fromLocal: string; toLocal: string };

export const RANGE_PRESETS: Array<{
  value: PresetRange;
  label: string;
  ms: number;
}> = [
  { value: "1h", label: "Last 1 hour", ms: 60 * 60 * 1000 },
  { value: "12h", label: "Last 12 hours", ms: 12 * 60 * 60 * 1000 },
  { value: "24h", label: "Last 24 hours", ms: 24 * 60 * 60 * 1000 },
  { value: "7d", label: "Last 7 days", ms: 7 * 24 * 60 * 60 * 1000 },
  { value: "14d", label: "Last 14 days", ms: 14 * 24 * 60 * 60 * 1000 },
  { value: "30d", label: "Last 30 days", ms: 30 * 24 * 60 * 60 * 1000 },
];

export function DateRangePicker(props: {
  value: LogTimeRange;
  onChange: (value: LogTimeRange) => void;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<LogTimeRange>(props.value);
  const ref = useDismissiblePopover<HTMLDivElement>(open, () => setOpen(false));

  useEffect(() => {
    if (open) setDraft(props.value);
  }, [open, props.value]);

  const absoluteDraft =
    draft.mode === "absolute" ? draft : presetAbsoluteRange(draft.preset);
  return (
    <div className="date-range-picker" ref={ref}>
      <button
        className="button date-range-trigger"
        type="button"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
      >
        <CalendarDays size={16} />
        {logTimeRangeLabel(props.value)}
        <ChevronDown size={15} />
      </button>
      {open ? (
        <div className="date-range-popover">
          <div className="date-range-presets" aria-label="Quick ranges">
            {RANGE_PRESETS.map((preset) => (
              <button
                className={
                  draft.mode === "preset" && draft.preset === preset.value
                    ? "active"
                    : ""
                }
                type="button"
                key={preset.value}
                onClick={() =>
                  setDraft({ mode: "preset", preset: preset.value })
                }
              >
                {preset.label}
              </button>
            ))}
          </div>
          <div className="date-range-custom">
            <div className="date-range-field-grid">
              <FieldGroup label="From">
                <input
                  type="datetime-local"
                  value={absoluteDraft.fromLocal}
                  onChange={(event) =>
                    setDraft({
                      mode: "absolute",
                      fromLocal: event.target.value,
                      toLocal: absoluteDraft.toLocal,
                    })
                  }
                />
              </FieldGroup>
              <FieldGroup label="To">
                <input
                  type="datetime-local"
                  value={absoluteDraft.toLocal}
                  onChange={(event) =>
                    setDraft({
                      mode: "absolute",
                      fromLocal: absoluteDraft.fromLocal,
                      toLocal: event.target.value,
                    })
                  }
                />
              </FieldGroup>
            </div>
            <FieldGroup label="Interval">
              <input
                value={formatBucketDuration(bucketSecondsForRange(draft))}
                disabled
                readOnly
              />
            </FieldGroup>
            <div className="date-range-footer">
              <span>{timezoneLabel()}</span>
              <div className="button-row compact">
                <button
                  className="button secondary"
                  type="button"
                  onClick={() => setOpen(false)}
                >
                  Cancel
                </button>
                <button
                  className="button primary"
                  type="button"
                  disabled={!isValidLogTimeRange(draft)}
                  onClick={() => {
                    props.onChange(draft);
                    setOpen(false);
                  }}
                >
                  Apply
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function isPresetRange(value: string): value is PresetRange {
  return RANGE_PRESETS.some((preset) => preset.value === value);
}

export function logTimeRangeToApi(value: LogTimeRange): TimeRange {
  if (value.mode === "absolute") {
    const from = dateFromLocal(value.fromLocal);
    const to = dateFromLocal(value.toLocal);
    if (!from || !to) return {};
    return {
      from: from.toISOString(),
      to: to.toISOString(),
    };
  }
  const preset =
    RANGE_PRESETS.find((item) => item.value === value.preset) ??
    defaultPreset();
  const now = ceilDateToBucket(new Date(), bucketSecondsForRange(value));
  return {
    from: new Date(now.getTime() - preset.ms).toISOString(),
    to: now.toISOString(),
  };
}

export function logTimeRangeLabel(value: LogTimeRange) {
  if (value.mode === "preset") {
    return (
      RANGE_PRESETS.find((item) => item.value === value.preset)?.label ??
      "Last 24 hours"
    );
  }
  return `${formatShortDateTime(new Date(value.fromLocal))} - ${formatShortDateTime(new Date(value.toLocal))}`;
}

export function bucketCountForRange(value: LogTimeRange) {
  const rangeMs = rangeMsForLogTimeRange(value);
  return Math.max(
    1,
    Math.ceil(rangeMs / (bucketSecondsForRange(value) * 1000)),
  );
}

export function bucketSecondsForRange(value: LogTimeRange) {
  if (value.mode === "preset") {
    if (value.preset === "1h") return 60;
    if (value.preset === "12h") return 10 * 60;
    if (value.preset === "24h" || value.preset === "7d") return 60 * 60;
    return 24 * 60 * 60;
  }
  const rangeMs = rangeMsForLogTimeRange(value);
  if (rangeMs <= 60 * 60 * 1000) return 60;
  if (rangeMs <= 12 * 60 * 60 * 1000) return 10 * 60;
  if (rangeMs <= 7 * 24 * 60 * 60 * 1000) return 60 * 60;
  return 24 * 60 * 60;
}

export function formatBucketDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "Auto";
  if (seconds < 60)
    return `${Math.round(seconds)} ${Math.round(seconds) === 1 ? "second" : "seconds"}`;
  const minutes = seconds / 60;
  if (minutes < 60) {
    const rounded = Math.round(minutes);
    return `${rounded} ${rounded === 1 ? "minute" : "minutes"}`;
  }
  const hours = minutes / 60;
  if (hours < 24) {
    const rounded = Math.round(hours);
    return `${rounded} ${rounded === 1 ? "hour" : "hours"}`;
  }
  const days = hours / 24;
  const rounded = Math.round(days);
  return `${rounded} ${rounded === 1 ? "day" : "days"}`;
}

function isValidLogTimeRange(value: LogTimeRange) {
  if (value.mode === "preset") return true;
  const from = dateFromLocal(value.fromLocal)?.getTime();
  const to = dateFromLocal(value.toLocal)?.getTime();
  return from !== undefined && to !== undefined && from < to;
}

function presetAbsoluteRange(
  presetValue: PresetRange,
): Extract<LogTimeRange, { mode: "absolute" }> {
  const preset =
    RANGE_PRESETS.find((item) => item.value === presetValue) ?? defaultPreset();
  const now = ceilDateToBucket(
    new Date(),
    bucketSecondsForRange({ mode: "preset", preset: preset.value }),
  );
  return {
    mode: "absolute",
    fromLocal: toDateTimeLocal(new Date(now.getTime() - preset.ms)),
    toLocal: toDateTimeLocal(now),
  };
}

function defaultPreset() {
  return RANGE_PRESETS.find((item) => item.value === "24h") ?? RANGE_PRESETS[0];
}

function rangeMsForLogTimeRange(value: LogTimeRange) {
  if (value.mode === "absolute") {
    const from = dateFromLocal(value.fromLocal)?.getTime();
    const to = dateFromLocal(value.toLocal)?.getTime();
    return from !== undefined && to !== undefined && to > from
      ? to - from
      : 24 * 60 * 60 * 1000;
  }
  const range = logTimeRangeToApi(value);
  const from = range.from
    ? new Date(range.from).getTime()
    : Date.now() - 24 * 60 * 60 * 1000;
  const to = range.to ? new Date(range.to).getTime() : Date.now();
  return Number.isFinite(from) && Number.isFinite(to) && to > from
    ? to - from
    : 24 * 60 * 60 * 1000;
}

function dateFromLocal(value: string) {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date : null;
}

function ceilDateToBucket(date: Date, bucketSeconds: number) {
  const bucketMs = Math.max(1, bucketSeconds * 1000);
  return new Date(Math.ceil(date.getTime() / bucketMs) * bucketMs);
}

export function toDateTimeLocal(date: Date) {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function formatShortDateTime(date: Date) {
  if (!Number.isFinite(date.getTime())) return "Invalid date";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

function timezoneLabel() {
  return Intl.DateTimeFormat().resolvedOptions().timeZone.replaceAll("_", " ");
}
