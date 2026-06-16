// Types for the imported pages. These are defined standalone rather than derived
// from our config.d so they stay compatible with John's codebase.
// The GatewayConfig type is intentionally a superset of our LocalConfig.

import type {
  CorsSerde,
  ExtAuthz,
  ExtProc,
  FileOrInline,
  LocalAPIKey,
  LocalAPIKeys,
  LocalConfig,
  BackendAuth,
  LocalJwtConfig,
  LocalLLMPolicy,
  PromptGuard,
  LocalRateLimitPolicy as GeneratedLocalRateLimitPolicy,
  LocalMcpTarget,
  LocalOidcConfig,
  LocalTransform,
  LocalTransformationConfig,
  LocalSimpleMcpConfig,
  LocalBind,
  LocalListener,
  LocalRoute,
  LocalRouteBackend,
  LocalTCPRoute,
  LocalTCPRouteBackend,
  LocalGatewayPolicy,
  FilterOrPolicy,
  TCPFilterOrPolicy,
  McpPrefixMode as GeneratedMcpPrefixMode,
  McpStatefulMode as GeneratedMcpStatefulMode,
  Provider8,
  ProviderFormat as GeneratedProviderFormat,
} from "../config.d";

// Provider names — superset of what our config.d knows about
export type ProviderName =
  | "openai" | "openAI"
  | "anthropic"
  | "gemini"
  | "vertex"
  | "bedrock"
  | "azure"
  | "copilot"
  | "cohere"
  | "ollama"
  | "baseten"
  | "cerebras"
  | "deepinfra"
  | "deepseek"
  | "groq"
  | "huggingface"
  | "mistral"
  | "openrouter"
  | "togetherai"
  | "xAI"
  | "fireworks"
  | "custom";

export type ProviderFormat = GeneratedProviderFormat;
export type CustomProvider = Provider8;

export type ModelProvider =
  | ProviderName
  | { reference: string }
  | { custom: Provider8 };

export type ProviderAuth = BackendAuth;
export type SecretFromFile = Extract<FileOrInline, { file: string }>;

export interface LlmParams {
  model?: string | null;
  apiKey?: string | SecretFromFile | null;
  azureResourceName?: string | null;
  azureResourceType?: "openAI" | "foundry" | null;
  azureApiVersion?: string | null;
  azureProjectName?: string | null;
  vertexProject?: string | null;
  vertexRegion?: string | null;
  awsRegion?: string | null;
  baseUrl?: string | null;
  [k: string]: unknown;
}

export interface LlmModelEviction {
  duration?: string | null;
  consecutiveFailures?: number | null;
  healthThreshold?: number | null;
  restoreHealth?: number | null;
  [k: string]: unknown;
}

export interface LlmModelHealth {
  unhealthyExpression?: string | null;
  eviction?: LlmModelEviction | null;
  [k: string]: unknown;
}

export interface LlmModelHeaderModifier {
  add?: Record<string, string>;
  set?: Record<string, string>;
  remove?: string[];
  [k: string]: unknown;
}

export interface LlmModelPromptCaching {
  cacheSystem?: boolean | null;
  cacheMessages?: boolean | null;
  cacheTools?: boolean | null;
  minTokens?: number | null;
  cacheMessageOffset?: number | null;
  [k: string]: unknown;
}

export interface LlmModelHeaderMatch {
  name: string;
  value?: { exact: string } | { regex: string } | null;
  [k: string]: unknown;
}

export interface LlmModelMatch {
  path?: Record<string, unknown>;
  headers?: LlmModelHeaderMatch[];
  [k: string]: unknown;
}

export interface LlmModel {
  name: string;
  provider: ModelProvider | null;
  params?: LlmParams | null;
  auth?: ProviderAuth | null;
  matches?: LlmModelMatch[] | null;
  health?: LlmModelHealth | null;
  requestHeaders?: LlmModelHeaderModifier | null;
  responseHeaders?: LlmModelHeaderModifier | null;
  promptCaching?: LlmModelPromptCaching | null;
  transformation?: Record<string, string> | null;
  defaults?: Record<string, unknown> | null;
  overrides?: Record<string, unknown> | null;
  [k: string]: unknown;
}

export interface LlmVirtualModelTarget {
  model: string;
  weight?: number;
  priority?: number;
  condition?: string;
  when?: string;
  [k: string]: unknown;
}

export interface LlmVirtualModel {
  name: string;
  routing: {
    weighted?: { targets: LlmVirtualModelTarget[] };
    failover?: { targets: LlmVirtualModelTarget[] };
    conditional?: { targets: LlmVirtualModelTarget[] };
    [k: string]: unknown;
  };
  [k: string]: unknown;
}

export interface LlmProviderDefaults {
  auth?: BackendAuth | null;
  [k: string]: unknown;
}

export interface LlmProvider {
  name: string;
  provider: ModelProvider;
  params?: LlmParams | null;
  defaults?: LlmProviderDefaults | null;
  [k: string]: unknown;
}

export type LlmGuardrail = PromptGuard;
export type VirtualApiKey = LocalAPIKey;
export type LlmApiKeyPolicy = LocalAPIKeys;
export type LlmPolicy = LocalLLMPolicy & { [k: string]: unknown };

export interface LlmConfig {
  models: LlmModel[];
  providers?: LlmProvider[] | null;
  virtualModels?: LlmVirtualModel[] | null;
  policies?: LlmPolicy | null;
  port?: number | null;
}

export type CorsPolicy = CorsSerde;
export type JwtPolicy = Partial<Extract<LocalJwtConfig, { issuer: string }>>;
export type LocalRateLimitPolicy = GeneratedLocalRateLimitPolicy;
export type SimpleLocalRateLimitPolicy = Extract<GeneratedLocalRateLimitPolicy, unknown[]>;
export type TransformationPolicy = LocalTransformationConfig;
export type TransformPolicy = LocalTransform;
export type ExtAuthzPolicy = ExtAuthz;
export type ExtProcPolicy = ExtProc;
export type OidcPolicy = Partial<LocalOidcConfig>;
export type TrafficBind = LocalBind;
export type TrafficListener = LocalListener;
export type TrafficRoute = LocalRoute;
export type TrafficRouteBackend = LocalRouteBackend;
export type TrafficTcpRoute = LocalTCPRoute;
export type TrafficTcpRouteBackend = LocalTCPRouteBackend;
export type TrafficListenerPolicy = LocalGatewayPolicy;
export type TrafficRoutePolicy = FilterOrPolicy;
export type TrafficTcpRoutePolicy = TCPFilterOrPolicy;

export type McpTargetKind = keyof Pick<LocalMcpTarget, "sse" | "mcp" | "stdio" | "openapi">;
export type McpStatefulMode = GeneratedMcpStatefulMode;
export type McpPrefixMode = GeneratedMcpPrefixMode;
export type McpFailureMode = NonNullable<LocalSimpleMcpConfig["failureMode"]>;
export interface McpNetworkTarget {
  host?: string | null;
  port?: number | null;
  path?: string | null;
  backend?: string | null;
}
export interface McpStdioTarget {
  cmd: string;
  args?: string[];
  env?: Record<string, string>;
  clear_env?: boolean;
}
export type McpTarget =
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & { sse: McpNetworkTarget })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & { mcp: McpNetworkTarget })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & { stdio: McpStdioTarget })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & { openapi: McpNetworkTarget & { schema: unknown } });
export type McpConfig = Omit<LocalSimpleMcpConfig, "targets"> & { targets: McpTarget[] };
export type GatewayConfig = Omit<LocalConfig, "llm" | "mcp"> & {
  llm?: LlmConfig | null;
  mcp?: McpConfig | null;
};

export interface LogEntry {
  id: string;
  startedAt: string;
  completedAt: string;
  durationMs: number;
  traceId?: string | null;
  spanId?: string | null;
  httpStatus?: number | null;
  error?: string | null;
  genAi: {
    operationName?: string | null;
    providerName?: string | null;
    requestModel?: string | null;
    responseModel?: string | null;
  };
  usage: {
    inputTokens?: number | null;
    outputTokens?: number | null;
    totalTokens?: number | null;
  };
  hasPayload: boolean;
  attributes?: unknown;
  payload?: {
    requestPrompt?: unknown;
    responseCompletion?: unknown;
  };
}

export interface LogFilters {
  httpStatus?: number[];
  provider?: string[];
  requestModel?: string[];
  responseModel?: string[];
  traceId?: string | null;
  hasPayload?: boolean | null;
  attributes?: Record<string, unknown>;
}

export interface TimeRange {
  from?: string | null;
  to?: string | null;
}

export interface SearchLogsRequest {
  limit?: number;
  cursor?: string;
  timeRange?: TimeRange;
  filters?: LogFilters;
  includeAttributes?: boolean;
}

export interface TokenUsageRequest {
  timeRange?: TimeRange;
  filters?: LogFilters;
  groupBy?: Array<{
    field: "provider" | "requestModel" | "responseModel" | "httpStatus" | "attributes";
    key?: string | null;
  }>;
}

export interface SearchLogsResponse {
  logs: LogEntry[];
  nextCursor?: string | null;
}

export interface TailEvent {
  entry: LogEntry;
  cursor: string;
}

export interface TokenUsageGroup {
  group: Record<string, unknown>;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}
