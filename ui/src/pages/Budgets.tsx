import { ChevronDown, Pencil, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

import type { BudgetStatus } from '@/api/budgetsApi';
import { budgetScopeLabel } from '@/api/budgetsApi';
import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
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
import { getApiKeyPolicy, getLlmBudgets, setLlmBudgets, upsertVirtualKey } from '@/config';
import { isServerMetadata, keyLabel, keyValue } from '@/credentialDisplay';
import type { Budget } from '@/gateway-config';
import { useBudgetStatus, useLlmConfigData, useUpdateConfig } from '@/hooks';
import { type SchemaHelp, useSchemaHelp } from '@/schemaHelp';
import type { GatewayConfig, VirtualApiKey } from '@/types';

/** Where a budget is declared, which also decides how it is written back to the configuration. */
type Source = { kind: 'policy' } | { kind: 'key'; key: string };

type BudgetRow = { budget: Budget; source: Source };

type ScopeKind = 'oneKey' | 'perKey' | 'groupBy' | 'selector';

const scopeSchemaPath: Record<Exclude<ScopeKind, 'oneKey'>, Array<string | number>> = {
	perKey: ['$defs', 'BudgetScope', 'oneOf', 0],
	groupBy: ['$defs', 'BudgetScope', 'oneOf', 1],
	selector: ['$defs', 'BudgetScope', 'oneOf', 2]
};

function scopeKinds(help: SchemaHelp): { value: ScopeKind; label: string; description: string }[] {
	return [
		{ value: 'oneKey', label: 'One key budget', description: 'Budget for a specific key' },
		{
			value: 'perKey',
			label: 'Each key budget',
			description: scopeDescription(help, 'perKey', 'Budget applied to each key individually')
		},
		{
			value: 'groupBy',
			label: 'Per api key metadata values',
			description: scopeDescription(
				help,
				'groupBy',
				'Budget applied to each distinct metadata fields'
			)
		},
		{
			value: 'selector',
			label: 'Shared pool',
			description: scopeDescription(help, 'selector', 'Matching keys split one allowance.')
		}
	];
}

function scopeDescription(
	help: SchemaHelp,
	kind: Exclude<ScopeKind, 'oneKey'>,
	fallback: string
): string {
	return help.description(scopeSchemaPath[kind], fallback) ?? fallback;
}

export function BudgetsPage() {
	const { config, apiKeys, isLoading, error } = useLlmConfigData();
	const status = useBudgetStatus();
	const updateConfig = useUpdateConfig();
	const help = useSchemaHelp();
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

	/** The edit itself, shared by the save and by the diff preview so both show the same change. */
	function applyBudget(
		draft: GatewayConfig,
		budget: Budget,
		source: Source,
		original: BudgetRow | null
	) {
		if (original) removeBudget(draft, original);
		addBudget(draft, source, budget);
	}

	function startNew() {
		setEditing({ budget: newBudget(), source: { kind: 'policy' } });
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="Budgets"
				description="Cap LLM spend or token usage for your API keys."
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
					Set config.database.url to use budgets.
				</StatusBanner>
			) : null}

			<BudgetHelp />

			<Panel>
				<div className="section-heading-row">
					<div>
						<h3>Configured budgets</h3>
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
						description="Scope a budget to one key, every key, each value of a metadata field, or a shared pool."
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

			<UsagePanel
				status={status.data?.budgets}
				loading={status.isLoading}
				rows={rows}
				keys={apiKeys}
			/>

			{editing ? (
				<BudgetEditor
					key={rowKey(editing)}
					initial={editing}
					config={config.data}
					help={help}
					keys={apiKeys}
					saving={updateConfig.isPending}
					onCancel={close}
					onApply={(draft, budget, source) =>
						applyBudget(draft, budget, source, editing.budget.name ? editing : null)
					}
					onSave={(budget, source) =>
						write(draft => applyBudget(draft, budget, source, editing.budget.name ? editing : null))
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

/** Collapsed explanation of how a budget behaves, for anyone meeting the page for the first time. */
function BudgetHelp() {
	return (
		<details className="schema-details budget-help">
			<summary>How budgets work</summary>
			<div className="budget-help-copy">
				<p>
					A budget caps how much the API keys it covers may use on LLM requests during a repeating
					window. Keys no budget covers are uncapped.
				</p>
				<ul>
					<li>
						<strong>What is counted.</strong> After every response the budget is charged the cost
						the provider reported, or the prompt plus completion tokens, depending on the unit.
						Responses that report neither are logged and left uncharged.
					</li>
					<li>
						<strong>When traffic stops.</strong> The check runs before a request, against the usage
						recorded so far. A request that starts under the limit always completes, so the total
						can land slightly above it.
					</li>
					<li>
						<strong>Who shares the allowance.</strong> The scope decides that: one key, each key on
						its own, one allowance per value of a metadata field, or a single pool split by every
						matching key. Several budgets can cover the same key, and all of them are charged.
					</li>
					<li>
						<strong>When it resets.</strong> Windows are fixed and aligned to the Unix epoch, not to
						the first request: <code>1h</code> follows UTC clock hours, <code>24h</code> starts at
						midnight UTC, and <code>30d</code> runs in consecutive 30-day periods rather than
						calendar months. Usage returns to zero at every reset.
					</li>
					<li>
						<strong>What editing changes.</strong> Raising or lowering the limit keeps the usage
						already recorded. Changing the window or the unit restarts the counter at zero, and so
						does renaming the budget.
					</li>
				</ul>
			</div>
		</details>
	);
}

/**
 * Live counters for every budget. One configured budget can back many counters, since a
 * per-metadata scope creates one for each distinct value it finds.
 */
function UsagePanel(props: {
	status?: BudgetStatus[];
	loading: boolean;
	rows: BudgetRow[];
	keys: VirtualApiKey[];
}) {
	const budgets = props.status ?? [];
	return (
		<Panel>
			<div className="section-heading-row">
				<div>
					<h3>Current usage</h3>
					<p>Limits can overshoot slightly under concurrent requests.</p>
				</div>
			</div>
			{props.loading ? (
				<StatusBanner state="loading" title="Loading usage" />
			) : budgets.length === 0 ? (
				<EmptyState
					title="No active counters"
					description="Counters appear once a budget matches an API key."
				/>
			) : (
				<div className="table-wrap">
					<table className="keys-table">
						<thead>
							<tr>
								<th>Name</th>
								<th>Applies to</th>
								<th>Keys</th>
								<th>Usage</th>
								<th>Remaining</th>
								<th>Resets</th>
							</tr>
						</thead>
						<tbody>
							{budgets.map((budget, index) => (
								<tr key={counterKey(budget, index)}>
									<td>
										<strong>{budget.name}</strong>
									</td>
									<td className="muted">{budgetScopeLabel(budget.scope)}</td>
									<td>
										<CounterKeyList keys={countingKeys(budget, props.rows, props.keys)} />
									</td>
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

/**
 * The API keys charging one live counter, since a pooled counter can be fed by many keys and the
 * scope alone does not name them. The list expands inside the cell rather than floating over the
 * table, which would otherwise scroll the header row out of view when a lower row is opened.
 */
function CounterKeyList(props: { keys: VirtualApiKey[] }) {
	if (!props.keys.length) return <span className="muted">No matching keys</span>;
	return (
		<details className="counter-key-list">
			<summary aria-label="Keys charging this counter">
				{props.keys.length === 1 ? '1 key' : `${props.keys.length} keys`}
				<ChevronDown size={14} aria-hidden="true" />
			</summary>
			<ul>
				{props.keys.map((key, index) => (
					// A key carrying neither a value nor a hash would otherwise collide with the next one.
					<li key={keyOf(key) || `key-${index}`}>{keyLabel(key)}</li>
				))}
			</ul>
		</details>
	);
}

/**
 * The keys that charge one live counter. A per-key counter names its key, a grouped counter takes
 * every key carrying its metadata value, and a pooled counter takes every key its selector matches.
 */
function countingKeys(counter: BudgetStatus, rows: BudgetRow[], keys: VirtualApiKey[]) {
	const { kind, field, value } = counter.scope;
	if (kind === 'perKey') {
		const declared = rows.find(
			item => item.source.kind === 'key' && item.budget.name === counter.name
		)?.source;
		if (declared?.kind === 'key') {
			const key = keys.find(item => keyOf(item) === declared.key);
			return key ? [key] : [];
		}
		return keys.filter(key => metadataValue(key, 'name') === value);
	}
	if (kind === 'groupBy') {
		// The status API reports a grouped counter as its fields and values joined with dots, so a key
		// belongs to the counter when its own values join to the same string.
		if (!field) return [];
		const fields = field.split('.');
		return keys.filter(key => {
			const values = fields.map(name => metadataValue(key, name));
			return values.every(item => item !== undefined) && values.join('.') === value;
		});
	}
	const row = rows.find(item => item.budget.name === counter.name && rowScopeKind(item) === kind);
	if (!row) return [];
	const selector = selectorRows(row.budget);
	return keys.filter(key =>
		selector.every(entry => metadataValue(key, entry.field) === entry.value)
	);
}

/** One metadata field of a key as a budget scope sees it: scalars compared by their display form. */
function metadataValue(key: VirtualApiKey, field: string) {
	for (const [name, value] of metadataEntries(key)) {
		if (name !== field) continue;
		if (value === null || typeof value === 'object') return undefined;
		return String(value);
	}
	return undefined;
}

function BudgetEditor(props: {
	initial: BudgetRow;
	config?: GatewayConfig;
	help: SchemaHelp;
	keys: VirtualApiKey[];
	saving: boolean;
	onCancel: () => void;
	onApply: (draft: GatewayConfig, budget: Budget, source: Source) => void;
	onSave: (budget: Budget, source: Source) => void;
}) {
	const [submitted, setSubmitted] = useState(false);
	const [name, setName] = useState(props.initial.budget.name);
	const [kind, setKind] = useState<ScopeKind>(rowScopeKind(props.initial));
	const [targetKey, setTargetKey] = useState(
		props.initial.source.kind === 'key' ? props.initial.source.key : (keyOf(props.keys[0]) ?? '')
	);
	const [fields, setFields] = useState(groupByFields(props.initial.budget));
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
					: kind === 'groupBy' && !namedFields(fields).length
						? 'Choose at least one metadata field to group by.'
						: kind === 'groupBy' && hasDuplicate(namedFields(fields))
							? 'Each metadata field can only be used once.'
							: kind === 'oneKey' && !targetKey
								? 'Choose the API key this budget applies to.'
								: null;

	/** The budget the form describes, or null when it is not complete enough to write. */
	function draftBudget(): { budget: Budget; source: Source } | null {
		setSubmitted(true);
		if (problem) return null;
		return {
			budget: {
				name: name.trim(),
				scope: buildScope(kind, fields, selector),
				limit: { unit, amount: amountValue },
				window: { rolling: rolling.trim() },
				onBudgetExceeded: action
			},
			source: kind === 'oneKey' ? { kind: 'key', key: targetKey } : { kind: 'policy' }
		};
	}

	function submit() {
		const next = draftBudget();
		if (!next) return;
		props.onSave(next.budget, next.source);
	}

	return (
		<Drawer
			title={props.initial.budget.name ? `Edit ${props.initial.budget.name}` : 'New budget'}
			onClose={props.onCancel}
			saving={props.saving}
			footer={requestClose => (
				<ConfigDiffSaveActions
					config={props.config}
					diffTitle="Budget config diff"
					saveLabel="Save budget"
					saving={props.saving}
					onCancel={requestClose}
					onSave={submit}
					beforeDiff={() => Boolean(draftBudget())}
					applyDiff={next => {
						const budget = draftBudget();
						if (budget) props.onApply(next, budget.budget, budget.source);
					}}
				/>
			)}
		>
			{submitted && problem ? (
				<StatusBanner state="bad" title="Cannot save budget">
					{problem}
				</StatusBanner>
			) : null}

			<Field
				label="Name"
				hint="Renaming a budget restarts its usage from zero."
				tooltip={props.help.field<Budget>('Budget', 'name')}
			>
				<input value={name} onChange={event => setName(event.target.value)} />
			</Field>

			<FieldGroup label="Applies to" tooltip={props.help.field<Budget>('Budget', 'scope')}>
				<SegmentedControl
					value={kind}
					options={scopeKinds(props.help)}
					onChange={setKind}
					ariaLabel="Budget scope"
				/>
			</FieldGroup>

			{kind === 'oneKey' ? (
				<Field label="API key" hint="Moving the budget to another key restarts its usage.">
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
				<FieldGroup
					label="Metadata fields"
					tooltip={scopeDescription(props.help, 'groupBy', '')}
					hint={
						namedFields(fields).length > 1
							? 'Each distinct combination of these values gets its own allowance. Keys missing any of the fields are not budgeted.'
							: 'Each value gets its own allowance. Keys without the field are not budgeted.'
					}
				>
					<div className="api-key-budget-list">
						{fields.map((entry, index) => (
							<div className="budget-group-field-row" key={`group-field-${index}`}>
								<Field label={index === 0 ? 'Field' : 'And'}>
									<FreeformCombobox
										ariaLabel="Metadata field"
										value={entry}
										options={fieldOptions.filter(
											option => option === entry || !fields.includes(option)
										)}
										onChange={value => setFields(fields.map((f, i) => (i === index ? value : f)))}
										placeholder="group"
									/>
								</Field>
								{fields.length > 1 ? (
									<button
										className="table-action danger"
										type="button"
										aria-label="Remove metadata field"
										onClick={() => setFields(fields.filter((_, i) => i !== index))}
									>
										<Trash2 size={14} />
									</button>
								) : null}
							</div>
						))}
						<button className="button" type="button" onClick={() => setFields([...fields, ''])}>
							<Plus size={16} />
							Add field
						</button>
					</div>
				</FieldGroup>
			) : null}

			{kind === 'selector' ? (
				<FieldGroup
					label="Api Key metadata selector"
					tooltip={scopeDescription(props.help, 'selector', '')}
					hint="A key joins when its metadata matches every entry. Leave empty to pool every key."
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
				hint="Responses reporting neither cost nor tokens are logged instead of charged."
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
				hint={unit === 'USD' ? 'Dollars per window.' : 'Tokens per window.'}
				tooltip={props.help.field<Budget>('Budget', 'limit')}
			>
				<input
					inputMode="decimal"
					value={amount}
					onChange={event => setAmount(event.target.value)}
				/>
			</Field>

			<Field
				label="Window"
				hint="For example 1h, 24h, or 30d. Aligned to the Unix epoch, so 24h starts at midnight UTC."
				tooltip={props.help.field<Budget>('Budget', 'window.rolling')}
			>
				<input value={rolling} onChange={event => setRolling(event.target.value)} />
			</Field>

			<Field
				label="When exceeded"
				hint="Applies until the window resets."
				tooltip={props.help.field<Budget>('Budget', 'onBudgetExceeded')}
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
	const rows: BudgetRow[] = getLlmBudgets(config).map(budget => ({
		budget,
		source: { kind: 'policy' as const }
	}));
	for (const key of keys) {
		for (const budget of key.budgets ?? []) {
			rows.push({ budget, source: { kind: 'key', key: keyOf(key) ?? '' } });
		}
	}
	return rows;
}

function removeBudget(draft: GatewayConfig, row: BudgetRow) {
	if (row.source.kind === 'policy') {
		setLlmBudgets(
			draft,
			getLlmBudgets(draft).filter(item => item.name !== row.budget.name)
		);
		return;
	}
	const key = findKey(draft, row.source.key);
	if (key)
		key.budgets = (key.budgets ?? []).filter((item: Budget) => item.name !== row.budget.name);
}

function addBudget(draft: GatewayConfig, source: Source, budget: Budget) {
	if (source.kind === 'policy') {
		setLlmBudgets(draft, [...getLlmBudgets(draft), budget]);
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
	const owner = row.source.kind === 'key' ? row.source.key : 'policy';
	return `${owner}:${row.budget.name}`;
}

/** The fields a budget groups by, always with at least one row so the editor has something to fill. */
function groupByFields(budget: Budget): string[] {
	const scope = budget.scope;
	if (!scope || typeof scope !== 'object' || !('groupBy' in scope)) return [''];
	return scope.groupBy.length ? [...scope.groupBy] : [''];
}

/** Editor rows the user actually filled in, ignoring the blank one a new row starts as. */
function namedFields(fields: string[]) {
	return fields.map(field => field.trim()).filter(Boolean);
}

function hasDuplicate(values: string[]) {
	return new Set(values).size !== values.length;
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
	fields: string[],
	selector: { field: string; value: string }[]
): Budget['scope'] {
	if (kind === 'groupBy') return { groupBy: namedFields(fields) };
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
	if (kind === 'groupBy')
		return `One allowance per ${namedFields(groupByFields(row.budget)).join(' + ')}`;
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

function metadataFields(keys: VirtualApiKey[]) {
	const fields = new Set<string>();
	for (const key of keys) {
		for (const [field, value] of metadataEntries(key)) {
			if (value !== null && typeof value !== 'object' && !isServerMetadata(field))
				fields.add(field);
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

function counterKey(budget: BudgetStatus, index: number) {
	const { kind, field, value } = budget.scope;
	return `${kind}:${field ?? ''}:${value ?? ''}:${budget.name}:${index}`;
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
