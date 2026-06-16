import type {
  GatewayConfig,
  LlmApiKeyPolicy,
  LlmConfig,
  LlmModel,
  LlmProvider,
  LlmGuardrail,
  LlmVirtualModel,
  McpConfig,
  McpTarget,
  ModelProvider,
  ProviderName,
  VirtualApiKey,
} from "./types";

export const providerNames: ProviderName[] = [
  "openAI",
  "anthropic",
  "gemini",
  "vertex",
  "bedrock",
  "azure",
  "copilot",
  "cohere",
  "ollama",
  "baseten",
  "cerebras",
  "deepinfra",
  "deepseek",
  "groq",
  "huggingface",
  "mistral",
  "openrouter",
  "togetherai",
  "xAI",
  "fireworks",
  "custom",
];

export const coreProviderNames = new Set<ProviderName>([
  "openAI",
  "anthropic",
  "gemini",
  "vertex",
  "bedrock",
  "azure",
  "copilot",
  "custom",
]);

const providerApiKeyDefaults: Partial<Record<ProviderName, string | null>> = {
  openAI: "$OPENAI_API_KEY",
  anthropic: "$ANTHROPIC_API_KEY",
  gemini: "$GEMINI_API_KEY",
  vertex: null,
  bedrock: null,
  azure: null,
  copilot: null,
  cohere: "$COHERE_API_KEY",
  ollama: null,
  baseten: "$BASETEN_API_KEY",
  cerebras: "$CEREBRAS_API_KEY",
  deepinfra: "$DEEPINFRA_API_KEY",
  deepseek: "$DEEPSEEK_API_KEY",
  groq: "$GROQ_API_KEY",
  huggingface: "$HF_TOKEN",
  mistral: "$MISTRAL_API_KEY",
  openrouter: "$OPENROUTER_API_KEY",
  togetherai: "$TOGETHER_API_KEY",
  xAI: "$XAI_API_KEY",
  fireworks: "$FIREWORKS_API_KEY",
  custom: null,
};

const providerApiKeyRequired = new Set<string>(
  Object.entries(providerApiKeyDefaults)
    .filter(([, value]) => Boolean(value))
    .map(([provider]) => provider),
);
providerApiKeyRequired.add("openAI");

export function providerLabel(provider: ModelProvider | null): string {
  if (!provider) return "";
  if (typeof provider === "string") return provider;
  if ("reference" in provider) return "reference";
  return "custom";
}

export function providerReferenceName(provider: ModelProvider | null): string | null {
  if (!provider) return null;
  if (typeof provider === "object" && "reference" in provider) return provider.reference;
  return null;
}

export function providerDisplayName(provider: ProviderName | string): string {
  const names: Record<string, string> = {
    openai: "OpenAI",
    openAI: "OpenAI",
    anthropic: "Anthropic",
    gemini: "Gemini",
    vertex: "Vertex AI",
    bedrock: "Amazon Bedrock",
    azure: "Azure",
    copilot: "GitHub Copilot",
    cohere: "Cohere",
    ollama: "Ollama",
    baseten: "Baseten",
    cerebras: "Cerebras",
    deepinfra: "DeepInfra",
    deepseek: "DeepSeek",
    groq: "Groq",
    huggingface: "Hugging Face",
    mistral: "Mistral AI",
    openrouter: "OpenRouter",
    togetherai: "Together AI",
    xAI: "xAI",
    fireworks: "Fireworks AI",
    custom: "Custom",
  };
  return names[provider] ?? provider;
}

export function visibleProviderNames(showAll: boolean): ProviderName[] {
  const core = providerNames.filter((name) => coreProviderNames.has(name));
  if (!showAll) return core;
  const tail = providerNames
    .filter((name) => !coreProviderNames.has(name))
    .sort((left, right) => providerDisplayName(left).localeCompare(providerDisplayName(right)));
  return [...core, ...tail];
}

export function providerDefaultApiKey(provider: ProviderName): string | null {
  return providerApiKeyDefaults[provider] ?? null;
}

export function providerNeedsApiKey(provider: string): boolean {
  return providerApiKeyRequired.has(provider);
}

export function cloneConfig(config: GatewayConfig): GatewayConfig {
  return structuredClone(config);
}

export function ensureLlm(config: GatewayConfig): LlmConfig {
  if (!config.llm) {
    config.llm = { models: [] };
  }
  if (!config.llm.port) {
    config.llm.port = 4000;
  }
  if (!Array.isArray(config.llm.models)) {
    config.llm.models = [];
  }
  if (!Array.isArray(config.llm.providers)) {
    config.llm.providers = [];
  }
  if (!Array.isArray(config.llm.virtualModels)) {
    config.llm.virtualModels = [];
  }
  return config.llm;
}

export function ensureMcp(config: GatewayConfig): McpConfig {
  if (!config.mcp) {
    config.mcp = { targets: [] };
  }
  if (!Array.isArray(config.mcp.targets)) {
    config.mcp.targets = [];
  }
  if (!config.mcp.port) {
    config.mcp.port = 4001;
  }
  return config.mcp;
}

export function upsertModel(config: GatewayConfig, model: LlmModel, previousName?: string) {
  const llm = ensureLlm(config);
  const index = llm.models.findIndex((item) => item.name === (previousName ?? model.name));
  if (index >= 0) {
    llm.models[index] = model;
  } else {
    llm.models.push(model);
  }
}

export function removeModel(config: GatewayConfig, name: string) {
  const llm = ensureLlm(config);
  llm.models = llm.models.filter((model) => model.name !== name);
}

export function upsertVirtualModel(config: GatewayConfig, model: LlmVirtualModel, previousName?: string) {
  const llm = ensureLlm(config);
  const index = llm.virtualModels?.findIndex((item) => item.name === (previousName ?? model.name)) ?? -1;
  if (index >= 0 && llm.virtualModels) {
    llm.virtualModels[index] = model;
  } else {
    llm.virtualModels?.push(model);
  }
}

export function removeVirtualModel(config: GatewayConfig, name: string) {
  const llm = ensureLlm(config);
  llm.virtualModels = (llm.virtualModels ?? []).filter((model) => model.name !== name);
}

export function upsertLlmProvider(config: GatewayConfig, provider: LlmProvider, previousName?: string) {
  const llm = ensureLlm(config);
  const index = llm.providers?.findIndex((item) => item.name === (previousName ?? provider.name)) ?? -1;
  if (index >= 0 && llm.providers) {
    llm.providers[index] = provider;
  } else {
    llm.providers?.push(provider);
  }
}

export function removeLlmProvider(config: GatewayConfig, name: string) {
  const llm = ensureLlm(config);
  llm.providers = (llm.providers ?? []).filter((provider) => provider.name !== name);
}

type LlmPolicyWithGuardrails = NonNullable<LlmConfig["policies"]> & { guardrails?: LlmGuardrail | null };

function ensureLlmPolicies(config: GatewayConfig): LlmPolicyWithGuardrails {
  const llm = ensureLlm(config);
  llm.policies ??= {};
  return llm.policies as LlmPolicyWithGuardrails;
}

export function getLlmGuardrails(config: GatewayConfig | undefined): LlmGuardrail | null {
  return ((config?.llm?.policies as LlmPolicyWithGuardrails | undefined)?.guardrails ?? null) as LlmGuardrail | null;
}

export function setLlmGuardrails(config: GatewayConfig, guardrails: LlmGuardrail | null) {
  const policies = ensureLlmPolicies(config);
  if (guardrails) policies.guardrails = guardrails;
  else delete policies.guardrails;
}

export function upsertMcpTarget(config: GatewayConfig, target: McpTarget, previousName?: string) {
  const mcp = ensureMcp(config);
  const index = mcp.targets.findIndex((item) => item.name === (previousName ?? target.name));
  if (index >= 0) {
    mcp.targets[index] = target;
  } else {
    mcp.targets.push(target);
  }
}

export function removeMcpTarget(config: GatewayConfig, name: string) {
  const mcp = ensureMcp(config);
  mcp.targets = mcp.targets.filter((target) => target.name !== name);
}

export function getApiKeyPolicy(config: GatewayConfig): LlmApiKeyPolicy {
  const policies = ensureLlmPolicies(config);
  policies.apiKey ??= {
    keys: [],
    mode: "strict",
    location: { header: { name: "authorization", prefix: "Bearer " } },
  };
  return policies.apiKey;
}

export function upsertVirtualKey(config: GatewayConfig, key: VirtualApiKey, previousKey?: string) {
  const policy = getApiKeyPolicy(config);
  const index = policy.keys.findIndex((item) => item.key === (previousKey ?? key.key));
  if (index >= 0) {
    policy.keys[index] = key;
  } else {
    policy.keys.push(key);
  }
}

export function removeVirtualKey(config: GatewayConfig, key: string) {
  const policy = getApiKeyPolicy(config);
  policy.keys = policy.keys.filter((item) => item.key !== key);
}

export function modelWarnings(model: LlmModel): string[] {
  const warnings: string[] = [];
  if (!model.provider) warnings.push("Provider is required.");
  const provider = providerLabel(model.provider);
  if (provider === "reference") {
    if (!providerReferenceName(model.provider)) warnings.push("Provider reference is required.");
    const extraParams = Object.keys(model.params ?? {}).filter((key) => key !== "model");
    if (extraParams.length > 0) warnings.push("Referenced models can only set the upstream model.");
    if (!model.name.trim()) warnings.push("Model name is required.");
    return warnings;
  }
  if (!model.name.trim()) warnings.push("Model name is required.");
  if (providerNeedsApiKey(provider) && !model.params?.apiKey && !model.auth) {
    warnings.push("Provider API key is unset; gateway will rely on environment auto-detection.");
  }
  if (provider === "vertex" && (!model.params?.vertexProject || !model.params?.vertexRegion)) {
    warnings.push("Vertex models should set project and region.");
  }
  if (provider === "bedrock" && !model.params?.awsRegion) {
    warnings.push("Bedrock models should set an AWS region.");
  }
  if (provider === "azure" && !model.params?.azureResourceName) {
    warnings.push("Azure models should set a resource name.");
  }
  if (provider === "custom" && model.provider && typeof model.provider !== "string" && "custom" in model.provider && !model.provider.custom.formats.length) {
    warnings.push("Custom providers need at least one supported format.");
  }
  return warnings;
}

export function configWarnings(config: GatewayConfig): string[] {
  const warnings: string[] = [];
  const models = config.llm?.models ?? [];
  const mcpTargets = config.mcp?.targets ?? [];
  if (!config.llm) warnings.push("LLM config is not initialized.");
  const duplicateNames = models
    .map((model) => model.name)
    .filter((name, index, names) => names.indexOf(name) !== index);
  if (duplicateNames.length > 0) warnings.push(`Duplicate model names: ${Array.from(new Set(duplicateNames)).join(", ")}.`);
  const apiPolicy = config.llm?.policies?.apiKey;
  if (apiPolicy?.mode && apiPolicy.mode !== "strict") {
    warnings.push(`Virtual API key mode is ${apiPolicy.mode}; unauthenticated requests may be accepted.`);
  }
  for (const model of models) {
    for (const warning of modelWarnings(model)) {
      warnings.push(`${model.name || "Unnamed model"}: ${warning}`);
    }
  }
  const duplicateMcpTargets = mcpTargets
    .map((target) => target.name)
    .filter((name, index, names) => names.indexOf(name) !== index);
  if (duplicateMcpTargets.length > 0) warnings.push(`Duplicate MCP server names: ${Array.from(new Set(duplicateMcpTargets)).join(", ")}.`);
  return warnings;
}

export function makeEmptyModel(): LlmModel {
  return {
    name: "",
    provider: null,
    params: {
      model: "",
    },
  } as unknown as LlmModel;
}

export function makeEmptyVirtualModel(): LlmVirtualModel {
  return {
    name: "",
    routing: {
      weighted: {
        targets: [{ model: "", weight: 1 }],
      },
    },
  };
}

export function makeEmptyLlmProvider(): LlmProvider {
  return {
    name: "",
    provider: "openai",
    params: {
      apiKey: "$OPENAI_API_KEY",
    },
  };
}

export function makeEmptyMcpTarget(): McpTarget {
  return {
    name: "",
    mcp: {
      host: "localhost",
      port: 8080,
      path: "/mcp",
    },
  };
}
