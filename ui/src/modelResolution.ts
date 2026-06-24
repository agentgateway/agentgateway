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

export function selectedConfiguredModelName(
  concreteModel: string,
  models: LlmModel[],
) {
  const exact = models.find((model) => model.name === concreteModel);
  if (exact) return exact.name;
  const wildcard = models
    .map((model, index) => ({ model, index }))
    .filter(
      ({ model }) =>
        isWildcardModelName(model.name) &&
        (concreteModel === wildcardModelPrefix(model.name) ||
          wildcardMatchesModel(model.name, concreteModel)),
    )
    .sort((left, right) => {
      const specificity =
        wildcardSpecificity(right.model.name) -
        wildcardSpecificity(left.model.name);
      return specificity || left.index - right.index;
    })[0]?.model;
  return wildcard?.name ?? models[0]?.name ?? "";
}

export function concreteModelName(
  configuredModelName: string,
  specificModel: string,
) {
  if (!isWildcardModelName(configuredModelName)) return configuredModelName;
  return `${wildcardModelPrefix(configuredModelName)}${specificModel}`;
}

function wildcardMatchesModel(pattern: string, model: string) {
  if (pattern === "*") return Boolean(model.trim());
  const wildcardIndex = pattern.indexOf("*");
  if (wildcardIndex < 0) return pattern === model;
  const prefix = pattern.slice(0, wildcardIndex);
  const suffix = pattern.slice(wildcardIndex + 1);
  return (
    model.startsWith(prefix) &&
    model.endsWith(suffix) &&
    model.length > prefix.length + suffix.length
  );
}

function wildcardSpecificity(pattern: string) {
  const wildcardIndex = pattern.indexOf("*");
  if (wildcardIndex < 0) return Number.MAX_SAFE_INTEGER;
  return pattern.length - 1;
}
