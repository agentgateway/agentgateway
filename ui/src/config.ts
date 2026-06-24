import type {
  GatewayConfig,
  LlmApiKeyPolicy,
  LlmConfig,
  LlmModel,
  LlmProvider,
  LlmGuardrail,
  LlmParams,
  LlmVirtualModel,
  McpConfig,
  McpTarget,
  ModelProvider,
  ProviderName,
  VirtualApiKey,
} from "./types";
import { keyValue } from "./credentialDisplay";

const promptLogKey = "gen_ai.prompt";
const completionLogKey = "gen_ai.completion";
const promptLogExpression = "llm.prompt";
const completionLogExpression =
  'llm.completion.map(c, {"role":"assistant", "content": c})';

export const providerNames: ProviderName[] = [
  "openai",
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
  "xai",
  "fireworks",
  "custom",
];

export const coreProviderNames = new Set<ProviderName>([
  "openai",
  "anthropic",
  "gemini",
  "vertex",
  "bedrock",
  "azure",
  "copilot",
  "custom",
]);

export function providerLabel(provider: ModelProvider | null): string {
  if (!provider) return "";
  if (typeof provider === "string") return provider;
  if ("reference" in provider) return "reference";
  return "custom";
}

export function providerReferenceName(
  provider: ModelProvider | null,
): string | null {
  if (!provider) return null;
  if (typeof provider === "object" && "reference" in provider)
    return provider.reference;
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
    xai: "xAI",
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
    .sort((left, right) =>
      providerDisplayName(left).localeCompare(providerDisplayName(right)),
    );
  return [...core, ...tail];
}

export function invalidProviderApiKey(value: LlmParams["apiKey"] | undefined) {
  if (value === null || value === undefined) return false;
  if (typeof value === "string") return !value.trim() || value.trim() === "$";
  if (typeof value === "object" && "file" in value) return !value.file.trim();
  return false;
}

export function cloneConfig(config: GatewayConfig): GatewayConfig {
  return structuredClone(config);
}

export function ensureLlm(config: GatewayConfig): LlmConfig {
  if (!config.llm) {
    config.llm = { models: [] };
    ensureLlmFrontendDefaults(config);
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

export function ensureLlmFrontendDefaults(config: GatewayConfig) {
  config.frontendPolicies ??= {};
  if (!config.frontendPolicies.http) {
    config.frontendPolicies.http = {
      // Raise the global body-buffer cap above the 2Mi default so the LLM filter
      // can read ~800k-1M-token request bodies (about 3-4 MB JSON) without rejecting
      // them as AIError::RequestTooLarge.
      maxBufferSize: 33554432,
    };
  }
}

export function ensureMcp(config: GatewayConfig): McpConfig {
  if (!config.mcp) {
    config.mcp = { targets: [] };
  }
  if (!Array.isArray(config.mcp.targets)) {
    config.mcp.targets = [];
  }
  return config.mcp;
}

export function upsertModel(
  config: GatewayConfig,
  model: LlmModel,
  previousName?: string,
) {
  const llm = ensureLlm(config);
  const index = llm.models.findIndex(
    (item) => item.name === (previousName ?? model.name),
  );
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

export function upsertVirtualModel(
  config: GatewayConfig,
  model: LlmVirtualModel,
  previousName?: string,
) {
  const llm = ensureLlm(config);
  const index =
    llm.virtualModels?.findIndex(
      (item) => item.name === (previousName ?? model.name),
    ) ?? -1;
  if (index >= 0 && llm.virtualModels) {
    llm.virtualModels[index] = model;
  } else {
    llm.virtualModels?.push(model);
  }
}

export function removeVirtualModel(config: GatewayConfig, name: string) {
  const llm = ensureLlm(config);
  llm.virtualModels = (llm.virtualModels ?? []).filter(
    (model) => model.name !== name,
  );
}

export function upsertLlmProvider(
  config: GatewayConfig,
  provider: LlmProvider,
  previousName?: string,
) {
  const llm = ensureLlm(config);
  const index =
    llm.providers?.findIndex(
      (item) => item.name === (previousName ?? provider.name),
    ) ?? -1;
  if (index >= 0 && llm.providers) {
    llm.providers[index] = provider;
  } else {
    llm.providers?.push(provider);
  }
}

export function removeLlmProvider(config: GatewayConfig, name: string) {
  const llm = ensureLlm(config);
  llm.providers = (llm.providers ?? []).filter(
    (provider) => provider.name !== name,
  );
}

type LlmPolicyWithGuardrails = NonNullable<LlmConfig["policies"]> & {
  guardrails?: LlmGuardrail | null;
};

function ensureLlmPolicies(config: GatewayConfig): LlmPolicyWithGuardrails {
  const llm = ensureLlm(config);
  llm.policies ??= {};
  return llm.policies as LlmPolicyWithGuardrails;
}

export function getLlmGuardrails(
  config: GatewayConfig | undefined,
): LlmGuardrail | null {
  return ((config?.llm?.policies as LlmPolicyWithGuardrails | undefined)
    ?.guardrails ?? null) as LlmGuardrail | null;
}

export function setLlmGuardrails(
  config: GatewayConfig,
  guardrails: LlmGuardrail | null,
) {
  const policies = ensureLlmPolicies(config);
  if (guardrails) policies.guardrails = guardrails;
  else delete policies.guardrails;
}

export function upsertMcpTarget(
  config: GatewayConfig,
  target: McpTarget,
  previousName?: string,
) {
  const mcp = ensureMcp(config);
  const index = mcp.targets.findIndex(
    (item) => item.name === (previousName ?? target.name),
  );
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

export function upsertVirtualKey(
  config: GatewayConfig,
  key: VirtualApiKey,
  previousKey?: string,
) {
  const policy = getApiKeyPolicy(config);
  const index = policy.keys.findIndex(
    (item) => keyValue(item) === (previousKey ?? keyValue(key)),
  );
  if (index >= 0) {
    policy.keys[index] = key;
  } else {
    policy.keys.push(key);
  }
}

export function removeVirtualKey(config: GatewayConfig, key: string) {
  const policy = getApiKeyPolicy(config);
  policy.keys = policy.keys.filter((item) => keyValue(item) !== key);
}

export function disableApiKeyPolicy(config: GatewayConfig) {
  if (!config.llm?.policies) return;
  delete config.llm.policies.apiKey;
  if (Object.keys(config.llm.policies).length === 0) delete config.llm.policies;
}

export function promptCompletionLoggingEnabled(
  config: GatewayConfig | undefined,
) {
  const accessLog = config?.frontendPolicies?.accessLog as
    | {
        add?: Record<string, string>;
        database?: { add?: Record<string, string> };
      }
    | undefined;
  const add = accessLog?.database?.add ?? accessLog?.add;
  return Boolean(add && promptLogKey in add && completionLogKey in add);
}

export function setPromptCompletionLogging(
  config: GatewayConfig,
  enabled: boolean,
) {
  if (!config.frontendPolicies) config.frontendPolicies = {};
  const frontendPolicies = config.frontendPolicies;
  if (!frontendPolicies.accessLog) frontendPolicies.accessLog = {};
  const accessLog = frontendPolicies.accessLog as {
    add?: Record<string, string>;
    database?: { add?: Record<string, string> };
  };
  accessLog.database ??= {};
  accessLog.database.add ??= {};
  if (enabled) {
    accessLog.database.add[promptLogKey] = promptLogExpression;
    accessLog.database.add[completionLogKey] = completionLogExpression;
    return;
  }
  delete accessLog.database.add[promptLogKey];
  delete accessLog.database.add[completionLogKey];
  if (Object.keys(accessLog.database.add).length === 0)
    delete accessLog.database.add;
  if (Object.keys(accessLog.database).length === 0) delete accessLog.database;
  if (accessLog.add) {
    delete accessLog.add[promptLogKey];
    delete accessLog.add[completionLogKey];
    if (Object.keys(accessLog.add).length === 0) delete accessLog.add;
  }
  if (Object.keys(accessLog).length === 0) delete frontendPolicies.accessLog;
  if (Object.keys(frontendPolicies).length === 0)
    delete config.frontendPolicies;
}

export function uiLogAttributeExpressions(config: GatewayConfig | undefined) {
  return {
    user: config?.config?.standardAttributes?.user ?? "",
    group: config?.config?.standardAttributes?.group ?? "",
  };
}

export function setUiLogAttributeExpressions(
  config: GatewayConfig,
  values: { user: string; group: string },
) {
  config.config ??= {};
  config.config.standardAttributes ??= {};
  const attributes = config.config.standardAttributes;
  const user = values.user.trim();
  const group = values.group.trim();
  if (user) attributes.user = user;
  else delete attributes.user;
  if (group) attributes.group = group;
  else delete attributes.group;
  if (Object.keys(attributes).length === 0)
    delete config.config.standardAttributes;
  if (Object.keys(config.config).length === 0) delete config.config;
}

export function modelWarnings(model: LlmModel): string[] {
  const warnings: string[] = [];
  if (!model.provider) warnings.push("Provider is required.");
  const provider = providerLabel(model.provider);
  if (provider === "reference") {
    if (!providerReferenceName(model.provider))
      warnings.push("Provider reference is required.");
    const extraParams = Object.keys(model.params ?? {}).filter(
      (key) => key !== "model",
    );
    if (extraParams.length > 0)
      warnings.push("Referenced models can only set the upstream model.");
    if (!model.name.trim()) warnings.push("Model name is required.");
    return warnings;
  }
  if (!model.name.trim()) warnings.push("Model name is required.");
  if (provider === "vertex" && !model.params?.vertexProject) {
    warnings.push("Vertex models should set a project.");
  }
  if (provider === "bedrock" && !model.params?.awsRegion) {
    warnings.push("Bedrock models should set an AWS region.");
  }
  if (provider === "azure" && !model.params?.azureResourceName) {
    warnings.push("Azure models should set a resource name.");
  }
  if (
    provider === "custom" &&
    typeof model.provider !== "string" &&
    "custom" in model.provider &&
    !model.provider.custom.formats.length
  ) {
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
  if (duplicateNames.length > 0)
    warnings.push(
      `Duplicate model names: ${Array.from(new Set(duplicateNames)).join(", ")}.`,
    );
  const apiPolicy = config.llm?.policies?.apiKey;
  if (apiPolicy?.mode && apiPolicy.mode !== "strict") {
    warnings.push(
      `Virtual API key mode is ${apiPolicy.mode}; unauthenticated requests may be accepted.`,
    );
  }
  for (const model of models) {
    for (const warning of modelWarnings(model)) {
      warnings.push(`${model.name || "Unnamed model"}: ${warning}`);
    }
  }
  const duplicateMcpTargets = mcpTargets
    .map((target) => target.name)
    .filter((name, index, names) => names.indexOf(name) !== index);
  if (duplicateMcpTargets.length > 0)
    warnings.push(
      `Duplicate MCP server names: ${Array.from(new Set(duplicateMcpTargets)).join(", ")}.`,
    );
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
      apiKey: null,
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
