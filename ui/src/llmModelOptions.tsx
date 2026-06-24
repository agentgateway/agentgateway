import { GitBranch } from "lucide-react";
import { providerReferenceName } from "./config";
import { ProviderIcon } from "./components/ProviderIcon";
import { modelProviderLabel, resolveModelName } from "./modelResolution";
import type {
  LlmConfig,
  LlmModel,
  LlmVirtualModel,
  ProviderName,
} from "./types";
import type { ReactNode } from "react";

export type LlmModelOption = {
  kind: "model" | "virtual";
  name: string;
  label: ReactNode;
  icon: ReactNode;
  searchText: string;
  model?: LlmModel;
  virtualModel?: LlmVirtualModel;
};

export function llmModelOptions(
  llm: LlmConfig | null | undefined,
): LlmModelOption[] {
  const models = llm?.models ?? [];
  const virtualModels = llm?.virtualModels ?? [];
  const providers = llm?.providers ?? [];
  return [
    ...models.map((model) => {
      const provider = modelProviderLabel(model, providers);
      const reference = providerReferenceName(model.provider);
      return {
        kind: "model" as const,
        name: model.name,
        label: reference ? (
          <span className="select-option-copy">
            <strong>{model.name}</strong>
            <small>{reference}</small>
          </span>
        ) : (
          model.name
        ),
        icon: <ProviderIcon provider={provider as ProviderName} />,
        searchText: `${model.name} ${provider} ${reference ?? ""} provider-backed`,
        model,
      };
    }),
    ...virtualModels.map((model) => ({
      kind: "virtual" as const,
      name: model.name,
      label: (
        <span className="select-option-copy">
          <strong>{model.name}</strong>
          <small>Virtual model</small>
        </span>
      ),
      icon: <GitBranch size={16} />,
      searchText: `${model.name} virtual model`,
      virtualModel: model,
    })),
  ];
}

export function resolveLlmModelOption(
  option: LlmModelOption | undefined,
  explicitModel?: string,
  providers = [] as NonNullable<LlmConfig["providers"]>,
) {
  if (!option) return "";
  if (option.kind === "virtual") return option.name;
  return resolveModelName(option.model, explicitModel, providers);
}
