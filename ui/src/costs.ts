import { refreshBaseCosts } from "./api/costsApi";
import type { GatewayConfig } from "./types";

export type CostCatalogSource = { file?: string; inline?: unknown };

export function configuredCostSources(
  config: GatewayConfig | undefined,
): CostCatalogSource[] {
  return (config?.config?.modelCatalog ??
    []) as unknown[] as CostCatalogSource[];
}

export function addBaseCostSource(config: GatewayConfig, file: string) {
  config.config = config.config ?? {};
  const existing = configuredCostSources(config);
  if (!existing.some((source) => source.file === file)) {
    config.config.modelCatalog = [
      { file },
      ...existing.filter((source) => source.file !== file),
    ] as never;
  }
}

export async function refreshBaseCostsAndConfigure(updateConfig: {
  mutateAsync: (
    updater: (config: GatewayConfig) => GatewayConfig | void,
  ) => Promise<unknown>;
}) {
  const refreshed = await refreshBaseCosts();
  await updateConfig.mutateAsync((next) =>
    addBaseCostSource(next, refreshed.file),
  );
  return refreshed;
}
