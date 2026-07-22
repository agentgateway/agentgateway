import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getConfig, writeConfig } from "./api/configApi";
import {
  deleteConfigResource,
  listConfigResources,
  putConfigResources,
  updateConfigResource,
  type ConfigResourceKind,
  type ConfigResourceValue,
} from "./api/configResourcesApi";
import { getConfigDump } from "./api/configDumpApi";
import { getRuntimeInfo } from "./api/runtimeApi";
import {
  cloneConfig,
  llmApiKeyResources,
  llmModelResources,
  llmProviderResources,
  llmVirtualModelResources,
} from "./config";
import { validateGatewayConfig } from "./configValidation";
import type { GatewayConfig } from "./types";

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

export function useGatewayConfig(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ["config"],
    queryFn: getConfig,
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
  const config = useGatewayConfig({ enabled });
  const runtime = useRuntimeInfo();
  const hybrid = runtime.data?.ui.configStoreMode === "hybrid";
  const configResources = useConfigResources({ enabled: enabled && hybrid });
  const resources = configResources.data?.resources;
  const models = useMemo(
    () => [
      ...(config.data?.llm?.models ?? []),
      ...(hybrid ? llmModelResources(resources) : []),
    ],
    [config.data, hybrid, resources],
  );
  const virtualModels = useMemo(
    () => [
      ...(config.data?.llm?.virtualModels ?? []),
      ...(hybrid ? llmVirtualModelResources(resources) : []),
    ],
    [config.data, hybrid, resources],
  );
  const providers = useMemo(
    () => [
      ...(config.data?.llm?.providers ?? []),
      ...(hybrid ? llmProviderResources(resources) : []),
    ],
    [config.data, hybrid, resources],
  );
  const apiKeys = useMemo(
    () => [
      ...(config.data?.llm?.policies?.apiKey?.keys ?? []),
      ...(hybrid ? llmApiKeyResources(resources) : []),
    ],
    [config.data, hybrid, resources],
  );

  return {
    config,
    runtime,
    hybrid,
    configResources,
    resources,
    models,
    virtualModels,
    providers,
    apiKeys,
    isLoading:
      config.isLoading ||
      runtime.isLoading ||
      (hybrid && configResources.isLoading),
    error: config.error ?? (hybrid ? configResources.error : null),
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
      if (input.kind === "llm.apiKey" && input.previousId) {
        await updateConfigResource(input.kind, input.previousId, input.value);
        return;
      }
      await putConfigResources(input.kind, [input.value]);
      if (input.kind === "llm.apiKey") return;
      const id = configResourceId(input.kind, input.value);
      if (input.previousId && input.previousId !== id) {
        await deleteConfigResource(input.kind, input.previousId);
      }
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

export function configResourceId<K extends ConfigResourceKind>(
  kind: K,
  value: ConfigResourceValue<K>,
) {
  if (
    kind === "llm.provider" ||
    kind === "llm.model" ||
    kind === "llm.virtualModel"
  ) {
    const name =
      (value as { id?: string; name?: string }).id ??
      (value as { name?: string }).name;
    if (name) return name;
  }
  if (kind === "modelCatalog") return "default";
  const apiKeyId = (value as { metadata?: { id?: string } })?.metadata?.id;
  if (apiKeyId) return apiKeyId;
  throw new Error(`Cannot derive config resource id for ${kind}`);
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
