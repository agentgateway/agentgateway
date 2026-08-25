import { Pencil, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

import type { BudgetStatus } from '@/api/budgetsApi';
import { budgetScopeLabel } from '@/api/budgetsApi';
import { ConfigSaveButton } from '@/components/ConfigDiffDrawer';
import { FreeformCombobox } from '@/components/FreeformCombobox';
import {
	ConfirmDialog,
	Drawer,
	Dropdown,
	EmptyState,
	Field,
	FieldGroup,
	formatNumber,
	PageHeader,
	Panel,
	SegmentedControl,
	StatusBanner,
	Tooltip
} from '@/components/Primitives';
import { getApiKeyPolicy, upsertVirtualKey } from '@/config';
import { keyLabel, keyValue } from '@/credentialDisplay';
import type { Budget } from '@/gateway-config';
import { useBudgetStatus, useLlmConfigData, useUpdateConfig } from '@/hooks';
import type { GatewayConfig, VirtualApiKey } from '@/types';

/** Where a budget is declared, which also decides how it is written back to the configuration. */
type Source = { kind: 'document' } | { kind: 'key'; key: string };

type BudgetRow = { budget: Budget; source: Source };

type ScopeKind = 'oneKey' | 'perKey' | 'groupBy' | 'selector';

/** The four ways a budget can decide which keys share its counter, in the order they are offered. */
const scopeKinds: { value: ScopeKind; label: string; description: string }[] = [
	{
		value: 'oneKey',
		label: 'One key',
		description: 'A single key gets this allowance.'
	},
	{
		value: 'perKey',
		label: 'Each key',
		description: 'Every key gets its own separate allowance of this size.'
	},
	{
		value: 'groupBy',
		label: 'Per metadata value',
		description: 'One allowance for each distinct value of a metadata field, such as one per team.'
	},
	{
		value: 'selector',
		label: 'Shared pool',
		description: 'One allowance split between every key whose metadata matches a selector.'
	}
];

export function BudgetsPage() {
	const { config, apiKeys, isLoading, error } = useLlmConfigData();
	const status = useBudgetStatus();
	const updateConfig = useUpdateConfig();
	const [editing, setEditing] = useState<BudgetRow | null>(null);
	const [deleting, setDeleting] = useState<BudgetRow | null>(null);

	const rows = budgetRows(config.data, apiKeys);
	const databaseConfigured = Boolean(config.data?.config?.database?.url);
	const saveError = updateConfig.error?.message ?? null;
	const close = () => {
		setEditing(null);
		setDeleting(null);
	};

	function write(mutate: (draft: GatewayConfig) => void) {
		updateConfig.mutate(mutate, { onSuccess: close });
	}

	function saveBudget(budget: Budget, source: Source, original: BudgetRow | null) {
		write(draft => {
			if (original) removeBudget(draft, original);
			addBudget(draft, source, budget);
		});
	}

	function startNew() {
		setEditing({ budget: newBudget(), source: { kind: 'document' } });
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="Budgets"
				description="Cap LLM spend or token usage for your API keys. Each budget is charged once a response completes, and resets when its rolling window rolls over."
				actions={
					<button
						className="button primary"
						type="button"
						disabled={updateConfig.isPending || Boolean(error)}
						onClick={startNew}
					>
						<Plus size={16} />
						New budget
					</button>
				}
			/>

			{saveError ? (
				<StatusBanner state="bad" title="Could not save">
					{saveError}
				</StatusBanner>
			) : null}
			{!databaseConfigured && rows.length > 0 ? (
				<StatusBanner state="warn" title="Budgets require a database">
					Counters are persisted so that usage survives restarts and is shared between gateway
					instances. Set config.database.url, or the gateway will reject this configuration on load.
				</StatusBanner>
			) : null}

			<Panel>
				<div className="section-heading-row">
					<div>
						<h3>Configured budgets</h3>
						<p>
							Every budget pairs a limit with a scope that decides which keys share the counter. A
							budget scoped to a single key is stored on that key; every other scope is stored at
							the top level of the configuration and applies across all API key policies.
						</p>
					</div>
				</div>
				{isLoading ? (
					<StatusBanner state="loading" title="Loading configuration" />
				) : error ? (
					<StatusBanner state="bad" title="Configuration API unavailable">
						{error.message}
					</StatusBanner>
				) : rows.length === 0 ? (
					<EmptyState
						title="No budgets configured"
						description="A budget caps spend or tokens over a rolling window. Scope it to one key, to every key individually, to each value of a metadata field such as team or tier, or to a pool of keys that share a single allowance."
						action={
							<button className="button primary" type="button" onClick={startNew}>
								<Plus size={16} />
								New budget
							</button>
						}
					/>
				) : (
					<div className="table-wrap">
						<table className="keys-table">
							<thead>
								<tr>
									<th>Name</th>
									<th>Applies to</th>
									<th>Limit</th>
									<th>Window</th>
									<th>When exceeded</th>
									<th>Live counters</th>
									<th />
								</tr>
							</thead>
							<tbody>
								{rows.map(row => (
									<tr key={rowKey(row)}>
										<td>
											<strong>{row.budget.name}</strong>
										</td>
										<td className="muted">{scopeSummary(row, apiKeys)}</td>
										<td>{amountLabel(String(row.budget.limit.amount), row.budget.limit.unit)}</td>
										<td className="muted">{row.budget.window.rolling}</td>
										<td className="muted">{row.budget.onBudgetExceeded}</td>
										<td className="muted">
											{counterCountLabel(countersFor(row, status.data?.budgets))}
										</td>
										<td className="key-action-cell">
											<div className="key-actions">
												<Tooltip content="Edit budget">
													<button
														className="table-action"
														type="button"
														aria-label="Edit budget"
														onClick={() => setEditing(structuredClone(row))}
													>
														<Pencil size={14} />
														Edit
													</button>
												</Tooltip>
												<Tooltip content="Delete budget">
													<button
														className="table-action danger"
														type="button"
														aria-label="Delete budget"
														disabled={updateConfig.isPending}
														onClick={() => setDeleting(row)}
													>
														<Trash2 size={14} />
														Delete
													</button>
												</Tooltip>
											</div>
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</div>
				)}
			</Panel>

			<UsagePanel status={status.data?.budgets} loading={status.isLoading} />

			{editing ? (
				<BudgetEditor
					key={rowKey(editing)}
					initial={editing}
					keys={apiKeys}
					saving={updateConfig.isPending}
					onCancel={close}
					onSave={(budget, source) =>
						saveBudget(budget, source, editing.budget.name ? editing : null)
					}
				/>
			) : null}

			{deleting ? (
				<ConfirmDialog
					title="Delete budget"
					confirmLabel="Delete"
					destructive
					confirmDisabled={updateConfig.isPending}
					onCancel={close}
					onConfirm={() => write(draft => removeBudget(draft, deleting))}
				>
					Usage recorded for "{deleting.budget.name}" stops being enforced immediately.
				</ConfirmDialog>
			) : null}
		</div>
	);
}

/**
 * Live counters for every budget. One configured budget can back many counters, since a
 * per-metadata scope creates one for each distinct value it finds.
 */
function UsagePanel(props: { status?: BudgetStatus[]; loading: boolean }) {
	const budgets = props.status ?? [];
	return (
		<Panel>
			<div className="section-heading-row">
				<div>
					<h3>Current usage</h3>
					<p>
						One configured budget can back several counters, because a per-metadata scope creates
						one for each distinct value it finds. Usage is charged after each response, so
						concurrent requests can overshoot a limit slightly before it takes effect.
					</p>
				</div>
			</div>
			{props.loading ? (
				<StatusBanner state="loading" title="Loading usage" />
			) : budgets.length === 0 ? (
				<EmptyState
					title="No active counters"
					description="A counter appears as soon as a configured budget matches an API key, and stays at zero until that key gets its first response."
				/>
			) : (
				<div className="table-wrap">
					<table className="keys-table">
						<thead>
							<tr>
								<th>Name</th>
								<th>Applies to</th>
								<th>Usage</th>
								<th>Remaining</th>
								<th>Resets</th>
							</tr>
						</thead>
						<tbody>
							{budgets.map(budget => (
								<tr key={counterKey(budget)}>
									<td>
										<strong>{budget.name}</strong>
									</td>
									<td className="muted">{budgetScopeLabel(budget.scope)}</td>
									<td>
										<div className="key-budget-summary-row">
											<span>
												{amountLabel(budget.usage.used, budget.limit.unit)} of{' '}
												{amountLabel(budget.limit.amount, budget.limit.unit)}
											</span>
											<div className="api-key-budget-meter">
												<div className={meterLevel(budget)} style={{ width: usedWidth(budget) }} />
											</div>
										</div>
									</td>
									<td className="muted">
										{amountLabel(budget.usage.remaining, budget.limit.unit)}
									</td>
									<td className="muted">
										{budget.window.expired
											? 'Window elapsed'
											: new Date(budget.window.end).toLocaleString()}
									</td>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			)}
		</Panel>
	);
}

function BudgetEditor(props: {
	initial: BudgetRow;
	keys: VirtualApiKey[];
	saving: boolean;
	onCancel: () => void;
	onSave: (budget: Budget, source: Source) => void;
}) {
	const [name, setName] = useState(props.initial.budget.name);
	const [kind, setKind] = useState<ScopeKind>(rowScopeKind(props.initial));
	const [targetKey, setTargetKey] = useState(
		props.initial.source.kind === 'key' ? props.initial.source.key : (keyOf(props.keys[0]) ?? '')
	);
	const [field, setField] = useState(groupByField(props.initial.budget));
	const [selector, setSelector] = useState(selectorRows(props.initial.budget));
	const [unit, setUnit] = useState(props.initial.budget.limit.unit);
	const [amount, setAmount] = useState(String(props.initial.budget.limit.amount));
	const [rolling, setRolling] = useState(props.initial.budget.window.rolling);
	const [action, setAction] = useState(props.initial.budget.onBudgetExceeded);

	const fieldOptions = metadataFields(props.keys);
	const amountValue = Number(amount);
	const problem = !name.trim()
		? 'Name is required.'
		: !Number.isFinite(amountValue) || amountValue <= 0
			? 'Limit must be a positive number.'
			: unit === 'Tokens' && !Number.isInteger(amountValue)
				? 'Token limits must be whole numbers.'
				: !rolling.trim()
					? 'Window is required, for example 1h, 24h, or 30d.'
					: kind === 'groupBy' && !field.trim()
						? 'Choose the metadata field to group by.'
						: kind === 'oneKey' && !targetKey
							? 'Choose the API key this budget applies to.'
							: null;

	function submit() {
		if (problem) return;
		props.onSave(
			{
				name: name.trim(),
				scope: buildScope(kind, field, selector),
				limit: { unit, amount: amountValue },
				window: { rolling: rolling.trim() },
				onBudgetExceeded: action
			},
			kind === 'oneKey' ? { kind: 'key', key: targetKey } : { kind: 'document' }
		);
	}

	return (
		<Drawer
			title={props.initial.budget.name ? `Edit ${props.initial.budget.name}` : 'New budget'}
			onClose={props.onCancel}
			saving={props.saving}
			footer={
				<div className="button-row">
					<button className="button" type="button" onClick={props.onCancel}>
						Cancel
					</button>
					<Tooltip content={problem ?? 'Save budget'}>
						<span>
							<ConfigSaveButton disabled={Boolean(problem) || props.saving} onClick={submit}>
								Save budget
							</ConfigSaveButton>
						</span>
					</Tooltip>
				</div>
			}
		>
			<Field
				label="Name"
				hint="Identifies the counter that accumulates usage, so renaming a budget starts it over from zero."
			>
				<input value={name} onChange={event => setName(event.target.value)} />
			</Field>

			<FieldGroup
				label="Applies to"
				hint="Only a single-key budget is stored on the key itself. The other scopes are stored at the top level of the configuration and apply across every API key policy."
			>
				<SegmentedControl
					value={kind}
					options={scopeKinds}
					onChange={setKind}
					ariaLabel="Budget scope"
				/>
			</FieldGroup>

			{kind === 'oneKey' ? (
				<Field
					label="API key"
					hint="The budget is stored on this key, so renaming the key's display name does not reset usage. Moving the budget to another key starts a new counter."
				>
					<Dropdown
						value={targetKey}
						options={props.keys.map(key => ({ value: keyOf(key) ?? '', label: keyLabel(key) }))}
						onChange={setTargetKey}
						ariaLabel="API key"
						placeholder="Select a key"
						searchable
					/>
				</Field>
			) : null}

			{kind === 'groupBy' ? (
				<Field
					label="Metadata field"
					hint="Each distinct value gets its own allowance: with team, keys tagged team=research and team=ops are budgeted separately. Keys without the field are not budgeted. Suggestions come from your configured keys."
				>
					<FreeformCombobox
						ariaLabel="Metadata field"
						value={field}
						options={fieldOptions}
						onChange={setField}
						placeholder="group"
					/>
				</Field>
			) : null}

			{kind === 'selector' ? (
				<FieldGroup
					label="Key selector"
					hint="A key joins the pool when its metadata matches every entry here. Leave it empty to pool every key. Changing the selector later does not reset usage already accumulated."
				>
					<div className="api-key-budget-list">
						{selector.map((row, index) => (
							<div className="api-key-budget-form" key={`selector-${index}`}>
								<Field label="Field">
									<FreeformCombobox
										ariaLabel="Selector field"
										value={row.field}
										options={fieldOptions}
										onChange={value => setSelector(updateRow(selector, index, { field: value }))}
									/>
								</Field>
								<Field label="Value">
									<FreeformCombobox
										ariaLabel="Selector value"
										value={row.value}
										options={metadataValues(props.keys, row.field)}
										onChange={value => setSelector(updateRow(selector, index, { value }))}
									/>
								</Field>
								<button
									className="table-action danger"
									type="button"
									aria-label="Remove selector entry"
									onClick={() => setSelector(selector.filter((_, i) => i !== index))}
								>
									<Trash2 size={14} />
								</button>
							</div>
						))}
						<button
							className="button"
							type="button"
							onClick={() => setSelector([...selector, { field: '', value: '' }])}
						>
							<Plus size={16} />
							Add match
						</button>
					</div>
				</FieldGroup>
			) : null}

			<Field
				label="Limit unit"
				hint="A response that reports neither cost nor tokens cannot be charged, and is logged instead."
			>
				<SegmentedControl
					value={unit}
					options={[
						{
							value: 'USD' as const,
							label: 'USD',
							description: 'Charges the cost of each response.'
						},
						{
							value: 'Tokens' as const,
							label: 'Tokens',
							description: 'Charges prompt plus completion tokens.'
						}
					]}
					onChange={setUnit}
					ariaLabel="Limit unit"
				/>
			</Field>

			<Field
				label="Limit"
				hint={
					unit === 'USD'
						? 'The most that may be spent in one window, in dollars.'
						: 'The most tokens that may be used in one window, as a whole number.'
				}
			>
				<input
					inputMode="decimal"
					value={amount}
					onChange={event => setAmount(event.target.value)}
				/>
			</Field>

			<Field
				label="Window"
				hint="How long usage accumulates before it resets, for example 1h, 24h, or 30d. Windows align to the Unix epoch, so 24h starts at midnight UTC rather than at the first request."
			>
				<input value={rolling} onChange={event => setRolling(event.target.value)} />
			</Field>

			<Field
				label="When exceeded"
				hint="Applies from the moment the counter passes the limit until the window resets."
			>
				<SegmentedControl
					value={action}
					options={[
						{
							value: 'Audit' as const,
							label: 'Audit',
							description: 'Records the overage and lets the request through.'
						},
						{
							value: 'Block' as const,
							label: 'Block',
							description: 'Rejects the request with 429 Too Many Requests.'
						}
					]}
					onChange={setAction}
					ariaLabel="Action when exceeded"
				/>
			</Field>
		</Drawer>
	);
}

function budgetRows(config: GatewayConfig | undefined, keys: VirtualApiKey[]): BudgetRow[] {
	const rows: BudgetRow[] = (config?.budgets ?? []).map(budget => ({
		budget,
		source: { kind: 'document' as const }
	}));
	for (const key of keys) {
		for (const budget of key.budgets ?? []) {
			rows.push({ budget, source: { kind: 'key', key: keyOf(key) ?? '' } });
		}
	}
	return rows;
}

function removeBudget(draft: GatewayConfig, row: BudgetRow) {
	if (row.source.kind === 'document') {
		draft.budgets = (draft.budgets ?? []).filter((item: Budget) => item.name !== row.budget.name);
		return;
	}
	const key = findKey(draft, row.source.key);
	if (key)
		key.budgets = (key.budgets ?? []).filter((item: Budget) => item.name !== row.budget.name);
}

function addBudget(draft: GatewayConfig, source: Source, budget: Budget) {
	if (source.kind === 'document') {
		draft.budgets = [...(draft.budgets ?? []), budget];
		return;
	}
	const key = findKey(draft, source.key);
	if (!key) throw new Error('The selected API key no longer exists.');
	// Storing the budget on the key already scopes it, so the scope field is never written out.
	const { scope: _scope, ...rest } = budget;
	upsertVirtualKey(draft, { ...key, budgets: [...(key.budgets ?? []), rest] });
}

function findKey(draft: GatewayConfig, value: string) {
	return getApiKeyPolicy(draft).keys.find(key => keyOf(key) === value);
}

function keyOf(key: VirtualApiKey | undefined) {
	return key ? keyValue(key) : undefined;
}

function newBudget(): Budget {
	return {
		name: '',
		scope: 'perKey',
		limit: { unit: 'USD', amount: 100 },
		window: { rolling: '24h' },
		onBudgetExceeded: 'Block'
	};
}

function rowScopeKind(row: BudgetRow): ScopeKind {
	if (row.source.kind === 'key') return 'oneKey';
	const scope = row.budget.scope;
	if (!scope || scope === 'perKey') return 'perKey';
	return 'groupBy' in scope ? 'groupBy' : 'selector';
}

function rowKey(row: BudgetRow) {
	const owner = row.source.kind === 'key' ? row.source.key : 'document';
	return `${owner}:${row.budget.name}`;
}

function groupByField(budget: Budget) {
	return budget.scope && typeof budget.scope === 'object' && 'groupBy' in budget.scope
		? budget.scope.groupBy
		: '';
}

function selectorRows(budget: Budget) {
	if (!budget.scope || typeof budget.scope !== 'object' || !('selector' in budget.scope)) return [];
	return Object.entries(budget.scope.selector).map(([field, value]) => ({ field, value }));
}

function updateRow(
	rows: { field: string; value: string }[],
	index: number,
	patch: Partial<{ field: string; value: string }>
) {
	return rows.map((row, i) => (i === index ? { ...row, ...patch } : row));
}

function buildScope(
	kind: ScopeKind,
	field: string,
	selector: { field: string; value: string }[]
): Budget['scope'] {
	if (kind === 'groupBy') return { groupBy: field.trim() };
	if (kind === 'selector') {
		const match: Record<string, string> = {};
		for (const row of selector) {
			if (row.field.trim()) match[row.field.trim()] = row.value;
		}
		return { selector: match };
	}
	return 'perKey';
}

function scopeSummary(row: BudgetRow, keys: VirtualApiKey[]) {
	if (row.source.kind === 'key') {
		const owner = row.source.key;
		const key = keys.find(item => keyOf(item) === owner);
		return key ? keyLabel(key) : 'Unknown key';
	}
	const kind = rowScopeKind(row);
	if (kind === 'groupBy') return `One allowance per ${groupByField(row.budget)}`;
	if (kind === 'selector') {
		const entries = selectorRows(row.budget);
		if (!entries.length) return 'All keys, pooled';
		return `${entries.map(entry => `${entry.field} = ${entry.value}`).join(', ')}, pooled`;
	}
	return 'Each key separately';
}

/** Reads under the "Live counters" column, where a bare number gives no sense of what it counts. */
function counterCountLabel(counters: BudgetStatus[]) {
	return counters.length ? `${counters.length} active` : 'None yet';
}

/** Metadata field names present on any configured key, used to suggest scopes. */
function metadataFields(keys: VirtualApiKey[]) {
	const fields = new Set<string>();
	for (const key of keys) {
		for (const [field, value] of metadataEntries(key)) {
			if (value !== null && typeof value !== 'object') fields.add(field);
		}
	}
	return [...fields].sort();
}

function metadataValues(keys: VirtualApiKey[], field: string) {
	const wanted = field.trim();
	if (!wanted) return [];
	const values = new Set<string>();
	for (const key of keys) {
		for (const [name, value] of metadataEntries(key)) {
			if (name === wanted && value !== null && typeof value !== 'object') values.add(String(value));
		}
	}
	return [...values].sort();
}

function metadataEntries(key: VirtualApiKey): [string, unknown][] {
	const metadata = key.metadata;
	if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) return [];
	return Object.entries(metadata as Record<string, unknown>);
}

/** Live counters backing one configured budget, matched by name and scope kind. */
function countersFor(row: BudgetRow, status: BudgetStatus[] | undefined) {
	const kind = rowScopeKind(row);
	const wanted = kind === 'oneKey' ? 'perKey' : kind;
	return (status ?? []).filter(item => item.name === row.budget.name && item.scope.kind === wanted);
}

function counterKey(budget: BudgetStatus) {
	const { kind, field, value } = budget.scope;
	return `${kind}:${field ?? ''}:${value ?? ''}:${budget.name}`;
}

function meterLevel(budget: BudgetStatus) {
	if (budget.usage.exceeded) return 'bad';
	return usedFraction(budget) >= 0.8 ? 'warn' : '';
}

function usedFraction(budget: BudgetStatus) {
	const limit = Number(budget.limit.amount);
	if (!Number.isFinite(limit) || limit <= 0) return 0;
	return Math.min(Number(budget.usage.used) / limit, 1);
}

function usedWidth(budget: BudgetStatus) {
	return `${usedFraction(budget) * 100}%`;
}

function amountLabel(amount: string, unit: BudgetStatus['limit']['unit']) {
	const value = Number(amount);
	if (!Number.isFinite(value)) return unit === 'USD' ? '$0' : '0 tokens';
	return unit === 'USD'
		? `$${value.toLocaleString(undefined, { maximumFractionDigits: 9 })}`
		: `${formatNumber(value)} tokens`;
}
