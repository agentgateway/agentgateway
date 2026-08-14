import { providerLabel, providerReferenceName } from "./config";
import type { LlmModel, LlmProvider } from "./types";

/**
 * Claude subscription tokens use the `sk-ant-oat` prefix.
 * Returns true if the value looks like a Claude subscription key.
 */
export function isClaudeSubscriptionKey(apiKey: unknown): boolean {
  return typeof apiKey === "string" && apiKey.startsWith("sk-ant-oat");
}

/**
 * Resolves the literal backend API key string for a model, following
 * provider references if needed.  Returns null when the key is not a
 * plain string (env-var reference, file reference, or absent).
 */
export function resolveBackendApiKey(
  model: LlmModel | undefined,
  providers: LlmProvider[],
): string | null {
  if (!model) return null;

  // Direct key on the model params takes priority.
  const directKey = model.params?.apiKey;
  if (typeof directKey === "string" && directKey.trim()) return directKey;

  // If the model uses a provider reference, look up that provider's key.
  const refName = providerReferenceName(model.provider);
  if (refName) {
    const provider = providers.find((item) => item.name === refName);
    const refKey = provider?.params?.apiKey;
    if (typeof refKey === "string" && refKey.trim()) return refKey;
  }

  return null;
}

/**
 * Returns a user-facing warning string if the selected model is backed
 * by a Claude subscription key, or null when no warning applies.
 */
export function claudeSubscriptionWarning(
  model: LlmModel | undefined,
  providers: LlmProvider[],
): string | null {
  if (!model) return null;

  const provider = providerLabel(model.provider);
  const refName = providerReferenceName(model.provider);
  const resolvedProvider = refName
    ? providerLabel(
        providers.find((item) => item.name === refName)?.provider ?? null,
      )
    : provider;

  if (resolvedProvider !== "anthropic") return null;

  const apiKey = resolveBackendApiKey(model, providers);
  if (!isClaudeSubscriptionKey(apiKey)) return null;

  return (
    "This model uses a Claude subscription key (sk-ant-oat…). " +
    "Anthropic maybe reject clients other than Claude Code " +
    "for Sonnet and Opus models with subscription key."
  );
}
