/**
 * SWR hooks for data fetching
 */

import useSWR, { type SWRConfiguration } from "swr";
import { fetchConfig, fetchConfigDump } from "./config";
import type { LocalConfig } from "./types";

/**
 * Hook to fetch and cache configuration
 */
export function useConfig(options?: SWRConfiguration<LocalConfig>) {
  return useSWR<LocalConfig>("/config", fetchConfig, {
    revalidateOnFocus: false,
    revalidateOnReconnect: true,
    ...options,
  });
}

/**
 * Hook to fetch config dump
 */
export function useConfigDump(options?: SWRConfiguration<any>) {
  return useSWR<any>("/config_dump", fetchConfigDump, {
    revalidateOnFocus: false,
    revalidateOnReconnect: false,
    ...options,
  });
}

/**
 * Hook to fetch listeners from config
 */
export function useListeners(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);

  const listeners = (data?.binds || []).flatMap((bind) =>
    (bind.listeners || []).map((listener) => ({
      ...listener,
      port: bind.port,
    })),
  );

  return {
    data: listeners,
    error,
    isLoading,
    mutate,
  };
}

/**
 * Hook to fetch routes from config
 */
export function useRoutes(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);

  const routes = (data?.binds || []).flatMap((bind) =>
    (bind.listeners || []).flatMap((listener) =>
      (listener.routes || []).map((route) => ({
        ...route,
        port: bind.port,
        listenerName: listener.name,
      })),
    ),
  );

  return {
    data: routes,
    error,
    isLoading,
    mutate,
  };
}

/**
 * Hook to fetch backends from config
 */
export function useBackends(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);

  const backends = (data?.binds || []).flatMap((bind) =>
    (bind.listeners || []).flatMap((listener) =>
      (listener.routes || []).flatMap((route) =>
        (route.backends || []).map((backend) => ({
          ...(backend as any),
          port: bind.port,
          listenerName: listener.name,
          routeName: route.name,
        })),
      ),
    ),
  );

  return {
    data: backends,
    error,
    isLoading,
    mutate,
  };
}

/**
 * Hook to fetch LLM config from config
 */
export function useLLMConfig(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);
  return {
    data: data?.llm ?? null,
    error,
    isLoading,
    mutate,
  };
}

/**
 * Hook to fetch MCP config from config
 */
export function useMCPConfig(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);
  return {
    data: data?.mcp ?? null,
    error,
    isLoading,
    mutate,
  };
}

/**
 * Hook to fetch policies from config
 */
export function usePolicies(options?: SWRConfiguration<LocalConfig>) {
  const { data, error, isLoading, mutate } = useConfig(options);

  const policies = (data?.binds || []).flatMap((bind) =>
    (bind.listeners || []).flatMap((listener) =>
      (listener.routes || [])
        .filter((route) => route.policies)
        .map((route) => ({
          name: route.name || "Unnamed Route",
          policies: route.policies,
          port: bind.port,
          listenerName: listener.name,
          routeName: route.name,
        })),
    ),
  );

  return {
    data: policies,
    error,
    isLoading,
    mutate,
  };
}
