import { requestJson } from '@/api/base';

export interface BudgetStatus {
	apiKeyName: string;
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
