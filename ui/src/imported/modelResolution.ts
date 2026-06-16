import { providerLabel, providerReferenceName } from "./config";
import type { LlmModel, LlmProvider, ModelProvider, ProviderName } from "./types";

const providerDefaultModels: Partial<Record<ProviderName, string>> = {
  openai: "gpt-5.4-nano",
  anthropic: "claude-haiku-4-5",
  gemini: "gemini-2.5-flash",
  vertex: "gemini-2.5-flash",
  bedrock: "anthropic.claude-3-5-haiku-20241022-v1:0",
  azure: "gpt-5.4-nano",
  copilot: "gpt-5.4-nano",
  cohere: "command-a-03-2025",
  ollama: "llama3.2",
  baseten: "openai/gpt-oss-120b",
  cerebras: "llama-3.3-70b",
  deepinfra: "meta-llama/Meta-Llama-3.1-8B-Instruct",
  deepseek: "deepseek-chat",
  groq: "llama-3.1-8b-instant",
  huggingface: "meta-llama/Llama-3.1-8B-Instruct",
  mistral: "mistral-small-latest",
  openrouter: "openai/gpt-4o-mini",
  togetherai: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
  xAI: "grok-3-mini",
  fireworks: "accounts/fireworks/models/llama-v3p1-8b-instruct",
  custom: "llama3.2",
};

export function resolveModelName(model: LlmModel | undefined, explicitModel?: string, providers: LlmProvider[] = []) {
  if (!model) return explicitModel?.trim() || "";
  if (!isWildcardModelName(model.name)) return model.name;
  const trimmed = explicitModel?.trim() ?? "";
  const prefix = wildcardModelPrefix(model.name);
  if (trimmed) {
    if (prefix && trimmed.startsWith(prefix)) return trimmed;
    return model.name.replace("*", trimmed);
  }
  return model.name.replace("*", providerDefaultModel(modelProviderLabel(model, providers)));
}

export function providerDefaultModel(provider: ProviderName | string) {
  return providerDefaultModels[provider as ProviderName] ?? "gpt-5.4-nano";
}

export function modelProviderLabel(model: LlmModel, providers: LlmProvider[] = []) {
  return resolvedProviderLabel(model.provider ?? "custom", providers);
}

export function resolvedProviderLabel(provider: ModelProvider, providers: LlmProvider[] = [], seen = new Set<string>()): string {
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

export function wildcardResolvedSuffix(targetModel: string, wildcardName: string, prefix: string) {
  if (targetModel === wildcardName || targetModel === prefix) return "";
  if (!prefix) return targetModel === "*" ? "" : targetModel;
  return targetModel.startsWith(prefix) ? targetModel.slice(prefix.length) : targetModel;
}
