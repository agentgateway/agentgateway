import { requestJson } from '@/api/base';

export interface RefreshBaseCostsResponse {
	file?: string;
	providers: number;
	models: number;
}

export interface CostCatalogModelsResponse {
	loaded: boolean;
	providers: Array<{
		provider: string;
		models: string[];
	}>;
}

export function refreshBaseCosts() {
	// Keep the old generation until resources are refetched, even if the refresh succeeds.
	// A failed refresh or refetch must not allow stale UI data to be written unpinned.
	return requestJson<RefreshBaseCostsResponse>('/api/costs/refresh-base', {
		method: 'POST'
	});
}

export function listCostModels() {
	return requestJson<CostCatalogModelsResponse>('/api/costs/models');
}
