import type { PlaygroundModel } from "./types";

export function extractModels(config: any): PlaygroundModel[] {
  const models: PlaygroundModel[] = [];
  const seen = new Set<string>();

  // 1) Top-level llm.models[] — requests go to the gateway at llm.port
  if (config?.llm?.models) {
    const llmPort = config.llm.port ?? 3000;
    const baseUrl = `http://localhost:${llmPort}`;
    for (const m of config.llm.models) {
      const label = m.name;
      if (!label || seen.has(label)) continue;
      seen.add(label);

      models.push({
        label,
        // params.model is the actual model forwarded to the provider
        defaultModel: m.params?.model ?? "",
        provider: m.provider ?? "unknown",
        baseUrl,
      });
    }
  }

  // 2) binds → listeners → routes → backends → ai providers
  if (config?.binds) {
    for (const bind of config.binds) {
      const port = bind.port;
      for (const listener of bind.listeners ?? []) {
        for (const route of [...(listener.routes ?? []), ...(listener.tcpRoutes ?? [])]) {
          for (const backend of route.backends ?? []) {
            const ai = backend.ai;
            if (!ai) continue;
            const providers = ai.groups
              ? ai.groups.flatMap((g: any) => g.providers ?? [])
              : [ai];
            for (const p of providers) {
              const providerEntry = p.provider ?? p;
              for (const [providerName, providerConfig] of Object.entries(providerEntry)) {
                const model = (providerConfig as any)?.model;
                if (!model || seen.has(model)) continue;
                seen.add(model);
                const baseUrl = `http://localhost:${port}`;
                models.push({
                  label: model,
                  defaultModel: model,
                  provider: providerName,
                  baseUrl,
                });
              }
            }
          }
        }
      }
    }
  }

  return models;
}
