import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getConfig, getEffectiveConfig, writeConfig } from "./api/configApi";
import {
  deleteConfigResource,
  listConfigResources,
  putConfigResources,
  updateConfigResource,
  type ConfigResourceKind,
  type ConfigResourceValue,
  type PolicyResourceKind,
} from "./api/configResourcesApi";
import { getConfigDump } from "./api/configDumpApi";
import { getRuntimeInfo } from "./api/runtimeApi";
import { cloneConfig, configWarnings } from "./config";
import { validateGatewayConfig } from "./configValidation";
import type { GatewayConfig, LlmApiKeyPolicy, LlmConfig } from "./types";

let hybridFileWriteOverride = false;

export function allowNextHybridFileWrite() {
  hybridFileWriteOverride = true;
}

export function takeHybridFileWriteOverride() {
  const active = hybridFileWriteOverride;
  hybridFileWriteOverride = false;
  return active;
}

export function useHybridFileWriteOverrideKeys() {
  const [active, setActive] = useState(false);
  useEffect(() => {
    const update = (event: KeyboardEvent) =>
      setActive(event.ctrlKey && event.shiftKey);
    const clear = () => setActive(false);
    window.addEventListener("keydown", update);
    window.addEventListener("keyup", update);
    window.addEventListener("blur", clear);
    return () => {
      window.removeEventListener("keydown", update);
      window.removeEventListener("keyup", update);
      window.removeEventListener("blur", clear);
    };
  }, []);
  return active;
}

export function useRawGatewayConfig(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ["config"],
    queryFn: getConfig,
    enabled: options?.enabled ?? true,
    retry: false,
  });
}

export function useEffectiveGatewayConfig(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ["effectiveConfig"],
    queryFn: getEffectiveConfig,
    enabled: options?.enabled ?? true,
    retry: false,
  });
}

export function useRuntimeInfo() {
  return useQuery({
    queryKey: ["runtime"],
    queryFn: getRuntimeInfo,
    retry: false,
  });
}

export function useConfigResources(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ["configResources"],
    queryFn: listConfigResources,
    enabled: options?.enabled ?? true,
    retry: false,
  });
}

export function useLlmConfigData(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true;
  const rawConfig = useRawGatewayConfig({ enabled });
  const config = useEffectiveGatewayConfig({ enabled });
  const runtime = useRuntimeInfo();
  const hybrid = runtime.data?.ui.configStoreMode === "hybrid";
  const configResources = useConfigResources({ enabled: enabled && hybrid });
  const resources = configResources.data?.resources;
  const filePolicies = useMemo(
    () => rawConfig.data?.llm?.policies ?? {},
    [rawConfig.data?.llm?.policies],
  );
  const policies = (config.data?.llm?.policies ?? {}) as NonNullable<
    LlmConfig["policies"]
  >;
  const models = config.data?.llm?.models ?? [];
  const virtualModels = config.data?.llm?.virtualModels ?? [];
  const providers = config.data?.llm?.providers ?? [];
  const apiKeys =
    (policies.apiKey as LlmApiKeyPolicy | null | undefined)?.keys ?? [];
  const warnings = useMemo(
    () =>
      config.data
        ? configWarnings(config.data, {
            models,
            policies,
          })
        : [],
    [config.data, models, policies],
  );

  return {
    config,
    rawConfig,
    runtime,
    hybrid,
    configResources,
    resources,
    filePolicies,
    models,
    virtualModels,
    providers,
    policies,
    apiKeys,
    warnings,
    isLoading:
      config.isLoading ||
      rawConfig.isLoading ||
      runtime.isLoading ||
      configResources.isLoading,
    error:
      config.error ?? rawConfig.error ?? runtime.error ?? configResources.error,
  };
}

export function useMcpConfigData(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true;
  const rawConfig = useRawGatewayConfig({ enabled });
  const config = useEffectiveGatewayConfig({ enabled });
  const runtime = useRuntimeInfo();
  const hybrid = runtime.data?.ui.configStoreMode === "hybrid";
  const configResources = useConfigResources({ enabled: enabled && hybrid });
  const resources = configResources.data?.resources;
  return {
    rawConfig,
    resources,
    hybrid,
    data: config.data,
    isLoading:
      config.isLoading ||
      rawConfig.isLoading ||
      runtime.isLoading ||
      configResources.isLoading,
    error:
      config.error ?? rawConfig.error ?? runtime.error ?? configResources.error,
  };
}

export function useTrafficConfigData(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true;
  const config = useEffectiveGatewayConfig({ enabled });
  const runtime = useRuntimeInfo();
  const hybrid = runtime.data?.ui.configStoreMode === "hybrid";
  const configResources = useConfigResources({ enabled: enabled && hybrid });
  const resources = configResources.data?.resources;
  return {
    config,
    resources,
    hybrid,
    data: config.data,
    isLoading:
      config.isLoading || runtime.isLoading || configResources.isLoading,
    error: config.error ?? runtime.error ?? configResources.error,
  };
}

export function useConfigDumpMode() {
  return useQuery({
    queryKey: ["config_dump_mode"],
    queryFn: async () => {
      try {
        const runtime = await getRuntimeInfo();
        if (runtime.ui.gatewayMode !== "xds")
          return { mode: "local" as const, dump: null };
        const dump = await getConfigDump();
        return { mode: "dump" as const, dump };
      } catch {
        return { mode: "local" as const, dump: null };
      }
    },
    retry: false,
    staleTime: 30_000,
  });
}

function invalidateConfigViews(queryClient: ReturnType<typeof useQueryClient>) {
  void queryClient.invalidateQueries({ queryKey: ["configResources"] });
  void queryClient.invalidateQueries({ queryKey: ["config"] });
  void queryClient.invalidateQueries({ queryKey: ["effectiveConfig"] });
  void queryClient.invalidateQueries({ queryKey: ["runtime"] });
  void queryClient.invalidateQueries({ queryKey: ["config_dump"] });
  void queryClient.invalidateQueries({ queryKey: ["config_dump_mode"] });
}

export function useUpdateConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (
      updater: (config: GatewayConfig) => GatewayConfig | void,
    ) => {
      const runtime =
        queryClient.getQueryData<Awaited<ReturnType<typeof getRuntimeInfo>>>([
          "runtime",
        ]) ?? (await getRuntimeInfo());
      const overrideHybridFileWrite = takeHybridFileWriteOverride();
      if (runtime.ui.configStoreMode === "hybrid" && !overrideHybridFileWrite) {
        throw new Error(
          "File configuration is read-only in hybrid mode. Copy the diff and update the configuration file directly.",
        );
      }
      const current =
        queryClient.getQueryData<GatewayConfig>(["config"]) ??
        (await getConfig());
      const next = cloneConfig(current);
      const returned = updater(next);
      const config = returned ?? next;
      await validateGatewayConfig(config);
      await writeConfig(config);
      return config;
    },
    onSuccess: (next) => {
      queryClient.setQueryData(["config"], next);
      invalidateConfigViews(queryClient);
    },
  });
}

export function useUpsertConfigResource() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: UpsertConfigResourceInput) => {
      if (input.previousId) {
        await updateConfigResource(input.kind, input.previousId, input.value);
        return;
      }
      await putConfigResources(input.kind, [input.value]);
    },
    onSuccess: () => invalidateConfigViews(queryClient),
  });
}

type UpsertConfigResourceInput = {
  [K in ConfigResourceKind]: {
    kind: K;
    value: ConfigResourceValue<K>;
    previousId?: string;
  };
}[ConfigResourceKind];

export function useDeleteConfigResource() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { kind: ConfigResourceKind; id: string }) =>
      deleteConfigResource(input.kind, input.id),
    onSuccess: () => invalidateConfigViews(queryClient),
  });
}

export function useUpsertPolicyResource() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: {
      kind: PolicyResourceKind;
      id: string;
      value: unknown;
    }) => updateConfigResource(input.kind, input.id, input.value),
    onSuccess: () => invalidateConfigViews(queryClient),
  });
}

export function useStoredStringState(key: string, defaultValue: string) {
  const [value, setValue] = useState(
    () => localStorage.getItem(key) ?? defaultValue,
  );
  useEffect(() => {
    localStorage.setItem(key, value);
  }, [key, value]);
  return [value, setValue] as const;
}
