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
  LocalLLMConfig,
  LocalLLMModels,
  LocalLLMParams,
  LocalLLMPolicy,
  LocalLLMProvider,
  LocalLLMVirtualModel,
  PromptGuard,
  LocalRateLimitPolicy as GeneratedLocalRateLimitPolicy,
  RemoteRateLimit as GeneratedRemoteRateLimitPolicy,
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
  FilterOrPolicy,
  McpPrefixMode as GeneratedMcpPrefixMode,
  McpStatefulMode as GeneratedMcpStatefulMode,
  CustomProvider as GeneratedCustomProvider,
  ProviderFormat as GeneratedProviderFormat,
} from "./gateway-config";
import type { StoresDump } from "./gateway-admin";

export type ProviderName =
  | Extract<LocalLLMModels["provider"], string>
  | keyof Extract<LocalLLMModels["provider"], { custom: unknown }>;
export type ProviderFormat = GeneratedProviderFormat;
export type CustomProvider = GeneratedCustomProvider;
export type ModelProvider = LocalLLMModels["provider"];
export type ProviderAuth = BackendAuth;
export type SecretFromFile = Extract<FileOrInline, { file: string }>;
export type LlmParams = LocalLLMParams;
export type LlmModel = LocalLLMModels;
export type LlmVirtualModel = LocalLLMVirtualModel;
export type LlmProvider = LocalLLMProvider;
export type LlmGuardrail = PromptGuard;
export type VirtualApiKey = LocalAPIKey;
export type LlmApiKeyPolicy = LocalAPIKeys;
export type LlmPolicy = LocalLLMPolicy;
export type LlmConfig = LocalLLMConfig;
export type CorsPolicy = CorsSerde;
export type JwtPolicy = Partial<Extract<LocalJwtConfig, { issuer: string }>>;
export type LocalRateLimitPolicy = GeneratedLocalRateLimitPolicy;
export type RemoteRateLimitPolicy = GeneratedRemoteRateLimitPolicy;
export type SimpleLocalRateLimitPolicy = Extract<
  GeneratedLocalRateLimitPolicy,
  unknown[]
>;
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
export type TrafficRoutePolicy = FilterOrPolicy;

export type McpTargetKind = keyof Pick<
  LocalMcpTarget,
  "sse" | "mcp" | "stdio" | "openapi"
>;
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
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & {
      sse: McpNetworkTarget;
    })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & {
      mcp: McpNetworkTarget;
    })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & {
      stdio: McpStdioTarget;
    })
  | ({ name: string; policies?: LocalMcpTarget["policies"] } & {
      openapi: McpNetworkTarget & { schema: unknown };
    });
export type McpConfig = Omit<LocalSimpleMcpConfig, "targets"> & {
  targets: McpTarget[];
};
export type GatewayConfig = Omit<LocalConfig, "llm" | "mcp"> & {
  llm?: LlmConfig | null;
  mcp?: McpConfig | null;
};
export type AdminConfigDump = StoresDump;

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
  cost?: number | null;
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

export interface AnalyticsSummaryRequest {
  timeRange?: TimeRange;
  filters?: LogFilters;
  groupBy?: Array<{
    field:
      | "provider"
      | "requestModel"
      | "responseModel"
      | "httpStatus"
      | "attributes";
    key?: string | null;
  }>;
  bucketCount?: number;
  bucketSeconds?: number;
}

export interface SearchLogsResponse {
  logs: LogEntry[];
  nextCursor?: string | null;
}

export interface TailEvent {
  entry: LogEntry;
  cursor: string;
}

export interface AnalyticsGroup {
  group: Record<string, unknown>;
  requests: number;
  totalTokens: number;
  cost?: number | null;
}

export interface AnalyticsTimeBucket {
  start: string;
  group: Record<string, unknown>;
  requests: number;
  totalTokens: number;
  cost?: number | null;
}

export interface AnalyticsSummaryResponse {
  timeRange: TimeRange;
  bucketSeconds: number;
  buckets: AnalyticsTimeBucket[];
  groups: AnalyticsGroup[];
  filterOptions?: Record<string, string[]>;
}
