import { Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { EnumSelector } from '@/components/EnumSelector';
import { Field, FieldGroup } from '@/components/Primitives';
import type { Budget } from '@/gateway-config';
import { ResultingYaml } from '@/policies/ResultingYaml';
import type { SchemaHelp } from '@/schemaHelp';

type ScopeKind = 'perKey' | 'groupBy' | 'selector';

type Draft = {
	name: string;
	scope: ScopeKind;
	fields: string[];
	selector: { field: string; value: string }[];
	unit: 'USD' | 'Tokens';
	amount: string;
	rolling: string;
	onBudgetExceeded: 'Audit' | 'Block';
};

export function BudgetsPolicyEditor(props: {
	formId?: string;
	budgets: Budget[] | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (budgets: Budget[]) => void;
}) {
	const [drafts, setDrafts] = useState<Draft[]>(() => (props.budgets ?? []).map(toDraft));
	const preview = drafts.map(toBudget);

	function update(index: number, patch: Partial<Draft>) {
		setDrafts(drafts.map((draft, i) => (i === index ? { ...draft, ...patch } : draft)));
	}

	return (
		<form
			id={props.formId}
			onSubmit={event => {
				event.preventDefault();
				props.onSave(preview);
			}}
		>
			{drafts.map((draft, index) => (
				<FieldGroup key={index} label={draft.name || `Budget ${index + 1}`}>
					<Field label="Name" hint="Renaming a budget starts its usage from zero.">
						<input value={draft.name} onChange={e => update(index, { name: e.target.value })} />
					</Field>

					<Field label="Applies to">
						<EnumSelector
							value={draft.scope}
							options={[
								{ value: 'perKey', label: 'Each key' },
								{ value: 'groupBy', label: 'Per metadata value' },
								{ value: 'selector', label: 'Shared pool' }
							]}
							onChange={value => update(index, { scope: value as ScopeKind })}
							ariaLabel="Budget scope"
						/>
					</Field>

					{draft.scope === 'groupBy' ? (
						<FieldGroup
							label="Metadata fields"
							hint="One allowance per distinct combination. Keys missing any of these fields are not budgeted."
						>
							{draft.fields.map((field, fieldIndex) => (
								<div className="budget-group-field-row" key={fieldIndex}>
									<Field label={fieldIndex === 0 ? 'Field' : 'And'}>
										<input
											value={field}
											placeholder="group"
											onChange={e =>
												update(index, {
													fields: draft.fields.map((entry, i) =>
														i === fieldIndex ? e.target.value : entry
													)
												})
											}
										/>
									</Field>
									{draft.fields.length > 1 ? (
										<button
											className="table-action danger"
											type="button"
											aria-label="Remove metadata field"
											onClick={() =>
												update(index, {
													fields: draft.fields.filter((_, i) => i !== fieldIndex)
												})
											}
										>
											<Trash2 size={14} />
										</button>
									) : null}
								</div>
							))}
							<button
								className="button"
								type="button"
								onClick={() => update(index, { fields: [...draft.fields, ''] })}
							>
								<Plus size={16} />
								Add field
							</button>
						</FieldGroup>
					) : null}

					{draft.scope === 'selector' ? (
						<FieldGroup label="Key selector" hint="Leave empty to pool every key.">
							{draft.selector.map((row, rowIndex) => (
								<div className="api-key-budget-form" key={rowIndex}>
									<Field label="Field">
										<input
											value={row.field}
											onChange={e =>
												update(index, {
													selector: draft.selector.map((entry, i) =>
														i === rowIndex ? { ...entry, field: e.target.value } : entry
													)
												})
											}
										/>
									</Field>
									<Field label="Value">
										<input
											value={row.value}
											onChange={e =>
												update(index, {
													selector: draft.selector.map((entry, i) =>
														i === rowIndex ? { ...entry, value: e.target.value } : entry
													)
												})
											}
										/>
									</Field>
									<button
										className="table-action danger"
										type="button"
										aria-label="Remove selector entry"
										onClick={() =>
											update(index, {
												selector: draft.selector.filter((_, i) => i !== rowIndex)
											})
										}
									>
										<Trash2 size={14} />
									</button>
								</div>
							))}
							<button
								className="button"
								type="button"
								onClick={() =>
									update(index, { selector: [...draft.selector, { field: '', value: '' }] })
								}
							>
								<Plus size={16} />
								Add match
							</button>
						</FieldGroup>
					) : null}

					<Field label="Limit unit">
						<EnumSelector
							value={draft.unit}
							options={[
								{ value: 'USD', label: 'USD' },
								{ value: 'Tokens', label: 'Tokens' }
							]}
							onChange={value => update(index, { unit: value as 'USD' | 'Tokens' })}
							ariaLabel="Limit unit"
						/>
					</Field>

					<Field label="Limit">
						<input
							inputMode="decimal"
							value={draft.amount}
							onChange={e => update(index, { amount: e.target.value })}
						/>
					</Field>

					<Field label="Window" hint="For example 1h, 24h, or 30d.">
						<input
							value={draft.rolling}
							onChange={e => update(index, { rolling: e.target.value })}
						/>
					</Field>

					<Field label="When exceeded" hint="Audit allows the request; Block rejects it with 429.">
						<EnumSelector
							value={draft.onBudgetExceeded}
							options={[
								{ value: 'Audit', label: 'Audit' },
								{ value: 'Block', label: 'Block' }
							]}
							onChange={value => update(index, { onBudgetExceeded: value as 'Audit' | 'Block' })}
							ariaLabel="Action when exceeded"
						/>
					</Field>

					<button
						className="table-action danger"
						type="button"
						onClick={() => setDrafts(drafts.filter((_, i) => i !== index))}
					>
						<Trash2 size={14} />
						Remove budget
					</button>
				</FieldGroup>
			))}

			<button className="button" type="button" onClick={() => setDrafts([...drafts, newDraft()])}>
				<Plus size={16} />
				Add budget
			</button>

			<ResultingYaml value={preview} />
		</form>
	);
}

function newDraft(): Draft {
	return {
		name: '',
		scope: 'perKey',
		fields: [''],
		selector: [],
		unit: 'USD',
		amount: '100',
		rolling: '24h',
		onBudgetExceeded: 'Block'
	};
}

function toDraft(budget: Budget): Draft {
	const scope = budget.scope;
	const isGroupBy = scope && typeof scope === 'object' && 'groupBy' in scope;
	const isSelector = scope && typeof scope === 'object' && 'selector' in scope;
	return {
		name: budget.name,
		scope: isGroupBy ? 'groupBy' : isSelector ? 'selector' : 'perKey',
		fields: isGroupBy && scope.groupBy.length ? [...scope.groupBy] : [''],
		selector: isSelector
			? Object.entries(scope.selector).map(([field, value]) => ({ field, value }))
			: [],
		unit: budget.limit.unit,
		amount: String(budget.limit.amount),
		rolling: budget.window.rolling,
		onBudgetExceeded: budget.onBudgetExceeded
	};
}

function toBudget(draft: Draft): Budget {
	return {
		name: draft.name.trim(),
		scope: buildScope(draft),
		limit: { unit: draft.unit, amount: Number(draft.amount) },
		window: { rolling: draft.rolling.trim() },
		onBudgetExceeded: draft.onBudgetExceeded
	};
}

function buildScope(draft: Draft): Budget['scope'] {
	if (draft.scope === 'groupBy') {
		// Deduplicated because the gateway stores the fields as a set, so a repeat would be dropped
		// server-side and the preview would stop matching what is written.
		return { groupBy: [...new Set(draft.fields.map(field => field.trim()).filter(Boolean))] };
	}
	if (draft.scope === 'selector') {
		const selector: Record<string, string> = {};
		for (const row of draft.selector) {
			if (row.field.trim()) selector[row.field.trim()] = row.value;
		}
		return { selector };
	}
	return 'perKey';
}
