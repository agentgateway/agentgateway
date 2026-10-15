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

export interface CostRecommendationsTimeRange {
	from: string;
	to: string;
}

export interface CostRecommendation {
	provider: string;
	model: string;
	requestCount: number;
	lastSeen: string;
}

export interface CostRecommendationsResponse {
	timeRange: CostRecommendationsTimeRange;
	scannedRequests: number;
	truncated: boolean;
	candidates: CostRecommendation[];
}

export function refreshBaseCosts() {
	return requestJson<RefreshBaseCostsResponse>('/api/costs/refresh-base', {
		method: 'POST'
	});
}

export function listCostModels() {
	return requestJson<CostCatalogModelsResponse>('/api/costs/models');
}

export function recommendCostModels(timeRange: CostRecommendationsTimeRange) {
	return requestJson<CostRecommendationsResponse>('/api/costs/recommendations', {
		method: 'POST',
		body: JSON.stringify({ timeRange })
	});
}
