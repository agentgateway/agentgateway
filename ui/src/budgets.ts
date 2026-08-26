import type { BudgetStatus } from '@/api/budgetsApi';
import type { Budget } from '@/gateway-config';
import type { VirtualApiKey } from '@/types';

export type BudgetScopeKind = 'perKey' | 'groupBy' | 'selector';

export type AppliedBudget = { budget: Budget; inline: boolean };

export function metadataEntries(key: VirtualApiKey): [string, unknown][] {
	const metadata = key.metadata;
	if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) return [];
	return Object.entries(metadata as Record<string, unknown>);
}

export function metadataValue(key: VirtualApiKey, field: string) {
	for (const [name, value] of metadataEntries(key)) {
		if (name !== field) continue;
		if (value === null || typeof value === 'object') return undefined;
		return String(value);
	}
	return undefined;
}

export function budgetScopeKind(budget: Budget): BudgetScopeKind {
	const scope = budget.scope;
	if (!scope || scope === 'perKey') return 'perKey';
	return 'groupBy' in scope ? 'groupBy' : 'selector';
}

export function budgetGroupByFields(budget: Budget) {
	const scope = budget.scope;
	if (!scope || typeof scope !== 'object' || !('groupBy' in scope)) return [];
	return [...scope.groupBy].sort();
}

export function budgetSelectorEntries(budget: Budget) {
	const scope = budget.scope;
	if (!scope || typeof scope !== 'object' || !('selector' in scope)) return [];
	return Object.entries(scope.selector).map(([field, value]) => ({ field, value }));
}

export function budgetCoversKey(budget: Budget, key: VirtualApiKey) {
	switch (budgetScopeKind(budget)) {
		case 'perKey':
			return metadataValue(key, 'name') !== undefined;
		case 'groupBy':
			return budgetGroupByFields(budget).every(field => metadataValue(key, field) !== undefined);
		default:
			return budgetSelectorEntries(budget).every(
				entry => metadataValue(key, entry.field) === entry.value
			);
	}
}

export function budgetsForKey(policyBudgets: Budget[], key: VirtualApiKey): AppliedBudget[] {
	return [
		...(key.budgets ?? []).map(budget => ({ budget, inline: true })),
		...policyBudgets
			.filter(budget => budgetCoversKey(budget, key))
			.map(budget => ({ budget, inline: false }))
	];
}

export function counterForKey(
	status: BudgetStatus[] | undefined,
	key: VirtualApiKey,
	budget: Budget
) {
	const kind = budgetScopeKind(budget);
	return (status ?? []).find(item => {
		if (item.name !== budget.name || item.scope.kind !== kind) return false;
		if (kind === 'perKey') return item.scope.value === metadataValue(key, 'name');
		if (kind === 'selector') return true;
		const fields = budgetGroupByFields(budget);
		const values = fields.map(field => metadataValue(key, field));
		if (values.some(value => value === undefined)) return false;
		return item.scope.field === fields.join('.') && item.scope.value === values.join('.');
	});
}
