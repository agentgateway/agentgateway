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
	if (scope.kind === 'groupBy') return `${scope.field} = ${scope.value}`;
	return 'All matching keys';
}
