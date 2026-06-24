import { requestJson } from "./base";

export interface RefreshBaseCostsResponse {
  file: string;
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
  return requestJson<RefreshBaseCostsResponse>("/api/costs/refresh-base", {
    method: "POST",
  });
}

export function listCostModels() {
  return requestJson<CostCatalogModelsResponse>("/api/costs/models");
}
