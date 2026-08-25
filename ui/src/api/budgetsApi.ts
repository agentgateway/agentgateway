import { requestJson } from '@/api/base';

/**
 * Which API keys share a budget's counter: a single key, every key with a given metadata value, or
 * every key a selector matched. `field` and `value` are only present where they identify the
 * counter, so `selector` budgets carry neither.
 */
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

/**
 * Finds the live counter for a budget declared on one API key. Group and selector budgets are pooled
 * across keys, so they are reported separately rather than under any single key.
 */
export function findPerKeyBudget(
	budgets: BudgetStatus[] | undefined,
	apiKeyName: string,
	name: string
) {
	return budgets?.find(
		item => item.scope.kind === 'perKey' && item.scope.value === apiKeyName && item.name === name
	);
}

/**
 * Budgets pooled across API keys. These have no single owning key, so they are reported separately
 * rather than under any one key's row.
 */
export function sharedBudgets(budgets: BudgetStatus[] | undefined) {
	return budgets?.filter(item => item.scope.kind !== 'perKey') ?? [];
}

/** Human-readable description of which keys a pooled budget covers. */
export function budgetScopeLabel(scope: BudgetScope) {
	if (scope.kind === 'groupBy') return `${scope.field} = ${scope.value}`;
	return 'All matching keys';
}
