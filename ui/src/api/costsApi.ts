import { requestJson } from '@/api/base';
import { forgetConfigGeneration } from '@/api/configResourcesApi';

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

export async function refreshBaseCosts() {
	try {
		return await requestJson<RefreshBaseCostsResponse>('/api/costs/refresh-base', {
			method: 'POST'
		});
	} finally {
		// In hybrid mode this rewrites the modelCatalog resource, moving the store generation.
		forgetConfigGeneration();
	}
}

export function listCostModels() {
	return requestJson<CostCatalogModelsResponse>('/api/costs/models');
}
