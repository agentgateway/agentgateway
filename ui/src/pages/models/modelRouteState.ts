import { makeEmptyModel } from "../../config";
import type { LlmModel } from "../../types";

export type ModelHash =
  | { kind: "edit"; modelId: string }
  | { kind: "add"; type: "model" | "virtual" };

export function modelHashFromUrl(): ModelHash | null {
  const raw = decodeURIComponent(window.location.hash.replace(/^#/, ""));
  if (!raw) return null;
  if (raw.startsWith("edit=")) {
    const modelId = raw.slice("edit=".length);
    return modelId ? { kind: "edit", modelId } : null;
  }
  if (raw === "add=model") return { kind: "add", type: "model" };
  if (raw === "add=virtual") return { kind: "add", type: "virtual" };
  if (raw.startsWith("policies=")) {
    const modelId = raw.slice("policies=".length);
    return modelId ? { kind: "edit", modelId } : null;
  }
  if (raw.startsWith("model=")) {
    const modelId = raw.slice("model=".length);
    return modelId ? { kind: "edit", modelId } : null;
  }
  if (raw.startsWith("modelPolicy=")) {
    const modelId = raw.slice("modelPolicy=".length);
    return modelId ? { kind: "edit", modelId } : null;
  }
  return null;
}

export function setModelHash(
  value: ModelHash | null,
  mode: "push" | "replace",
) {
  const hash = value
    ? value.kind === "edit"
      ? `#edit=${encodeURIComponent(value.modelId)}`
      : `#add=${value.type}`
    : "";
  const nextUrl = `${window.location.pathname}${window.location.search}${hash}`;
  if (
    nextUrl ===
    `${window.location.pathname}${window.location.search}${window.location.hash}`
  )
    return;
  if (mode === "push") {
    window.history.pushState(null, "", nextUrl);
  } else {
    window.history.replaceState(null, "", nextUrl);
  }
}

export function clearModelSearch() {
  if (!window.location.search) return;
  window.history.replaceState(
    null,
    "",
    `${window.location.pathname}${window.location.hash}`,
  );
}

export function providerFromUrl(): string | null {
  const provider = new URLSearchParams(window.location.search)
    .get("provider")
    ?.trim();
  return provider || null;
}

export function modelFromProviderReference(providerName: string): LlmModel {
  return {
    ...makeEmptyModel(),
    provider: { reference: providerName },
    params: undefined,
  };
}
