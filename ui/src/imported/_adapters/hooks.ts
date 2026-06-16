// Adapter: wraps our SWR hooks to match John's React Query shape
import useSWR, { mutate as swrMutate } from "swr";
import { useEffect, useState } from "react";
import { fetchConfig } from "../../api/config";
import { cleanupConfig } from "../../api/helpers";
import { post } from "../../api/client";
import type { GatewayConfig } from "../types";

export function useGatewayConfig() {
  const swr = useSWR<GatewayConfig>("/config", fetchConfig as unknown as () => Promise<GatewayConfig>, {
    revalidateOnFocus: false,
    revalidateOnReconnect: true,
  });
  return {
    data: swr.data,
    isLoading: swr.isLoading,
    isError: !!swr.error,
    error: swr.error as Error | undefined,
  };
}

export function useUpdateConfig() {
  const [isPending, setIsPending] = useState(false);
  const [isSuccess, setIsSuccess] = useState(false);
  const [isError, setIsError] = useState(false);
  const [error, setError] = useState<Error | undefined>(undefined);

  async function mutate(
    updater: (config: GatewayConfig) => GatewayConfig | void,
    options?: { onSuccess?: () => void; onError?: (err: Error) => void },
  ) {
    setIsPending(true);
    setIsSuccess(false);
    setIsError(false);
    setError(undefined);
    try {
      const current = (await fetchConfig()) as unknown as GatewayConfig;
      const next = structuredClone(current);
      const returned = updater(next);
      const config = returned ?? next;
      const cleaned = cleanupConfig(config as unknown as Parameters<typeof cleanupConfig>[0]);
      await post("/config", cleaned);
      await swrMutate("/config");
      setIsSuccess(true);
      options?.onSuccess?.();
    } catch (err) {
      const error = err instanceof Error ? err : new Error(String(err));
      setIsError(true);
      setError(error);
      options?.onError?.(error);
      throw err;
    } finally {
      setIsPending(false);
    }
  }

  function reset() {
    setIsPending(false);
    setIsSuccess(false);
    setIsError(false);
    setError(undefined);
  }

  return { mutate, reset, isPending, isSuccess, isError, error };
}

export function useStoredStringState(key: string, defaultValue: string) {
  const [value, setValue] = useState(() => localStorage.getItem(key) ?? defaultValue);
  useEffect(() => { localStorage.setItem(key, value); }, [key, value]);
  return [value, setValue] as const;
}

export function useStoredBooleanState(key: string, defaultValue: boolean) {
  const [value, setValue] = useState(() => {
    const stored = localStorage.getItem(key);
    return stored === null ? defaultValue : stored === "true";
  });
  useEffect(() => { localStorage.setItem(key, String(value)); }, [key, value]);
  return [value, setValue] as const;
}
