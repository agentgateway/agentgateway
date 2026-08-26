import { requestJson } from '@/api/base';

export interface BudgetScope {
	kind: 'perKey' | 'groupBy' | 'selector';
	field?: string;
	value?: string;
}

export interface BudgetStatus {
	scope: BudgetScope;
	name: string;
	limit: {
		unit: 'USD' | 'Tokens';
		amount: string;
	};
	usage: {
		used: string;
		remaining: string;
		exceeded: boolean;
	};
	window: {
		start: number;
		end: number;
		durationMs: number;
		expired: boolean;
	};
	onBudgetExceeded: 'Audit' | 'Block';
	updatedAt: number;
}

export interface BudgetStatusResponse {
	observedAt: number;
	budgets: BudgetStatus[];
}

export function getBudgetStatus() {
	return requestJson<BudgetStatusResponse>('/api/budgets/status');
}

export function findPerKeyBudget(
	budgets: BudgetStatus[] | undefined,
	apiKeyName: string,
	name: string
) {
	return budgets?.find(
		item => item.scope.kind === 'perKey' && item.scope.value === apiKeyName && item.name === name
	);
}

export function budgetScopeLabel(scope: BudgetScope) {
	if (scope.kind === 'perKey') return scope.value ?? 'One key';
	if (scope.kind === 'groupBy') {
		// Multi-field counters arrive as their fields and values joined with dots. Pair them back up
		// when the two sides line up, falling back to the raw form if a value contained a dot.
		const fields = (scope.field ?? '').split('.');
		const values = (scope.value ?? '').split('.');
		if (fields.length !== values.length) return `${scope.field} = ${scope.value}`;
		return fields.map((field, index) => `${field} = ${values[index]}`).join(', ');
	}
	return 'All matching keys';
}
