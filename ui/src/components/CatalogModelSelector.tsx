import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { listCostModels } from "../api/costsApi";
import { FreeformCombobox } from "./FreeformCombobox";

export function CatalogModelSelector(props: {
  ariaLabel: string;
  value: string;
  onChange: (value: string) => void;
  provider?: string | null;
  placeholder?: string;
}) {
  const catalog = useQuery({
    queryKey: ["cost-models"],
    queryFn: listCostModels,
    staleTime: 60_000,
    retry: false,
  });
  const options = useMemo(() => {
    const providers = catalog.data?.providers ?? [];
    const keys = catalogProviderKeys(props.provider);
    const matched = keys.length
      ? providers.filter((provider) =>
          keys.includes(provider.provider.toLowerCase()),
        )
      : providers;
    const source = matched.length ? matched : [];
    return [...new Set(source.flatMap((provider) => provider.models))].sort(
      (a, b) => a.localeCompare(b),
    );
  }, [catalog.data, props.provider]);

  return (
    <FreeformCombobox
      ariaLabel={props.ariaLabel}
      value={props.value}
      options={options}
      onChange={props.onChange}
      placeholder={props.placeholder ?? "Select or type a model"}
      emptyText={
        catalog.isLoading
          ? "Loading model catalog..."
          : "No catalog matches. Custom model names are allowed."
      }
    />
  );
}

function catalogProviderKeys(provider: string | null | undefined) {
  const normalized = (provider ?? "").trim().toLowerCase();
  const aliases: Record<string, string[]> = {
    bedrock: ["aws.bedrock"],
    gemini: ["gcp.gemini", "google"],
    google: ["gcp.gemini", "google"],
    vertex: ["gcp.vertex_ai", "google-vertex"],
    "vertex ai": ["gcp.vertex_ai", "google-vertex"],
    openai: ["openai"],
    openaii: ["openai"],
    openai_legacy: ["openai"],
    anthropic: ["anthropic"],
    azure: ["azure"],
    copilot: ["copilot"],
  };
  return [normalized, ...(aliases[normalized] ?? [])]
    .filter(Boolean)
    .map((item) => item.toLowerCase());
}
