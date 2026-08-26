import { useNavigate } from '@tanstack/react-router';
import DOMPurify from 'dompurify';
import {
	ArrowRight,
	Braces,
	Check,
	ChevronDown,
	ChevronRight,
	Copy,
	Download,
	Ellipsis,
	MessageSquare,
	RefreshCw,
	Settings,
	User,
	Wrench
} from 'lucide-react';
import { marked } from 'marked';
import { useEffect, useMemo, useRef, useState } from 'react';

import { analyticsSummary, getLog, searchLogs, streamLogs } from '@/api/logsApi';
import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
import {
	bucketCountForRange,
	bucketSecondsForRange,
	DateRangePicker,
	isPresetRange,
	type LogTimeRange,
	logTimeRangeLabel,
	logTimeRangeToApi,
	toDateTimeLocal
} from '@/components/DateRangePicker';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { MultiCheckboxDropdown } from '@/components/MultiCheckboxDropdown';
import {
	Drawer,
	EmptyState,
	FieldGroup,
	formatDate,
	formatNumber,
	formatRelativeTime,
	JsonBlock,
	PageHeader,
	Panel,
	StatusBanner,
	Tooltip,
	useDismissiblePopover
} from '@/components/Primitives';
import { ProviderIcon } from '@/components/ProviderIcon';
import {
	promptCompletionLoggingEnabled,
	providerDisplayName,
	setPromptCompletionLogging,
	setUiLogAttributeExpressions,
	uiLogAttributeExpressions
} from '@/config';
import { queryParam, useStickyQueryParam } from '@/drawerRouteState';
import { useLlmConfigData, useUpdateConfig } from '@/hooks';
import { llmModelOptions } from '@/llmModelOptions';
import {
	ANALYTICS_DIMENSIONS,
	AnalyticsBreakdownChart,
	type AnalyticsDimension,
	type AnalyticsMeasure,
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
	mergeAnalyticsFilterOptions
} from '@/pages/analytics/AnalyticsCharts';
import type {
	AnalyticsGroup,
	AnalyticsTimeBucket,
	GatewayConfig,
	LogEntry,
	SearchLogsResponse,
	TimeRange
} from '@/types';

export function LogsPage() {
	const navigate = useNavigate({ from: '/llm/logs' });
	const {
		config,
		models: configuredModels,
		virtualModels,
		providers,
		isLoading: configDataLoading,
		error: configDataError
	} = useLlmConfigData();
	const updateConfig = useUpdateConfig();
	const models = useMemo(
		() =>
			llmModelOptions({
				models: configuredModels,
				virtualModels,
				providers
			}),
		[configuredModels, providers, virtualModels]
	);
	const promptLoggingEnabled = promptCompletionLoggingEnabled(config.data);
	const [settings, setSettings] = useStickyQueryParam('settings');
	const [linkedLogId, setLinkedLogId] = useState(() => queryParam('log'));
	const [logFilters, setLogFilters] = useState<Record<AnalyticsDimension, string[]>>(
		emptyAnalyticsFilterOptions
	);
	const [filterOptionMap, setFilterOptionMap] = useState<Record<AnalyticsDimension, string[]>>(
		emptyAnalyticsFilterOptions
	);
	const [status, setStatus] = useState('');
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
			httpStatus: status ? [Number(status)] : []
		}),
		[logFiltersKey, status]
	);
	const visibleLogs = useMemo(() => {
		if (!expanded || !expandedId || response.logs.some(entry => entry.id === expandedId))
			return response.logs;
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
				includeAttributes: true
			});
			if (loadSeq !== loadSeqRef.current) return;
			setResponse(logs);
		} catch (err) {
			if (loadSeq !== loadSeqRef.current) return;
			setError(err instanceof Error ? err.message : 'Failed to load logs');
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
					bucketCount: 1
				});
				if (loadSeq !== filterOptionsSeqRef.current) return;
				const discoveredOptions =
					analyticsFilterOptionsFromResponse(analytics.filterOptions) ??
					analyticsFilterOptions(analytics.groups);
				setFilterOptionMap(current =>
					mergeAnalyticsFilterOptions(current, discoveredOptions, logFilters)
				);
			} catch {
				if (loadSeq !== filterOptionsSeqRef.current) return;
				setFilterOptionMap(current =>
					mergeAnalyticsFilterOptions(
						current,
						{
							...emptyAnalyticsFilterOptions(),
							model: models.map(item => item.name)
						},
						logFilters
					)
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
				for await (const event of streamLogs({ limit: 100, filters }, controller.signal)) {
					setResponse(current => ({
						...current,
						logs: [event.entry, ...current.logs].slice(0, 200)
					}));
				}
			} catch (err) {
				if (!controller.signal.aborted)
					setError(err instanceof Error ? err.message : 'Log stream failed');
			}
		})();
		return () => controller.abort();
	}, [stream, filters]);

	useEffect(() => {
		function syncLinkedLogId() {
			setLinkedLogId(queryParam('log'));
		}
		window.addEventListener('popstate', syncLinkedLogId);
		return () => window.removeEventListener('popstate', syncLinkedLogId);
	}, []);

	function setLinkedLogQuery(next: string | null) {
		setLinkedLogId(next);
		void navigate({
			to: '/llm/logs',
			replace: true,
			resetScroll: false,
			search: previous => {
				const search = { ...(previous as Record<string, unknown>) };
				if (next) search.log = next;
				else delete search.log;
				return search;
			}
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
		const fallback = response.logs.find(entry => entry.id === linkedLogId);
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
			setExpanded(nextLog);
		} catch (err) {
			if (detailSeq !== detailSeqRef.current) return;
			setError(err instanceof Error ? err.message : 'Failed to load log detail');
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
						<button className="button" type="button" onClick={() => setSettings('logs')}>
							<Settings size={16} />
							Settings
						</button>
						<button className="button" type="button" disabled={loading} onClick={load}>
							<RefreshCw size={16} className={loading ? 'spin' : undefined} />
							Refresh
						</button>
					</div>
				}
			/>
			{configDataLoading ? (
				<StatusBanner state="loading" title="Loading LLM configuration" />
			) : configDataError ? (
				<StatusBanner state="bad" title="Configuration API unavailable">
					{configDataError.message}
				</StatusBanner>
			) : null}
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
					{ANALYTICS_DIMENSIONS.map(meta => {
						const dimension = meta.value;
						return (
							<MultiCheckboxDropdown
								kind="filter"
								key={dimension}
								label={meta.filterLabel}
								options={filterOptionMap[dimension].map(value => ({
									value,
									label: value
								}))}
								values={logFilters[dimension]}
								placeholder={`All ${meta.filterLabel.toLowerCase()}`}
								allLabel={`All ${meta.filterLabel.toLowerCase()}`}
								onChange={values =>
									setLogFilters(current => ({
										...current,
										[dimension]: values
									}))
								}
							/>
						);
					})}
					<MultiCheckboxDropdown
						kind="filter"
						label="HTTP status"
						options={[
							{ value: '200', label: '200 OK' },
							{ value: '400', label: '400 Bad request' },
							{ value: '401', label: '401 Unauthorized' },
							{ value: '403', label: '403 Forbidden' },
							{ value: '404', label: '404 Not found' },
							{ value: '429', label: '429 Rate limited' },
							{ value: '500', label: '500 Server error' }
						]}
						values={status ? [status] : []}
						placeholder="Any status"
						allLabel="Any status"
						onChange={values => setStatus(values.at(-1) ?? '')}
					/>
					<label className="toggle-row logs-stream-toggle">
						<input
							type="checkbox"
							checked={stream}
							onChange={event => setStream(event.target.checked)}
						/>
						Stream
						{stream ? <span className="stream-live-dot" aria-label="streaming" /> : null}
					</label>
					{hasAnalyticsFilters(logFilters) || status ? (
						<button
							className="button"
							type="button"
							onClick={() => {
								setLogFilters(emptyAnalyticsFilterOptions());
								setStatus('');
							}}
						>
							Clear filters
						</button>
					) : null}
				</div>
			</Panel>

			<Panel className="logs-results-panel">
				{visibleLogs.length === 0 ? (
					<EmptyState
						title={loading ? 'Loading logs' : 'No log entries'}
						description={
							loading ? 'Fetching recent LLM calls.' : 'No LLM calls match the current filters.'
						}
					/>
				) : (
					<div className="log-table-wrap">
						<table className="log-table">
							<colgroup>
								<col className="col-time" />
								<col className="col-status" />
								<col className="col-model" />
								<col className="col-prompt" />
								<col className="col-turn" />
								<col className="col-duration" />
								<col className="col-in" />
								<col className="col-out" />
								<col className="col-cache" />
								<col className="col-cost" />
								<col className="col-chevron" />
							</colgroup>
							<thead>
								<tr>
									<th>Time</th>
									<th className="center">Status</th>
									<th>Model</th>
									<th>Prompt</th>
									<th>Turn</th>
									<th className="num">Duration</th>
									<th className="num">In</th>
									<th className="num">Out</th>
									<th className="num">Cache</th>
									<th className="num">Cost</th>
									<th aria-label="Expand" />
								</tr>
							</thead>
							{visibleLogs.map(entry => (
								<LogCallRow
									key={entry.id}
									entry={entry}
									detail={expandedId === entry.id ? (expanded ?? entry) : entry}
									expanded={expandedId === entry.id}
									loading={expandedId === entry.id && expandedLoading}
									onToggle={() => void expand(entry)}
									onOpenSettings={() => setSettings('logs')}
								/>
							))}
						</table>
						<div className="log-table-footer">
							{loading
								? 'Refreshing...'
								: `Showing ${formatNumber(visibleLogs.length)} recent calls`}
						</div>
					</div>
				)}
			</Panel>

			{settings === 'logs' ? (
				<LogsSettingsDrawer
					config={config.data}
					enabled={promptLoggingEnabled}
					attributes={uiLogAttributeExpressions(config.data)}
					saving={updateConfig.isPending}
					saveError={updateConfig.isError ? updateConfig.error.message : null}
					onClose={() => setSettings(null, 'replace')}
					onSave={values =>
						updateConfig.mutate(
							next => {
								setPromptCompletionLogging(next, values.enabled);
								setUiLogAttributeExpressions(next, values.attributes);
							},
							{
								onSuccess: () => setSettings(null, 'replace')
							}
						)
					}
				/>
			) : null}
		</div>
	);
}

function LogsSettingsDrawer(props: {
	config?: GatewayConfig | null;
	enabled: boolean;
	attributes: { user: string; group: string };
	saving: boolean;
	saveError?: string | null;
	onClose: () => void;
	onSave: (values: { enabled: boolean; attributes: { user: string; group: string } }) => void;
}) {
	const [enabled, setEnabled] = useState(props.enabled);
	const [userExpression, setUserExpression] = useState(props.attributes.user);
	const [groupExpression, setGroupExpression] = useState(props.attributes.group);
	const dirty =
		enabled !== props.enabled ||
		userExpression !== props.attributes.user ||
		groupExpression !== props.attributes.group;
	useEffect(() => setEnabled(props.enabled), [props.enabled]);
	useEffect(() => {
		setUserExpression(props.attributes.user);
		setGroupExpression(props.attributes.group);
	}, [props.attributes.group, props.attributes.user]);
	const values = {
		enabled,
		attributes: { user: userExpression, group: groupExpression }
	};
	return (
		<Drawer
			title="Log settings"
			onClose={props.onClose}
			dirty={dirty}
			saving={props.saving}
			footer={requestClose => (
				<ConfigDiffSaveActions
					config={props.config}
					diffTitle="Log settings config diff"
					saveLabel="Save settings"
					saving={props.saving}
					onCancel={requestClose}
					onSave={() => props.onSave(values)}
					applyDiff={next => {
						setPromptCompletionLogging(next, values.enabled);
						setUiLogAttributeExpressions(next, values.attributes);
					}}
				/>
			)}
		>
			<label className="config-option-row">
				<input
					type="checkbox"
					checked={enabled}
					onChange={event => setEnabled(event.target.checked)}
				/>
				<span>
					<strong>Include prompts and completions in logs</strong>
					<small>
						Stores prompt and completion content in the database payload. Metadata, usage, timing,
						and cost are always logged.
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
							Optional CEL expressions for populating user and group attributes in database logs. If
							not set a default will be used.
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

function hasAnalyticsFilters(filters: Record<AnalyticsDimension, string[]>) {
	return ANALYTICS_DIMENSIONS.some(item => filters[item.value].length > 0);
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
		(params.get('groupBy') ?? '')
			.split(',')
			.filter((value): value is AnalyticsDimension => isAnalyticsDimension(value))
	);
	return {
		timeRange: readAnalyticsTimeRange(params),
		groupBy,
		filters: {
			model: readRepeatedParam(params, 'model'),
			user: readRepeatedParam(params, 'user'),
			group: readRepeatedParam(params, 'group'),
			provider: readRepeatedParam(params, 'provider'),
			userAgent: readRepeatedParam(params, 'userAgent')
		},
		metric: readAnalyticsMetric(params)
	};
}

function writeAnalyticsUrlState(state: AnalyticsUrlState) {
	const params = new URLSearchParams();
	if (state.timeRange.mode === 'preset') {
		if (state.timeRange.preset !== '24h') params.set('range', state.timeRange.preset);
	} else {
		const from = new Date(state.timeRange.fromLocal);
		const to = new Date(state.timeRange.toLocal);
		if (Number.isFinite(from.getTime()) && Number.isFinite(to.getTime())) {
			params.set('from', from.toISOString());
			params.set('to', to.toISOString());
		}
	}
	if (state.groupBy.length > 0) params.set('groupBy', state.groupBy.join(','));
	if (state.metric !== 'tokens') params.set('metric', state.metric);
	for (const dimension of ANALYTICS_DIMENSIONS.map(item => item.value)) {
		for (const value of state.filters[dimension]) {
			params.append(dimension, value);
		}
	}
	const nextSearch = params.toString();
	const nextUrl = `${window.location.pathname}${nextSearch ? `?${nextSearch}` : ''}${window.location.hash}`;
	if (nextUrl !== `${window.location.pathname}${window.location.search}${window.location.hash}`) {
		window.history.replaceState(null, '', nextUrl);
	}
}

function readAnalyticsMetric(params: URLSearchParams): AnalyticsMetric {
	const value = params.get('metric');
	return value === 'cost' || value === 'requests' || value === 'tokens' ? value : 'tokens';
}

function readAnalyticsTimeRange(params: URLSearchParams): LogTimeRange {
	const range = params.get('range');
	if (range && isPresetRange(range)) return { mode: 'preset', preset: range };
	const from = params.get('from');
	const to = params.get('to');
	if (from && to) {
		const fromDate = new Date(from);
		const toDate = new Date(to);
		if (
			Number.isFinite(fromDate.getTime()) &&
			Number.isFinite(toDate.getTime()) &&
			fromDate < toDate
		) {
			return {
				mode: 'absolute',
				fromLocal: toDateTimeLocal(fromDate),
				toLocal: toDateTimeLocal(toDate)
			};
		}
	}
	return { mode: 'preset', preset: '24h' };
}

function readRepeatedParam(params: URLSearchParams, key: AnalyticsDimension) {
	return sortedUnique(
		params
			.getAll(key)
			.flatMap(value => value.split(','))
			.filter(Boolean)
	);
}

function uniqueAnalyticsDimensions(values: AnalyticsDimension[]) {
	return ANALYTICS_DIMENSIONS.map(item => item.value).filter(dimension =>
		values.includes(dimension)
	);
}

export function AnalyticsPage() {
	const initialUrlState = useMemo(readAnalyticsUrlState, []);
	const [timeRange, setTimeRange] = useState<LogTimeRange>(initialUrlState.timeRange);
	const [groupBy, setGroupBy] = useState<AnalyticsDimension[]>(initialUrlState.groupBy);
	const [filters, setFilters] = useState<Record<AnalyticsDimension, string[]>>(
		initialUrlState.filters
	);
	const [metric, setMetric] = useState<AnalyticsMetric>(initialUrlState.metric);
	const [buckets, setBuckets] = useState<AnalyticsTimeBucket[]>([]);
	const [summaryRange, setSummaryRange] = useState<TimeRange | null>(null);
	const [bucketSeconds, setBucketSeconds] = useState<number | null>(null);
	const [usage, setUsage] = useState<AnalyticsGroup[]>([]);
	const [filterOptionMap, setFilterOptionMap] = useState<Record<AnalyticsDimension, string[]>>(
		emptyAnalyticsFilterOptions
	);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const loadSeqRef = useRef(0);
	const selectedGroupBy: AnalyticsDimension[] = groupBy;
	const groupByKey = selectedGroupBy.join(',');
	const filtersKey = analyticsFiltersKey(filters);
	const analyticsState = useMemo(
		() => ({
			timeRange,
			groupBy: selectedGroupBy,
			filters,
			metric
		}),
		[timeRange, groupByKey, filtersKey, metric]
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
		window.addEventListener('popstate', onPopState);
		return () => window.removeEventListener('popstate', onPopState);
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
				bucketSeconds: bucketSecondsForRange(timeRange)
			});
			if (loadSeq !== loadSeqRef.current) return;
			setBuckets(analytics.buckets);
			setSummaryRange(analytics.timeRange);
			setBucketSeconds(analytics.bucketSeconds);
			setUsage(analytics.groups);
			const discoveredOptions =
				analyticsFilterOptionsFromResponse(analytics.filterOptions) ??
				analyticsFilterOptions(analytics.groups, selectedGroupBy);
			setFilterOptionMap(current =>
				mergeAnalyticsFilterOptions(current, discoveredOptions, filters)
			);
		} catch (err) {
			if (loadSeq !== loadSeqRef.current) return;
			setError(err instanceof Error ? err.message : 'Failed to load analytics');
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
			isAnalyticsDimension(value)
		);
		setGroupBy(normalized);
	}

	function updateFilter(dimension: AnalyticsDimension, values: string[]) {
		setFilters(current => ({ ...current, [dimension]: values }));
	}

	const requestedRange = useMemo(() => logTimeRangeToApi(timeRange), [timeRange]);
	const effectiveBucketSeconds = bucketSeconds ?? bucketSecondsForRange(timeRange);
	const timeline = useMemo(
		() =>
			analyticsTimelineData(
				buckets,
				summaryRange ?? requestedRange,
				effectiveBucketSeconds,
				selectedGroupBy,
				metric
			),
		[
			buckets,
			summaryRange?.from,
			summaryRange?.to,
			requestedRange.from,
			requestedRange.to,
			effectiveBucketSeconds,
			selectedGroupBy,
			metric
		]
	);
	const chartData = useMemo(
		() => analyticsBreakdownData(usage, selectedGroupBy, metric),
		[usage, selectedGroupBy, metric]
	);
	const totalTokens = usage.reduce((sum, item) => sum + item.totalTokens, 0);
	const totalRequests = usage.reduce((sum, item) => sum + item.requests, 0);
	const totalCost = usage.reduce((sum, item) => sum + analyticsRecordCost(item), 0);

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
							options={ANALYTICS_DIMENSIONS.map(dimension => ({
								value: dimension.value,
								label: dimension.label
							}))}
							values={selectedGroupBy}
							placeholder="Total"
							allLabel="Total"
							onChange={updateGroupBy}
						/>
						{ANALYTICS_DIMENSIONS.map(meta => {
							const dimension = meta.value;
							return (
								<MultiCheckboxDropdown
									kind="filter"
									key={dimension}
									label={meta.filterLabel}
									options={filterOptionMap[dimension].map(value => ({
										value,
										label: value
									}))}
									values={filters[dimension]}
									placeholder={`All ${meta.filterLabel.toLowerCase()}`}
									allLabel={`All ${meta.filterLabel.toLowerCase()}`}
									onChange={values => updateFilter(dimension, values)}
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
									{ value: 'tokens', label: 'Tokens' },
									{ value: 'cost', label: 'Cost' },
									{ value: 'requests', label: 'Requests' }
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
								? 'Loading analytics...'
								: `${formatCost(totalCost)} / ${formatNumber(totalTokens)} tokens / ${formatNumber(totalRequests)} calls`}
						</p>
					</div>
					<span className="muted-copy">{logTimeRangeLabel(timeRange)}</span>
				</div>
				<AnalyticsTimelineChart data={timeline.data} measure={metric} series={timeline.series} />
			</Panel>
			<Panel className="monitoring-activity-card">
				<div className="monitoring-card-header">
					<div>
						<h3>Breakdown</h3>
						<p>
							{selectedGroupBy.length
								? `${analyticsMetricLabel(metric)} by ${selectedGroupBy.map(dimension => ANALYTICS_DIMENSIONS.find(item => item.value === dimension)?.label.toLowerCase()).join(', ')}`
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
	return [...new Set(values.map(value => value.trim()).filter(Boolean))].sort((a, b) =>
		a.localeCompare(b)
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
				onClick={() => setOpen(current => !current)}
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

function downloadAnalyticsCsv(buckets: AnalyticsTimeBucket[], groupBy: AnalyticsDimension[]) {
	const headers = [
		'start',
		...groupBy.map(dimension => dimension),
		'requests',
		'total_tokens',
		'cost'
	];
	const rows = buckets.map(bucket => [
		bucket.start,
		...groupBy.map(dimension => analyticsCsvGroupValue(bucket.group, dimension)),
		bucket.requests,
		bucket.totalTokens,
		bucket.cost ?? ''
	]);
	const csv = [headers, ...rows].map(row => row.map(csvCell).join(',')).join('\n');
	const blob = new Blob([`${csv}\n`], { type: 'text/csv;charset=utf-8' });
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement('a');
	anchor.href = url;
	anchor.download = `agentgateway-analytics-${new Date().toISOString().slice(0, 10)}.csv`;
	anchor.click();
	URL.revokeObjectURL(url);
}

function analyticsCsvGroupValue(group: Record<string, unknown>, dimension: AnalyticsDimension) {
	if (dimension === 'model') return group.requestModel ?? group.model ?? '';
	if (dimension === 'provider') return group.provider ?? group.providerName ?? '';
	if (dimension === 'group') return group['agentgateway.group'] ?? group.group ?? '';
	if (dimension === 'userAgent') return group['user_agent.name'] ?? group.userAgent ?? '';
	return group['agentgateway.user'] ?? group.user ?? '';
}

function csvCell(value: unknown) {
	const text = value == null ? '' : String(value);
	return /[",\n\r]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function analyticsMetricLabel(metric: AnalyticsMetric) {
	if (metric === 'requests') return 'Requests';
	if (metric === 'cost') return 'Cost';
	return 'Tokens';
}

function formatCost(value: number, minimumFractionDigits = 0) {
	let formatted: string;
	if (value === 0) formatted = '$0.00';
	else if (value >= 10) formatted = `$${value.toFixed(2)}`;
	else if (value >= 0.01) formatted = `$${value.toFixed(4)}`;
	else {
		// Sub-cent: show 2 significant figures in fixed decimal notation. Converting
		// through a number would turn sufficiently small values back into `6e-7`.
		const exponent = Math.floor(Math.log10(Math.abs(value)));
		const fractionDigits = Math.min(100, Math.max(2, 1 - exponent));
		const fixed = value.toFixed(fractionDigits).replace(/0+$/, '').replace(/\.$/, '');
		formatted = `$${fixed}`;
	}

	const decimal = formatted.indexOf('.');
	const fractionDigits = decimal < 0 ? 0 : formatted.length - decimal - 1;
	if (fractionDigits >= minimumFractionDigits) return formatted;
	return `${formatted}${decimal < 0 ? '.' : ''}${'0'.repeat(
		minimumFractionDigits - fractionDigits
	)}`;
}

function costFractionDigits(value: number) {
	const formatted = formatCost(value);
	const decimal = formatted.indexOf('.');
	return decimal < 0 ? 0 : formatted.length - decimal - 1;
}

function analyticsRecordCost(value: unknown) {
	if (!value || typeof value !== 'object') return 0;
	const record = value as Record<string, unknown>;
	for (const key of ['cost', 'totalCost', 'estimatedCost', 'usdCost']) {
		const number = Number(record[key]);
		if (Number.isFinite(number)) return number;
	}
	return 0;
}

type RenderedLogMessagePart =
	| { type: 'text'; text: string }
	| { type: 'toolCall'; id?: string; name: string; arguments?: unknown }
	| { type: 'toolResult'; id?: string; name?: string; content?: unknown; isError?: boolean }
	| { type: 'reasoning'; content?: unknown };

type RenderedLogMessage = {
	role: 'system' | 'user' | 'assistant' | 'tool';
	content: string;
	name?: string;
	parts?: RenderedLogMessagePart[];
	toolCalls?: Array<{ id?: string; name: string; arguments?: unknown }>;
	toolResults?: Array<{
		id?: string;
		name?: string;
		content?: unknown;
		isError?: boolean;
	}>;
	reasoning?: unknown[];
};

function originalModelForLog(entry: LogEntry) {
	const attributes =
		entry.attributes && typeof entry.attributes === 'object'
			? (entry.attributes as Record<string, unknown>)
			: {};
	const value = attributes['agw.ai.original_model'] ?? attributes.originalModel;
	return typeof value === 'string' && value.trim() ? value : null;
}

function logIdentity(entry: LogEntry) {
	const attributes =
		entry.attributes && typeof entry.attributes === 'object'
			? (entry.attributes as Record<string, unknown>)
			: {};
	const record = entry as unknown as Record<string, unknown>;
	return {
		user: stringValue(record.user ?? attributes['agentgateway.user'] ?? attributes.user),
		group: stringValue(record.group ?? attributes['agentgateway.group'] ?? attributes.group)
	};
}

function stringValue(value: unknown) {
	return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function logMessagePreview(entry: LogEntry) {
	const prompt = payloadValue(entry, 'gen_ai.prompt', 'requestPrompt');
	const messages = messagesFromPrompt(prompt);
	const message =
		findLastMessage(messages, item => item.role === 'user' && Boolean(item.content.trim())) ??
		findLastMessage(messages, item => Boolean(item.content.trim()));
	const text = message?.content.trim() || entry.promptPreview?.trim() || entry.error || '';
	return text.length > 180 ? `${text.slice(0, 177)}...` : text;
}

function logTurn(entry: LogEntry) {
	const labels = {
		user: 'User',
		assistant: 'Assistant',
		toolCall: 'Tool call',
		toolResult: 'Tool result'
	};
	const input = entry.turn?.input ? labels[entry.turn.input] : null;
	const output = entry.turn?.output ? labels[entry.turn.output] : null;
	return [input, output].filter(Boolean).join(' → ');
}

function LogTurnBadge(props: { entry: LogEntry }) {
	const input = props.entry.turn?.input;
	const output = props.entry.turn?.output;
	const label = logTurn(props.entry) || 'Unknown turn';
	const variant = `${input ?? 'unknown'}-${output ?? 'unknown'}`;
	const common =
		(input === 'user' || input === 'toolResult') &&
		(output === 'assistant' || output === 'toolCall');
	return (
		<Tooltip content={label}>
			<span
				className={`log-turn-badge ${common ? variant : 'other'}`}
				aria-label={label}
				tabIndex={0}
			>
				{common ? (
					<>
						{input === 'user' ? <User size={13} /> : <Braces size={13} />}
						<ArrowRight size={10} />
						{output === 'assistant' ? <MessageSquare size={13} /> : <Wrench size={13} />}
					</>
				) : (
					<Ellipsis size={15} />
				)}
			</span>
		</Tooltip>
	);
}

function findLastMessage(
	messages: RenderedLogMessage[],
	predicate: (message: RenderedLogMessage) => boolean
) {
	for (let index = messages.length - 1; index >= 0; index -= 1) {
		if (predicate(messages[index])) return messages[index];
	}
	return null;
}

function formatDuration(ms: number | null | undefined) {
	if (ms == null || !Number.isFinite(ms)) return 'n/a';
	if (ms < 1000) return `${formatNumber(Math.round(ms))} ms`;
	if (ms < 10_000) return `${(ms / 1000).toFixed(2)} s`;
	if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
	return `${(ms / 60_000).toFixed(1)} min`;
}

const compactNumberFormat = new Intl.NumberFormat(undefined, {
	notation: 'compact',
	maximumFractionDigits: 1
});

function formatCompactNumber(value: number | null | undefined) {
	return typeof value === 'number' ? compactNumberFormat.format(value) : '—';
}

const PROVIDER_ICON_KEYS = [
	'openai',
	'anthropic',
	'gemini',
	'vertex',
	'bedrock',
	'azure',
	'copilot',
	'cohere',
	'ollama',
	'baseten',
	'cerebras',
	'deepinfra',
	'deepseek',
	'groq',
	'huggingface',
	'mistral',
	'openrouter',
	'togetherai',
	'xai',
	'fireworks'
];

function providerIconName(name: string | null | undefined) {
	if (!name) return 'custom';
	const normalized = name.trim().toLowerCase();
	const match = PROVIDER_ICON_KEYS.find(key => normalized === key || normalized.includes(key));
	if (!match) return name;
	return match === 'xai' ? 'xAI' : match;
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
	const statusBad = Boolean(props.entry.error || (props.entry.httpStatus ?? 0) >= 400);
	const preview = logMessagePreview(props.entry);
	const requestModel = props.entry.genAi.requestModel ?? null;
	const responseModel = props.entry.genAi.responseModel ?? null;
	const primaryModel = originalModel ?? requestModel ?? 'unknown model';
	const resolvedModel = responseModel ?? requestModel;
	const showResolved = Boolean(resolvedModel && resolvedModel !== primaryModel);
	const operation = props.entry.genAi.operationName ?? 'chat';
	const provider = props.entry.genAi.providerName ?? null;
	return (
		<tbody
			className={
				props.expanded
					? `log-row expanded ${statusBad ? 'bad' : 'ok'}`
					: `log-row ${statusBad ? 'bad' : 'ok'}`
			}
		>
			<tr
				className="log-row-summary"
				tabIndex={0}
				role="button"
				aria-expanded={props.expanded}
				onClick={props.onToggle}
				onKeyDown={event => {
					if (event.key === 'Enter' || event.key === ' ') {
						event.preventDefault();
						props.onToggle();
					}
				}}
			>
				<td className="log-td-time">
					<span className="log-cell-lines">
						<span className="log-cell-primary">{formatRelativeTime(props.entry.completedAt)}</span>
						<span className="log-cell-secondary">{formatDate(props.entry.completedAt)}</span>
					</span>
				</td>
				<td className="log-td-status">
					<span className={statusBad ? 'log-status-pill bad' : 'log-status-pill ok'}>
						{props.entry.httpStatus ?? 'err'}
					</span>
				</td>
				<td className="log-td-model">
					<span className="log-model-cell">
						<ProviderIcon provider={providerIconName(provider)} />
						<span className="log-cell-lines">
							<span className="log-cell-primary log-model-name">{primaryModel}</span>
							<span className="log-cell-secondary">
								{operation !== 'chat' ? <span className="log-op-chip">{operation}</span> : null}
								<span className="log-meta-item log-provider-label">
									{providerDisplayName(provider ?? 'unknown')}
								</span>
								{showResolved ? (
									<span className="log-meta-item log-model-resolved">
										<ArrowRight size={11} />
										{resolvedModel}
									</span>
								) : null}
							</span>
						</span>
					</span>
				</td>
				<td className="log-td-prompt">
					<span className={preview ? 'log-call-preview' : 'log-call-preview empty'}>
						{preview || '—'}
					</span>
				</td>
				<td className="log-td-turn">
					<LogTurnBadge entry={props.entry} />
				</td>
				<td className="log-td-num">{formatDuration(props.entry.durationMs)}</td>
				<TokenCells entry={props.entry} />
				<td className="log-td-num">
					<CostSummary entry={props.entry} />
				</td>
				<td className="log-td-chevron">
					<ChevronDown className="log-call-chevron" size={16} />
				</td>
			</tr>
			{props.expanded ? (
				<tr className="log-row-detail">
					<td colSpan={11}>
						<div className="expanded-log">
							{props.loading ? <StatusBanner state="loading" title="Loading log payload" /> : null}
							<LogDetailView entry={props.detail} onOpenSettings={props.onOpenSettings} />
						</div>
					</td>
				</tr>
			) : null}
		</tbody>
	);
}
type LogUsageDetail = {
	inputTokens?: number;
	outputTokens?: number;
	totalTokens?: number;
	cacheReadTokens?: number;
	cacheWriteTokens?: number;
	reasoningTokens?: number;
	inputAudioTokens?: number;
	outputAudioTokens?: number;
	timeToFirstTokenMs?: number;
	tokensPerSecond?: number;
	inputCost?: number;
	outputCost?: number;
	cacheReadCost?: number;
	cacheWriteCost?: number;
	reasoningCost?: number;
	inputAudioCost?: number;
	outputAudioCost?: number;
	totalCost?: number;
	costDigits: number;
};

function logUsageDetail(entry: LogEntry): LogUsageDetail {
	const a = entry.attributes;
	const inputTokens = entry.usage.inputTokens ?? attributeNumber(a, ['gen_ai.usage.input_tokens']);
	const outputTokens =
		entry.usage.outputTokens ?? attributeNumber(a, ['gen_ai.usage.output_tokens']);
	const totalTokens = entry.usage.totalTokens ?? attributeNumber(a, ['gen_ai.usage.total_tokens']);
	const timeToFirstToken = attributeNumber(a, ['agw.ai.time_to_first_token']);
	const timePerOutputToken = attributeNumber(a, ['agw.ai.time_per_output_token']);
	const costs = {
		inputCost: attributeNumber(a, ['agw.ai.usage.cost.input']),
		outputCost: attributeNumber(a, ['agw.ai.usage.cost.output']),
		cacheReadCost: attributeNumber(a, ['agw.ai.usage.cost.cacheRead']),
		cacheWriteCost: attributeNumber(a, ['agw.ai.usage.cost.cacheWrite']),
		reasoningCost: attributeNumber(a, ['agw.ai.usage.cost.reasoning']),
		inputAudioCost: attributeNumber(a, ['agw.ai.usage.cost.inputAudio']),
		outputAudioCost: attributeNumber(a, ['agw.ai.usage.cost.outputAudio']),
		totalCost: entry.cost ?? attributeNumber(a, ['agw.ai.usage.cost.total'])
	};
	const costDigits = Math.max(
		0,
		...Object.values(costs)
			.filter((cost): cost is number => cost != null && cost > 0)
			.map(costFractionDigits)
	);
	return {
		inputTokens,
		outputTokens,
		totalTokens,
		cacheReadTokens: attributeNumber(a, [
			'gen_ai.usage.cache_read.input_tokens',
			'cacheTokens',
			'cachedTokens',
			'cache_tokens',
			'cached_tokens'
		]),
		cacheWriteTokens: attributeNumber(a, ['gen_ai.usage.cache_creation.input_tokens']),
		reasoningTokens: attributeNumber(a, ['gen_ai.usage.reasoning_tokens']),
		inputAudioTokens: attributeNumber(a, ['gen_ai.usage.input_audio_tokens']),
		outputAudioTokens: attributeNumber(a, ['gen_ai.usage.output_audio_tokens']),
		timeToFirstTokenMs: timeToFirstToken != null ? timeToFirstToken * 1000 : undefined,
		tokensPerSecond:
			timePerOutputToken != null && timePerOutputToken > 0 ? 1 / timePerOutputToken : undefined,
		...costs,
		costDigits
	};
}

function LogDetailView(props: { entry: LogEntry; onOpenSettings?: () => void }) {
	const messages = logConversation(props.entry);
	const trajectory = trajectoryEvents(messages);
	const conversationRef = useRef<HTMLDetailsElement>(null);
	const usage = logUsageDetail(props.entry);
	const identity = logIdentity(props.entry);
	const provider = props.entry.genAi.providerName ?? null;
	const original = originalModelForLog(props.entry);
	const request = props.entry.genAi.requestModel ?? null;
	const response = props.entry.genAi.responseModel ?? null;
	const primaryModel = original ?? request ?? 'unknown model';
	const hasRouting =
		(original != null && request != null && original !== request) ||
		(response != null && request != null && response !== request);
	const userAgent = stringValue(attributeValue(props.entry.attributes, 'user_agent.name'));
	const trajectoryAnchors = trajectory.map(event => event.anchorId).join('|');

	function jumpToConversation(anchorId: string) {
		if (conversationRef.current) conversationRef.current.open = true;
		window.requestAnimationFrame(() => {
			document.getElementById(anchorId)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
			const hash = `#${anchorId}`;
			if (window.location.hash !== hash) window.history.pushState(null, '', hash);
		});
	}

	useEffect(() => {
		const anchorId = decodeURIComponent(window.location.hash.slice(1));
		if (!trajectory.some(event => event.anchorId === anchorId)) return;
		if (conversationRef.current) conversationRef.current.open = true;
		const frame = window.requestAnimationFrame(() => {
			document.getElementById(anchorId)?.scrollIntoView({ block: 'center' });
		});
		return () => window.cancelAnimationFrame(frame);
	}, [props.entry.id, trajectoryAnchors]);

	return (
		<div className="log-detail-view">
			<div className="log-detail-top">
				<div className="log-detail-title">
					<ProviderIcon provider={providerIconName(provider)} />
					<div className="log-detail-title-text">
						<h3>
							{providerDisplayName(provider ?? 'unknown')}
							<span className="log-detail-title-sep">/</span>
							<span className="log-detail-title-model">{primaryModel}</span>
						</h3>
						<div className="log-detail-id-row">
							<code>{props.entry.id}</code>
							<CopyButton value={props.entry.id} />
						</div>
					</div>
				</div>
				<button className="button" type="button" onClick={() => downloadJson(props.entry)}>
					<Download size={15} />
					JSON
				</button>
			</div>

			{props.entry.error ? (
				<StatusBanner state="bad" title="Gateway error">
					{props.entry.error}
				</StatusBanner>
			) : null}

			<div className="log-stat-row">
				<LogStat label="Duration" value={formatDuration(props.entry.durationMs)} />
				{usage.timeToFirstTokenMs != null ? (
					<LogStat label="First token" value={formatDuration(usage.timeToFirstTokenMs)} />
				) : null}
				{usage.tokensPerSecond != null ? (
					<LogStat
						label="Speed"
						value={`${formatNumber(Math.round(usage.tokensPerSecond))} tok/s`}
					/>
				) : null}
				<LogStat
					label="Tokens"
					value={usage.totalTokens != null ? formatNumber(usage.totalTokens) : 'n/a'}
				/>
				{usage.cacheReadTokens != null && usage.inputTokens != null && usage.inputTokens > 0 ? (
					<LogStat
						label="Cache hit"
						value={`${Math.min(
							Math.round((usage.cacheReadTokens / usage.inputTokens) * 100),
							100
						)}%`}
					/>
				) : null}
				<LogStat
					label="Cost"
					value={
						usage.totalCost != null && usage.totalCost > 0
							? formatCost(usage.totalCost, usage.costDigits)
							: 'n/a'
					}
				/>
			</div>

			{trajectory.length ? <LogTrajectory events={trajectory} onJump={jumpToConversation} /> : null}

			<div className="log-detail-grid">
				<section className="log-detail-section">
					<h4>Request</h4>
					{hasRouting ? (
						<div className="model-route">
							<ModelRouteStep label="Client requested" value={original ?? request ?? 'n/a'} />
							<ModelRouteStep label="Gateway sent" value={request ?? 'n/a'} />
							<ModelRouteStep label="Provider returned" value={response ?? 'n/a'} last />
						</div>
					) : null}
					<div className="log-field-grid">
						{!hasRouting ? <LogField label="Model" value={primaryModel} mono /> : null}
						<LogField label="Provider" value={providerDisplayName(provider ?? 'unknown')} />
						<LogField label="Operation" value={props.entry.genAi.operationName ?? 'unknown'} />
						<LogField label="HTTP status" value={String(props.entry.httpStatus ?? 'n/a')} />
						{userAgent ? <LogField label="Client" value={userAgent} /> : null}
						{identity.user ? <LogField label="User" value={identity.user} /> : null}
						{identity.group ? <LogField label="Group" value={identity.group} /> : null}
						<LogField label="Completed" value={formatDate(props.entry.completedAt)} />
					</div>
				</section>
				<section className="log-detail-section">
					<h4>Usage</h4>
					<LogUsagePanel usage={usage} />
				</section>
			</div>

			{messages.length ? (
				<details className="log-conversation" ref={conversationRef}>
					<summary>
						<ChevronRight className="log-conversation-chevron" size={15} />
						<span className="log-conversation-title">Conversation</span>
						<span className="log-conversation-count">
							{messages.length === 1 ? '1 message' : `${formatNumber(messages.length)} messages`}
						</span>
					</summary>
					<div className="log-thread">
						{messages.map((message, index) => (
							<LogMessageView
								events={trajectory.filter(event => event.messageIndex === index)}
								message={message}
								key={`${message.role}-${index}`}
							/>
						))}
					</div>
				</details>
			) : (
				<StatusBanner state="info" title="Prompt logging is off">
					Enable "Include prompts and completions in logs" in{' '}
					{props.onOpenSettings ? (
						<button className="link-button" type="button" onClick={props.onOpenSettings}>
							log settings
						</button>
					) : (
						'log settings'
					)}{' '}
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

type TrajectoryLane = 'input' | 'model' | 'tool-call' | 'tool-result';

type TrajectoryEvent = {
	lane: TrajectoryLane;
	label: string;
	approxTokens: number;
	anchorId: string;
	messageIndex: number;
	partIndex?: number;
	system?: boolean;
};

const TRAJECTORY_LANES: Array<{ lane: TrajectoryLane; label: string }> = [
	{ lane: 'input', label: 'Input' },
	{ lane: 'model', label: 'Model' },
	{ lane: 'tool-call', label: 'Tool call' },
	{ lane: 'tool-result', label: 'Tool result' }
];

function LogTrajectory(props: { events: TrajectoryEvent[]; onJump: (anchorId: string) => void }) {
	const [selected, setSelected] = useState<number | null>(null);
	if (!props.events.length) return null;

	const selectedEvent = selected == null ? null : props.events[selected];
	const chartStyle = {
		minWidth: `${Math.max(420, props.events.length * 24 + 92)}px`
	};
	const trackStyle = {
		gridTemplateColumns: props.events
			.map(event => `minmax(14px, ${event.approxTokens}fr)`)
			.join(' ')
	};

	return (
		<section className="log-trajectory" aria-labelledby="log-trajectory-title">
			<div className="log-trajectory-header">
				<h3 id="log-trajectory-title">Trajectory</h3>
				<span>
					{formatNumber(props.events.length)} step{props.events.length === 1 ? '' : 's'}
				</span>
			</div>
			<div className="log-trajectory-scroll">
				<div className="log-trajectory-chart" style={chartStyle}>
					{TRAJECTORY_LANES.map(({ lane, label }) => (
						<div className="log-trajectory-row" key={lane}>
							<span className="log-trajectory-label">{label}</span>
							<div className="log-trajectory-track" style={trackStyle}>
								{props.events.map((event, index) =>
									event.lane === lane ? (
										<button
											aria-label={`Step ${index + 1}: ${event.label}`}
											aria-pressed={selected === index}
											className={`log-trajectory-bar ${lane}${event.system ? ' system' : ''}${selected === index ? ' selected' : ''}`}
											key={`${lane}-${index}`}
											title={`Step ${index + 1}: ${event.label}`}
											type="button"
											onClick={() => setSelected(current => (current === index ? null : index))}
											onDoubleClick={() => {
												setSelected(index);
												props.onJump(event.anchorId);
											}}
										/>
									) : (
										<span
											aria-hidden="true"
											className="log-trajectory-gap"
											key={`${lane}-${index}`}
										/>
									)
								)}
							</div>
						</div>
					))}
				</div>
			</div>
			<div className="log-trajectory-caption" aria-live="polite">
				<span>
					{selectedEvent
						? `Step ${selected! + 1} ${selectedEvent.label}`
						: 'Width shows approximate tokens'}
				</span>
				{selectedEvent ? (
					<a
						href={`#${selectedEvent.anchorId}`}
						onClick={event => {
							event.preventDefault();
							props.onJump(selectedEvent.anchorId);
						}}
					>
						Jump to conversation
					</a>
				) : null}
			</div>
		</section>
	);
}

function trajectoryEvents(messages: RenderedLogMessage[]): TrajectoryEvent[] {
	const events: Array<Omit<TrajectoryEvent, 'anchorId'>> = [];
	for (const [messageIndex, message] of messages.entries()) {
		if (message.parts) {
			for (const [partIndex, part] of message.parts.entries()) {
				const base = { messageIndex, partIndex };
				switch (part.type) {
					case 'text': {
						if (!part.text.trim()) break;
						const model = message.role === 'assistant';
						events.push({
							...base,
							lane: model ? 'model' : 'input',
							label: trajectoryMessageLabel(
								model ? 'Model' : message.role === 'system' ? 'System' : 'User',
								part.text
							),
							approxTokens: approximateTokenCount(part.text),
							system: message.role === 'system'
						});
						break;
					}
					case 'toolCall':
						events.push({
							...base,
							lane: 'tool-call',
							label: `Tool call: ${part.name}`,
							approxTokens: approximateTokenCount(
								`${part.name}\n${trajectoryValueText(part.arguments)}`
							)
						});
						break;
					case 'toolResult':
						events.push({
							...base,
							lane: 'tool-result',
							label: `Tool result: ${part.name ?? 'unknown'}`,
							approxTokens: approximateTokenCount(trajectoryValueText(part.content))
						});
						break;
					case 'reasoning':
						// Encrypted reasoning size is not a meaningful token estimate.
						break;
				}
			}
			continue;
		}

		if (message.role === 'assistant') {
			if (message.content.trim()) {
				events.push({
					lane: 'model',
					label: trajectoryMessageLabel('Model', message.content),
					approxTokens: approximateTokenCount(message.content),
					messageIndex
				});
			}
			for (const call of message.toolCalls ?? []) {
				events.push({
					lane: 'tool-call',
					label: `Tool call: ${call.name}`,
					approxTokens: approximateTokenCount(
						`${call.name}\n${trajectoryValueText(call.arguments)}`
					),
					messageIndex
				});
			}
		} else if (message.role === 'tool') {
			events.push({
				lane: 'tool-result',
				label: `Tool result: ${message.name ?? 'unknown'}`,
				approxTokens: approximateTokenCount(message.content),
				messageIndex
			});
		} else if (message.content.trim()) {
			events.push({
				lane: 'input',
				label: trajectoryMessageLabel(
					message.role === 'system' ? 'System' : 'User',
					message.content
				),
				approxTokens: approximateTokenCount(message.content),
				messageIndex,
				system: message.role === 'system'
			});
		}
	}
	return events.map((event, index) => ({
		...event,
		anchorId: `conversation-step-${index + 1}`
	}));
}

function approximateTokenCount(text: string) {
	return Math.max(1, Math.ceil(text.length / 4));
}

function trajectoryValueText(value: unknown) {
	if (value === undefined || value === null) return '';
	if (typeof value === 'string') return value;
	try {
		return JSON.stringify(value);
	} catch {
		return String(value);
	}
}

function trajectoryMessageLabel(prefix: string, content: string) {
	const compact = content.replace(/\s+/g, ' ').trim();
	if (!compact) return prefix;
	const preview = compact.length > 64 ? `${compact.slice(0, 61)}...` : compact;
	return `${prefix}: ${preview}`;
}

function LogStat(props: { label: string; value: string; sub?: string }) {
	return (
		<div className="log-stat">
			<span className="log-stat-label">{props.label}</span>
			<span className="log-stat-value">{props.value}</span>
			{props.sub ? <span className="log-stat-sub">{props.sub}</span> : null}
		</div>
	);
}

function LogField(props: { label: string; value: string; mono?: boolean }) {
	return (
		<div className="log-field">
			<span className="log-field-label">{props.label}</span>
			<span className={props.mono ? 'log-field-value mono' : 'log-field-value'}>{props.value}</span>
		</div>
	);
}

function ModelRouteStep(props: { label: string; value: string; last?: boolean }) {
	return (
		<div className={props.last ? 'model-route-step last' : 'model-route-step'}>
			<span className="model-route-marker" aria-hidden="true" />
			<span className="model-route-label">{props.label}</span>
			<code className="model-route-value">{props.value}</code>
		</div>
	);
}

function LogUsagePanel(props: { usage: LogUsageDetail }) {
	const usage = props.usage;
	const inputBreakdown =
		usage.inputTokens != null
			? splitInputTokens(
					usage.inputTokens,
					usage.cacheReadTokens,
					usage.cacheWriteTokens,
					usage.inputAudioTokens
				)
			: null;
	const outputBreakdownUnavailable =
		(usage.reasoningTokens == null && Boolean(usage.reasoningCost)) ||
		(usage.outputAudioTokens == null && Boolean(usage.outputAudioCost));
	const outputBreakdown =
		usage.outputTokens != null && !outputBreakdownUnavailable
			? splitOutputTokens(usage.outputTokens, usage.reasoningTokens, usage.outputAudioTokens)
			: null;
	const showBar =
		usage.inputTokens != null &&
		usage.outputTokens != null &&
		usage.inputTokens + usage.outputTokens > 0;
	const showCostBar = [
		usage.inputCost,
		usage.cacheReadCost,
		usage.cacheWriteCost,
		usage.inputAudioCost,
		usage.outputCost,
		usage.reasoningCost,
		usage.outputAudioCost
	].some(cost => cost != null && cost > 0);
	const allRows: Array<{
		label: string;
		swatch?: 'input' | 'output' | 'reasoning' | 'audio' | 'cache-read' | 'cache-write';
		tokens?: number | null;
		cost?: number | null;
		kind?: 'input-group' | 'input-detail' | 'output-group' | 'output-detail';
	}> = [
		{
			label: 'Input',
			kind: 'input-group'
		},
		{
			label: 'Uncached',
			swatch: 'input',
			tokens: inputBreakdown?.uncached,
			cost: usage.inputCost,
			kind: 'input-detail'
		},
		{
			label: 'Cache write',
			swatch: 'cache-write',
			tokens: inputBreakdown?.cacheWrite ?? usage.cacheWriteTokens,
			cost: usage.cacheWriteCost,
			kind: 'input-detail'
		},
		{
			label: 'Cache read',
			swatch: 'cache-read',
			tokens: inputBreakdown?.cacheRead ?? usage.cacheReadTokens,
			cost: usage.cacheReadCost,
			kind: 'input-detail'
		},
		{
			label: 'Audio',
			swatch: 'audio',
			tokens: inputBreakdown?.audio ?? usage.inputAudioTokens,
			cost: usage.inputAudioCost,
			kind: 'input-detail'
		},
		{
			label: 'Output',
			kind: 'output-group'
		},
		{
			label: 'Reported total',
			tokens: outputBreakdownUnavailable ? usage.outputTokens : undefined,
			kind: 'output-detail'
		},
		{
			label: 'Generated',
			swatch: 'output',
			tokens: outputBreakdown?.generated,
			cost: usage.outputCost,
			kind: 'output-detail'
		},
		{
			label: 'Reasoning',
			swatch: 'reasoning',
			tokens: outputBreakdown?.reasoning ?? usage.reasoningTokens,
			cost: usage.reasoningCost,
			kind: 'output-detail'
		},
		{
			label: 'Audio',
			swatch: 'audio',
			tokens: outputBreakdown?.audio ?? usage.outputAudioTokens,
			cost: usage.outputAudioCost,
			kind: 'output-detail'
		}
	];
	const rows = allRows.filter(
		row =>
			row.label === 'Input' ||
			row.label === 'Uncached' ||
			row.label === 'Output' ||
			row.label === 'Generated' ||
			Boolean(row.tokens) ||
			Boolean(row.cost)
	);
	return (
		<div className="log-usage">
			{showBar || showCostBar ? (
				<div className="log-usage-bars">
					{showBar ? (
						<div className="log-usage-bar-row">
							<span className="log-usage-bar-label">Tokens</span>
							<TokenBar
								input={usage.inputTokens!}
								output={usage.outputTokens!}
								cacheRead={usage.cacheReadTokens ?? undefined}
								cacheWrite={usage.cacheWriteTokens ?? undefined}
								inputAudio={usage.inputAudioTokens ?? undefined}
								reasoning={usage.reasoningTokens ?? undefined}
								outputAudio={usage.outputAudioTokens ?? undefined}
								splitOutput={!outputBreakdownUnavailable}
							/>
						</div>
					) : null}
					{showCostBar ? (
						<div className="log-usage-bar-row">
							<span className="log-usage-bar-label">Cost</span>
							<CostBar usage={usage} />
						</div>
					) : null}
				</div>
			) : null}
			<div className="log-usage-table">
				<div className="log-usage-row head">
					<span />
					<span>Tokens</span>
					<span>Cost</span>
				</div>
				{rows.map(row => (
					<div
						className={`log-usage-row${row.kind ? ` ${row.kind}` : ''}`}
						key={`${row.kind ?? 'other'}-${row.label}`}
					>
						<span className="log-usage-label">
							{row.swatch ? (
								<span className={`log-usage-swatch ${row.swatch}`} aria-hidden="true" />
							) : null}
							{row.label}
						</span>
						<span className="log-usage-tokens">
							{row.tokens != null ? formatNumber(row.tokens) : ''}
						</span>
						<span className="log-usage-cost">
							{row.cost != null && row.cost > 0 ? formatCost(row.cost, usage.costDigits) : ''}
						</span>
					</div>
				))}
				<div className="log-usage-row total">
					<span className="log-usage-label">Total</span>
					<span className="log-usage-tokens">
						{usage.totalTokens != null ? formatNumber(usage.totalTokens) : ''}
					</span>
					<span className="log-usage-cost">
						{usage.totalCost != null && usage.totalCost > 0
							? formatCost(usage.totalCost, usage.costDigits)
							: ''}
					</span>
				</div>
			</div>
		</div>
	);
}

const LOG_ROLE_LABELS: Record<RenderedLogMessage['role'], string> = {
	system: 'System',
	user: 'User',
	assistant: 'Assistant',
	tool: 'Tool'
};

function LogMessageView(props: { message: RenderedLogMessage; events: TrajectoryEvent[] }) {
	const message = props.message;
	const content = message.content;
	const isSystem = message.role === 'system';
	const collapsible = content.length > (isSystem ? 280 : 2000);
	const [collapsed, setCollapsed] = useState(collapsible);
	const hasToolCalls = Boolean(message.toolCalls?.length);
	const hasToolResults = Boolean(message.toolResults?.length);
	const hasReasoning = Boolean(message.reasoning?.length);
	const copyValue =
		message.parts || hasToolCalls || hasToolResults || hasReasoning
			? serializeLogMessage(message)
			: content;
	const messageAnchor = props.events.find(
		event => event.partIndex == null && event.lane !== 'tool-call'
	)?.anchorId;
	const toolCallEvents = props.events.filter(
		event => event.partIndex == null && event.lane === 'tool-call'
	);

	return (
		<article className={`log-msg ${message.role}`} id={messageAnchor}>
			<header className="log-msg-header">
				<span className="log-msg-role">{LOG_ROLE_LABELS[message.role]}</span>
				{message.role === 'tool' && message.name ? (
					<code className="log-msg-name">{message.name}</code>
				) : null}
				{copyValue ? (
					<span className="log-msg-meta">{formatNumber(copyValue.length)} chars</span>
				) : null}
				{copyValue ? <CopyButton value={copyValue} /> : null}
			</header>
			<div className="log-msg-body">
				{message.parts ? (
					<>
						{message.parts.map((part, index) => (
							<LogMessagePartView
								anchorId={props.events.find(event => event.partIndex === index)?.anchorId}
								part={part}
								collapsed={part.type === 'text' && collapsed}
								key={`${part.type}-${index}`}
							/>
						))}
						{collapsible ? (
							<button
								className="log-msg-toggle"
								type="button"
								onClick={() => setCollapsed(current => !current)}
							>
								{collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
								{collapsed ? `Show all (${formatNumber(content.length)} chars)` : 'Collapse'}
							</button>
						) : null}
					</>
				) : message.role === 'tool' && !hasToolResults ? (
					<LogToolBlock kind="result" name={message.name ?? 'unknown'} value={content} />
				) : content ? (
					<>
						<LogMarkdown content={content} collapsed={collapsed} />
						{collapsible ? (
							<button
								className="log-msg-toggle"
								type="button"
								onClick={() => setCollapsed(current => !current)}
							>
								{collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
								{collapsed ? `Show all (${formatNumber(content.length)} chars)` : 'Collapse'}
							</button>
						) : null}
					</>
				) : null}
				{!message.parts && hasToolCalls
					? message.toolCalls!.map((call, index) => (
							<LogToolBlock
								anchorId={toolCallEvents[index]?.anchorId}
								kind="call"
								name={call.name}
								value={call.arguments}
								key={`${call.name}-${index}`}
							/>
						))
					: null}
				{!message.parts && hasToolResults
					? message.toolResults!.map((result, index) => (
							<LogToolBlock
								kind="result"
								name={result.name ?? 'unknown'}
								value={result.content}
								isError={result.isError}
								key={`${result.id ?? result.name ?? 'result'}-${index}`}
							/>
						))
					: null}
				{!message.parts && hasReasoning
					? message.reasoning!.map((reasoning, index) => (
							<LogReasoningBlock content={reasoning} key={`reasoning-${index}`} />
						))
					: null}
				{!content && !hasToolCalls && !hasToolResults && !hasReasoning ? (
					<span className="log-msg-empty">empty message</span>
				) : null}
			</div>
		</article>
	);
}

function LogMessagePartView(props: {
	part: RenderedLogMessagePart;
	collapsed: boolean;
	anchorId?: string;
}) {
	const part = props.part;
	switch (part.type) {
		case 'text':
			return part.text ? (
				<LogMarkdown content={part.text} collapsed={props.collapsed} anchorId={props.anchorId} />
			) : null;
		case 'toolCall':
			return (
				<LogToolBlock
					anchorId={props.anchorId}
					kind="call"
					name={part.name}
					value={part.arguments}
				/>
			);
		case 'toolResult':
			return (
				<LogToolBlock
					anchorId={props.anchorId}
					kind="result"
					name={part.name ?? 'unknown'}
					value={part.content}
					isError={part.isError}
				/>
			);
		case 'reasoning':
			return <LogReasoningBlock content={part.content} />;
	}
}

function LogReasoningBlock(props: { content: unknown }) {
	return (
		<LogToolBlock kind="reasoning" name="reasoning" value={reasoningDisplayText(props.content)} />
	);
}

function reasoningDisplayText(content: unknown) {
	if (!content || typeof content !== 'object') {
		return typeof content === 'string' && content.trim()
			? content
			: 'Reasoning details unavailable';
	}
	const record = content as Record<string, unknown>;
	const summary = reasoningSummaryText(record.summary);
	if (summary) return summary;

	const encrypted = record.encrypted_content ?? record.encryptedContent;
	if (typeof encrypted !== 'string' || !encrypted) return 'Reasoning details unavailable';
	const bytes = decodedBase64UrlLength(encrypted);
	return bytes == null ? 'Encrypted' : `Encrypted (${formatNumber(bytes)} bytes)`;
}

function reasoningSummaryText(value: unknown): string {
	if (typeof value === 'string') return value.trim();
	if (Array.isArray(value)) return value.map(reasoningSummaryText).filter(Boolean).join('\n');
	if (!value || typeof value !== 'object') return '';
	const record = value as Record<string, unknown>;
	return reasoningSummaryText(record.text ?? record.content);
}

function decodedBase64UrlLength(value: string) {
	if (!/^[A-Za-z0-9_-]+={0,2}$/.test(value)) return null;
	const unpadded = value.replace(/=+$/, '');
	if (unpadded.length % 4 === 1) return null;
	const padded = unpadded
		.replaceAll('-', '+')
		.replaceAll('_', '/')
		.padEnd(unpadded.length + ((4 - (unpadded.length % 4)) % 4), '=');
	try {
		return atob(padded).length;
	} catch {
		return null;
	}
}

function serializeLogMessage(message: RenderedLogMessage) {
	return (
		JSON.stringify(
			{
				role: message.role,
				...(message.parts
					? { parts: message.parts }
					: {
							...(message.content ? { content: message.content } : {}),
							...(message.toolCalls?.length ? { toolCalls: message.toolCalls } : {}),
							...(message.toolResults?.length ? { toolResults: message.toolResults } : {}),
							...(message.reasoning?.length ? { reasoning: message.reasoning } : {})
						})
			},
			null,
			2
		) ?? ''
	);
}

const logMarkdownRenderer = new marked.Renderer();
logMarkdownRenderer.html = ({ text }) => escapeLogMarkdownHtml(text).replaceAll('\n', '<br>');
logMarkdownRenderer.link = ({ text }) => escapeLogMarkdownHtml(text);
logMarkdownRenderer.image = ({ text }) =>
	text ? `[image: ${escapeLogMarkdownHtml(text)}]` : '[image]';

const LOG_MARKDOWN_TAGS = [
	'blockquote',
	'br',
	'code',
	'del',
	'em',
	'h1',
	'h2',
	'h3',
	'h4',
	'h5',
	'h6',
	'hr',
	'li',
	'ol',
	'p',
	'pre',
	'strong',
	'table',
	'tbody',
	'td',
	'th',
	'thead',
	'tr',
	'ul'
];

function escapeLogMarkdownHtml(value: string) {
	return value
		.replaceAll('&', '&amp;')
		.replaceAll('<', '&lt;')
		.replaceAll('>', '&gt;')
		.replaceAll('"', '&quot;')
		.replaceAll("'", '&#39;');
}

function LogMarkdown(props: { content: string; collapsed: boolean; anchorId?: string }) {
	const html = useMemo(
		() =>
			DOMPurify.sanitize(
				marked.parse(props.content, {
					async: false,
					breaks: true,
					gfm: true,
					renderer: logMarkdownRenderer
				}) as string,
				{
					ALLOWED_ATTR: [],
					ALLOWED_TAGS: LOG_MARKDOWN_TAGS
				}
			),
		[props.content]
	);
	return (
		<div
			className={`log-msg-content log-markdown${props.collapsed ? ' collapsed' : ''}`}
			dangerouslySetInnerHTML={{ __html: html }}
			id={props.anchorId}
		/>
	);
}

function LogToolBlock(props: {
	kind: 'call' | 'result' | 'reasoning';
	name: string;
	anchorId?: string;
	value?: unknown;
	isError?: boolean;
}) {
	const [open, setOpen] = useState(false);
	const isCall = props.kind === 'call';
	const summary = summarizeLogValue(props.value);
	const parsedResult =
		props.kind === 'result' && typeof props.value === 'string'
			? (() => {
					try {
						return JSON.parse(props.value) as unknown;
					} catch {
						return null;
					}
				})()
			: null;
	const copyValue = summary;
	return (
		<div
			className={`log-tool-block ${props.kind}${props.isError ? ' error' : ''}${open ? ' open' : ''}`}
			id={props.anchorId}
		>
			<div className="log-tool-head">
				<button
					className="log-tool-toggle"
					type="button"
					aria-expanded={open}
					onClick={() => setOpen(current => !current)}
				>
					{isCall ? <Wrench size={13} /> : <Braces size={13} />}
					<span className="log-tool-kind">
						{isCall ? 'tool call' : props.kind === 'reasoning' ? 'reasoning' : 'result'}
					</span>
					{props.kind !== 'reasoning' ? <code className="log-tool-name">{props.name}</code> : null}
					{!open ? (
						<span className="log-tool-summary">
							{summary || (isCall ? 'no args' : 'empty result')}
						</span>
					) : null}
					<ChevronDown className="log-tool-chevron" size={14} />
				</button>
				{copyValue ? <CopyButton value={copyValue} /> : null}
			</div>
			{open ? (
				<div className="log-tool-body">
					{typeof props.value === 'string' && parsedResult == null ? (
						<pre className="log-tool-text">{props.value || 'empty result'}</pre>
					) : parsedResult != null ? (
						<JsonBlock value={parsedResult} />
					) : props.value != null ? (
						<JsonBlock value={props.value} />
					) : (
						<pre className="log-tool-text">{isCall ? 'no arguments' : 'empty result'}</pre>
					)}
				</div>
			) : null}
		</div>
	);
}

function logConversation(entry: LogEntry): RenderedLogMessage[] {
	const prompt = payloadValue(entry, 'gen_ai.prompt', 'requestPrompt');
	const completion = payloadValue(entry, 'gen_ai.completion', 'responseCompletion');
	return [...messagesFromPrompt(prompt), ...messagesFromCompletion(completion)];
}

function payloadValue(
	entry: LogEntry,
	attributeKey: string,
	payloadKey: 'requestPrompt' | 'responseCompletion'
) {
	const attribute = attributeValue(entry.attributes, attributeKey);
	if (attribute !== undefined && attribute !== null) return parseMaybeJson(attribute);
	return parseMaybeJson(entry.payload?.[payloadKey]);
}

function attributeValue(attributes: unknown, key: string): unknown {
	if (!attributes || typeof attributes !== 'object') return undefined;
	const record = attributes as Record<string, unknown>;
	if (key in record) return record[key];
	const parts = key.split('.');
	let current: unknown = record;
	for (const part of parts) {
		if (!current || typeof current !== 'object') return undefined;
		current = (current as Record<string, unknown>)[part];
	}
	return current;
}

function parseMaybeJson(value: unknown): unknown {
	if (typeof value !== 'string') return value;
	const trimmed = value.trim();
	if (!trimmed) return '';
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
	if (typeof value === 'object') {
		const record = value as Record<string, unknown>;
		if (Array.isArray(record.messages)) return record.messages.flatMap(messageFromUnknown);
		if (Array.isArray(record.contents)) return record.contents.flatMap(messageFromUnknown);
		if (record.prompt !== undefined) return [{ role: 'user', content: contentText(record.prompt) }];
	}
	return [{ role: 'user', content: contentText(value) }];
}

function messagesFromCompletion(value: unknown): RenderedLogMessage[] {
	if (!value) return [];
	if (Array.isArray(value))
		return value.flatMap(messageFromUnknown).map(message => ({
			...message,
			role: message.role === 'user' ? 'assistant' : message.role
		}));
	if (typeof value === 'object') {
		const record = value as Record<string, unknown>;
		if (Array.isArray(record.choices)) {
			return record.choices
				.flatMap(choice => {
					const choiceRecord =
						choice && typeof choice === 'object' ? (choice as Record<string, unknown>) : {};
					return messageFromUnknown(
						choiceRecord.message ?? choiceRecord.delta ?? choiceRecord.text
					);
				})
				.map(message => ({
					...message,
					role: message.role === 'user' ? 'assistant' : message.role
				}));
		}
		if (record.message !== undefined)
			return messageFromUnknown(record.message).map(message => ({
				...message,
				role: 'assistant'
			}));
		if (
			record.content !== undefined ||
			record.text !== undefined ||
			record.tool_calls !== undefined
		) {
			return messageFromUnknown({ role: 'assistant', ...record });
		}
	}
	return [{ role: 'assistant', content: contentText(value) }];
}

function messageFromUnknown(value: unknown): RenderedLogMessage[] {
	if (value === undefined || value === null) return [];
	if (typeof value !== 'object' || Array.isArray(value))
		return [{ role: 'user', content: contentText(value) }];
	const record = value as Record<string, unknown>;
	if (Array.isArray(record.parts)) return [messageFromNormalizedParts(record)];
	const role = normalizeRole(record.role);
	const content = contentText(record.content ?? record.text ?? record.message ?? '');
	const toolCalls = toolCallsFromUnknown(record.tool_calls ?? record.toolCalls);
	const name = typeof record.name === 'string' ? record.name : undefined;
	return [{ role, content, name, toolCalls }];
}

function messageFromNormalizedParts(record: Record<string, unknown>): RenderedLogMessage {
	const text: string[] = [];
	const parts: RenderedLogMessagePart[] = [];
	const toolCalls: NonNullable<RenderedLogMessage['toolCalls']> = [];
	const toolResults: NonNullable<RenderedLogMessage['toolResults']> = [];
	const reasoning: unknown[] = [];
	for (const part of record.parts as unknown[]) {
		if (!part || typeof part !== 'object') continue;
		const value = part as Record<string, unknown>;
		switch (value.type) {
			case 'text':
				if (typeof value.text === 'string') {
					text.push(value.text);
					parts.push({ type: 'text', text: value.text });
				}
				break;
			case 'toolCall':
				toolCalls.push({
					id: typeof value.id === 'string' ? value.id : undefined,
					name: typeof value.name === 'string' ? value.name : 'unknown',
					arguments: value.arguments
				});
				parts.push({ type: 'toolCall', ...toolCalls[toolCalls.length - 1] });
				break;
			case 'toolResult':
				toolResults.push({
					id: typeof value.id === 'string' ? value.id : undefined,
					name: typeof value.name === 'string' ? value.name : undefined,
					content: value.content,
					isError: typeof value.isError === 'boolean' ? value.isError : undefined
				});
				parts.push({ type: 'toolResult', ...toolResults[toolResults.length - 1] });
				break;
			case 'reasoning':
				reasoning.push(value.content);
				parts.push({ type: 'reasoning', content: value.content });
				break;
		}
	}
	return {
		role: normalizeRole(record.role),
		content: text.join('\n'),
		parts,
		toolCalls: toolCalls.length ? toolCalls : undefined,
		toolResults: toolResults.length ? toolResults : undefined,
		reasoning: reasoning.length ? reasoning : undefined
	};
}

function normalizeRole(value: unknown): RenderedLogMessage['role'] {
	if (value === 'system' || value === 'assistant' || value === 'tool') return value;
	if (value === 'developer') return 'system';
	return 'user';
}

function toolCallsFromUnknown(value: unknown): RenderedLogMessage['toolCalls'] {
	if (!Array.isArray(value)) return undefined;
	const calls = value.flatMap(item => {
		if (!item || typeof item !== 'object') return [];
		const record = item as Record<string, unknown>;
		const fn =
			record.function && typeof record.function === 'object'
				? (record.function as Record<string, unknown>)
				: record;
		const name = typeof fn.name === 'string' ? fn.name : 'unknown';
		const id = typeof record.id === 'string' ? record.id : undefined;
		return [{ id, name, arguments: parseMaybeJson(fn.arguments) }];
	});
	return calls.length ? calls : undefined;
}

function contentText(value: unknown): string {
	if (value === undefined || value === null) return '';
	if (typeof value === 'string') return value;
	if (typeof value === 'number' || typeof value === 'boolean') return String(value);
	if (Array.isArray(value)) {
		return value
			.map(item => {
				if (typeof item === 'string') return item;
				if (item && typeof item === 'object') {
					const record = item as Record<string, unknown>;
					if (typeof record.text === 'string') return record.text;
					if (typeof record.content === 'string') return record.content;
				}
				return summarizeLogValue(item);
			})
			.join('\n');
	}
	return summarizeLogValue(value);
}

function summarizeLogValue(value: unknown) {
	if (value === undefined || value === null || value === '') return '';
	if (typeof value === 'string') return value;
	try {
		return JSON.stringify(value, null, 2);
	} catch {
		return String(value);
	}
}

function downloadJson(value: unknown) {
	const blob = new Blob([JSON.stringify(value, null, 2)], {
		type: 'application/json'
	});
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement('a');
	anchor.href = url;
	anchor.download = 'agentgateway-log.json';
	anchor.click();
	URL.revokeObjectURL(url);
}

function TokenCells(props: { entry: LogEntry }) {
	const input = props.entry.usage.inputTokens;
	const output = props.entry.usage.outputTokens;
	const cacheRead = attributeNumber(props.entry.attributes, [
		'gen_ai.usage.cache_read.input_tokens',
		'cacheTokens',
		'cachedTokens',
		'cache_tokens',
		'cached_tokens'
	]);
	const cachePercent =
		cacheRead != null && cacheRead > 0 && input != null && input > 0
			? Math.min(Math.round((cacheRead / input) * 100), 100)
			: null;
	return (
		<>
			<td
				className="log-td-num"
				title={input != null ? `${formatNumber(input)} input tokens` : undefined}
			>
				{input == null ? '—' : formatCompactNumber(input)}
			</td>
			<td
				className="log-td-num"
				title={output != null ? `${formatNumber(output)} output tokens` : undefined}
			>
				{output == null ? '—' : formatCompactNumber(output)}
			</td>
			<td
				className="log-td-num"
				title={cacheRead != null ? `${formatNumber(cacheRead)} cached input tokens` : undefined}
			>
				{cachePercent == null ? '—' : `${cachePercent}%`}
			</td>
		</>
	);
}

function CostSummary(props: { entry: LogEntry }) {
	if (props.entry.cost == null) return <span className="log-cost-summary empty">—</span>;
	return <span className="log-cost-summary">{formatCost(props.entry.cost)}</span>;
}

function attributeNumber(value: unknown, keys: string[]) {
	if (!value || typeof value !== 'object') return undefined;
	const record = value as Record<string, unknown>;
	for (const key of keys) {
		const direct = record[key];
		if (typeof direct === 'number') return direct;
		if (typeof direct === 'string' && Number.isFinite(Number(direct))) return Number(direct);
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
			className={`copy-button${copied ? ' copied' : ''}`}
			type="button"
			aria-label="Copy to clipboard"
			onClick={handleCopy}
		>
			{copied ? <Check size={11} /> : <Copy size={11} />}
		</button>
	);
}

function TokenBar(props: {
	input: number;
	output: number;
	cacheRead?: number;
	cacheWrite?: number;
	inputAudio?: number;
	reasoning?: number;
	outputAudio?: number;
	splitOutput: boolean;
}) {
	const {
		input,
		cacheRead,
		cacheWrite,
		uncached,
		audio: inputAudio
	} = splitInputTokens(props.input, props.cacheRead, props.cacheWrite, props.inputAudio);
	const {
		output,
		generated,
		reasoning,
		audio: outputAudio
	} = splitOutputTokens(props.output, props.reasoning, props.outputAudio);
	const total = input + output;
	if (!total) return null;
	const inputPct = (uncached / total) * 100;
	const cacheReadPct = (cacheRead / total) * 100;
	const cacheWritePct = (cacheWrite / total) * 100;
	const inputAudioPct = (inputAudio / total) * 100;
	const outputPct = (output / total) * 100;
	const generatedPct = (generated / total) * 100;
	const reasoningPct = (reasoning / total) * 100;
	const outputAudioPct = (outputAudio / total) * 100;
	const title = [
		`input: ${formatNumber(input)}`,
		`uncached: ${formatNumber(uncached)}`,
		cacheRead ? `cache read: ${formatNumber(cacheRead)}` : null,
		cacheWrite ? `cache write: ${formatNumber(cacheWrite)}` : null,
		inputAudio ? `input audio: ${formatNumber(inputAudio)}` : null,
		props.splitOutput ? `generated: ${formatNumber(generated)}` : null,
		props.splitOutput && reasoning ? `reasoning: ${formatNumber(reasoning)}` : null,
		props.splitOutput && outputAudio ? `output audio: ${formatNumber(outputAudio)}` : null,
		!props.splitOutput ? `output: ${formatNumber(output)}` : null
	]
		.filter(Boolean)
		.join(' / ');
	return (
		<UsageBar
			title={title}
			segments={[
				{ key: 'input', value: inputPct, className: 'token-bar-input' },
				{
					key: 'cache-read',
					value: cacheReadPct,
					className: 'token-bar-cache-read'
				},
				{
					key: 'cache-write',
					value: cacheWritePct,
					className: 'token-bar-cache-write'
				},
				{
					key: 'input-audio',
					value: inputAudioPct,
					className: 'token-bar-audio'
				},
				{
					key: 'combined-output',
					value: props.splitOutput ? 0 : outputPct,
					className: 'token-bar-output'
				},
				{
					key: 'generated',
					value: props.splitOutput ? generatedPct : 0,
					className: 'token-bar-output'
				},
				{
					key: 'reasoning',
					value: props.splitOutput ? reasoningPct : 0,
					className: 'token-bar-reasoning'
				},
				{
					key: 'output-audio',
					value: props.splitOutput ? outputAudioPct : 0,
					className: 'token-bar-audio'
				}
			]}
		/>
	);
}

function CostBar(props: { usage: LogUsageDetail }) {
	const usage = props.usage;
	const components = [
		['input', usage.inputCost, 'token-bar-input'],
		['cache read', usage.cacheReadCost, 'token-bar-cache-read'],
		['cache write', usage.cacheWriteCost, 'token-bar-cache-write'],
		['input audio', usage.inputAudioCost, 'token-bar-audio'],
		['generated', usage.outputCost, 'token-bar-output'],
		['reasoning', usage.reasoningCost, 'token-bar-reasoning'],
		['output audio', usage.outputAudioCost, 'token-bar-audio']
	] as const;
	const total = components.reduce((sum, [, cost]) => sum + Math.max(cost ?? 0, 0), 0);
	if (!total) return null;
	const title = components
		.filter(([, cost]) => cost != null && cost > 0)
		.map(([label, cost]) => `${label}: ${formatCost(cost!)}`)
		.join(' / ');
	return (
		<UsageBar
			title={title}
			segments={components.map(([key, cost, className]) => ({
				key,
				value: (Math.max(cost ?? 0, 0) / total) * 100,
				className
			}))}
		/>
	);
}

function UsageBar(props: {
	title: string;
	segments: Array<{ key: string; value: number; className: string }>;
}) {
	return (
		<div className="token-bar" title={props.title}>
			{props.segments
				.filter(segment => segment.value > 0)
				.map(segment => (
					<div
						className={segment.className}
						key={segment.key}
						style={{ width: `${segment.value}%` }}
					/>
				))}
		</div>
	);
}

function splitInputTokens(
	inputTokens: number,
	cacheReadTokens?: number,
	cacheWriteTokens?: number,
	inputAudioTokens?: number
) {
	const input = Math.max(inputTokens, 0);
	const audio = Math.min(Math.max(inputAudioTokens ?? 0, 0), input);
	const cacheRead = Math.min(Math.max(cacheReadTokens ?? 0, 0), input - audio);
	const cacheWrite = Math.min(Math.max(cacheWriteTokens ?? 0, 0), input - audio - cacheRead);
	return {
		input,
		uncached: input - audio - cacheRead - cacheWrite,
		cacheRead,
		cacheWrite,
		audio
	};
}

function splitOutputTokens(
	outputTokens: number,
	reasoningTokens?: number,
	outputAudioTokens?: number
) {
	const output = Math.max(outputTokens, 0);
	const reasoning = Math.min(Math.max(reasoningTokens ?? 0, 0), output);
	const audio = Math.min(Math.max(outputAudioTokens ?? 0, 0), output - reasoning);
	return {
		output,
		generated: output - reasoning - audio,
		reasoning,
		audio
	};
}
