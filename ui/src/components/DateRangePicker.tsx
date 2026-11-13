import { DatePicker, type GetRef } from 'antd';
import dayjs, { type Dayjs } from 'dayjs';
import { CalendarDays, ChevronDown } from 'lucide-react';
import { useEffect, useRef } from 'react';

import type { TimeRange } from '@/types';

const { RangePicker } = DatePicker;

export type PresetRange = '1h' | '12h' | '24h' | '7d' | '14d' | '30d';
export type LogTimeRange =
	| { mode: 'preset'; preset: PresetRange }
	| { mode: 'absolute'; fromLocal: string; toLocal: string };

export const RANGE_PRESETS: Array<{
	value: PresetRange;
	label: string;
	ms: number;
}> = [
	{ value: '1h', label: 'Last 1 hour', ms: 60 * 60 * 1000 },
	{ value: '12h', label: 'Last 12 hours', ms: 12 * 60 * 60 * 1000 },
	{ value: '24h', label: 'Last 24 hours', ms: 24 * 60 * 60 * 1000 },
	{ value: '7d', label: 'Last 7 days', ms: 7 * 24 * 60 * 60 * 1000 },
	{ value: '14d', label: 'Last 14 days', ms: 14 * 24 * 60 * 60 * 1000 },
	{ value: '30d', label: 'Last 30 days', ms: 30 * 24 * 60 * 60 * 1000 }
];

export function DateRangePicker(props: {
	value: LogTimeRange;
	onChange: (value: LogTimeRange) => void;
}) {
	const absolute =
		props.value.mode === 'absolute' ? props.value : presetAbsoluteRange(props.value.preset);
	const from = dayjs(absolute.fromLocal);
	const to = dayjs(absolute.toLocal);
	const presetLabel = props.value.mode === 'preset' ? logTimeRangeLabel(props.value) : undefined;
	const pickerRef = useRef<GetRef<typeof RangePicker>>(null);

	// A preset hides the end input; keep it out of the tab order while it's hidden.
	useEffect(() => {
		const end = pickerRef.current?.nativeElement?.querySelector<HTMLInputElement>(
			'.ant-picker-input-end input'
		);
		if (end) end.tabIndex = presetLabel ? -1 : 0;
	}, [presetLabel]);

	return (
		<RangePicker
			ref={pickerRef}
			className={presetLabel ? 'date-range-picker is-preset' : 'date-range-picker'}
			aria-label="Time range"
			suffixIcon={<ChevronDown size={15} />}
			value={[from, to]}
			showTime={{ format: 'HH:mm' }}
			allowClear={false}
			format={
				presetLabel ? (date: Dayjs) => (date.isSame(to) ? 'Now' : presetLabel) : 'MMM D, HH:mm'
			}
			prefix={<CalendarDays size={16} />}
			presets={RANGE_PRESETS.map(preset => ({
				label: preset.label,
				value: () => {
					const range = presetAbsoluteRange(preset.value);
					return [dayjs(range.fromLocal), dayjs(range.toLocal)];
				}
			}))}
			renderExtraFooter={() => (
				<span className="date-range-footer">
					Interval: {formatBucketDuration(bucketSecondsForRange(props.value))} · {timezoneLabel()}
				</span>
			)}
			onChange={dates => {
				if (!dates?.[0] || !dates[1]) return;
				const fromLocal = toDateTimeLocal(dates[0].toDate());
				const toLocal = toDateTimeLocal(dates[1].toDate());
				// antd submits a preset as plain dates; keep it rolling when they match one exactly.
				const preset = RANGE_PRESETS.find(item => {
					const range = presetAbsoluteRange(item.value);
					return range.fromLocal === fromLocal && range.toLocal === toLocal;
				});
				if (preset) {
					props.onChange({ mode: 'preset', preset: preset.value });
					return;
				}
				props.onChange({
					mode: 'absolute',
					fromLocal,
					toLocal
				});
			}}
		/>
	);
}

export function isPresetRange(value: string): value is PresetRange {
	return RANGE_PRESETS.some(preset => preset.value === value);
}

export function logTimeRangeToApi(value: LogTimeRange): TimeRange {
	if (value.mode === 'absolute') {
		const from = dateFromLocal(value.fromLocal);
		const to = dateFromLocal(value.toLocal);
		if (!from || !to) return {};
		return {
			from: from.toISOString(),
			to: to.toISOString()
		};
	}
	const preset = RANGE_PRESETS.find(item => item.value === value.preset) ?? defaultPreset();
	const now = ceilDateToBucket(new Date(), bucketSecondsForRange(value));
	return {
		from: new Date(now.getTime() - preset.ms).toISOString(),
		to: now.toISOString()
	};
}

export function logTimeRangeLabel(value: LogTimeRange) {
	if (value.mode === 'preset') {
		return RANGE_PRESETS.find(item => item.value === value.preset)?.label ?? 'Last 24 hours';
	}
	return `${formatShortDateTime(new Date(value.fromLocal))} - ${formatShortDateTime(new Date(value.toLocal))}`;
}

export function bucketCountForRange(value: LogTimeRange) {
	const rangeMs = rangeMsForLogTimeRange(value);
	return Math.max(1, Math.ceil(rangeMs / (bucketSecondsForRange(value) * 1000)));
}

export function bucketSecondsForRange(value: LogTimeRange) {
	if (value.mode === 'preset') {
		if (value.preset === '1h') return 60;
		if (value.preset === '12h') return 10 * 60;
		if (value.preset === '24h' || value.preset === '7d') return 60 * 60;
		return 24 * 60 * 60;
	}
	const rangeMs = rangeMsForLogTimeRange(value);
	if (rangeMs <= 60 * 60 * 1000) return 60;
	if (rangeMs <= 12 * 60 * 60 * 1000) return 10 * 60;
	if (rangeMs <= 7 * 24 * 60 * 60 * 1000) return 60 * 60;
	return 24 * 60 * 60;
}

export function formatBucketDuration(seconds: number) {
	if (!Number.isFinite(seconds) || seconds <= 0) return 'Auto';
	if (seconds < 60)
		return `${Math.round(seconds)} ${Math.round(seconds) === 1 ? 'second' : 'seconds'}`;
	const minutes = seconds / 60;
	if (minutes < 60) {
		const rounded = Math.round(minutes);
		return `${rounded} ${rounded === 1 ? 'minute' : 'minutes'}`;
	}
	const hours = minutes / 60;
	if (hours < 24) {
		const rounded = Math.round(hours);
		return `${rounded} ${rounded === 1 ? 'hour' : 'hours'}`;
	}
	const days = hours / 24;
	const rounded = Math.round(days);
	return `${rounded} ${rounded === 1 ? 'day' : 'days'}`;
}

function presetAbsoluteRange(
	presetValue: PresetRange
): Extract<LogTimeRange, { mode: 'absolute' }> {
	const preset = RANGE_PRESETS.find(item => item.value === presetValue) ?? defaultPreset();
	const now = ceilDateToBucket(
		new Date(),
		bucketSecondsForRange({ mode: 'preset', preset: preset.value })
	);
	return {
		mode: 'absolute',
		fromLocal: toDateTimeLocal(new Date(now.getTime() - preset.ms)),
		toLocal: toDateTimeLocal(now)
	};
}

function defaultPreset() {
	return RANGE_PRESETS.find(item => item.value === '24h') ?? RANGE_PRESETS[0];
}

function rangeMsForLogTimeRange(value: LogTimeRange) {
	if (value.mode === 'absolute') {
		const from = dateFromLocal(value.fromLocal)?.getTime();
		const to = dateFromLocal(value.toLocal)?.getTime();
		return from !== undefined && to !== undefined && to > from ? to - from : 24 * 60 * 60 * 1000;
	}
	const range = logTimeRangeToApi(value);
	const from = range.from ? new Date(range.from).getTime() : Date.now() - 24 * 60 * 60 * 1000;
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
	const pad = (value: number) => String(value).padStart(2, '0');
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function formatShortDateTime(date: Date) {
	if (!Number.isFinite(date.getTime())) return 'Invalid date';
	return new Intl.DateTimeFormat(undefined, {
		month: 'short',
		day: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	}).format(date);
}

function timezoneLabel() {
	return Intl.DateTimeFormat().resolvedOptions().timeZone.replaceAll('_', ' ');
}
