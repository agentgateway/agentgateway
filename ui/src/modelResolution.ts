import { providerLabel, providerReferenceName } from "./config";
import type { LlmModel, LlmProvider, ModelProvider } from "./types";

export function resolveModelName(
  model: LlmModel | undefined,
  explicitModel?: string,
  providers: LlmProvider[] = [],
) {
  void providers;
  if (!model) return explicitModel?.trim() || "";
  if (!isWildcardModelName(model.name)) return model.name;
  const trimmed = explicitModel?.trim() ?? "";
  const prefix = wildcardModelPrefix(model.name);
  if (trimmed) {
    if (prefix && trimmed.startsWith(prefix)) return trimmed;
    return model.name.replace("*", trimmed);
  }
  return model.name;
}

export function modelProviderLabel(
  model: LlmModel,
  providers: LlmProvider[] = [],
) {
  return resolvedProviderLabel(model.provider, providers);
}

export function resolvedProviderLabel(
  provider: ModelProvider,
  providers: LlmProvider[] = [],
  seen = new Set<string>(),
): string {
  const reference = providerReferenceName(provider);
  if (!reference) return providerLabel(provider);
  if (seen.has(reference)) return "custom";
  const shared = providers.find((item) => item.name === reference);
  if (!shared) return "custom";
  seen.add(reference);
  return resolvedProviderLabel(shared.provider, providers, seen);
}

export function isWildcardModelName(name: string) {
  return name.includes("*");
}

export function wildcardModelPrefix(name: string) {
  const wildcardIndex = name.indexOf("*");
  return wildcardIndex >= 0 ? name.slice(0, wildcardIndex) : name;
}

export function wildcardResolvedSuffix(
  targetModel: string,
  wildcardName: string,
  prefix: string,
) {
  if (targetModel === wildcardName || targetModel === prefix) return "";
  if (!prefix) return targetModel === "*" ? "" : targetModel;
  return targetModel.startsWith(prefix)
    ? targetModel.slice(prefix.length)
    : targetModel;
}
