import type {
  ExtAuthzPolicy,
  ExtProcPolicy,
  JwtPolicy as GeneratedJwtPolicy,
  LlmPolicy,
  LocalRateLimitPolicy,
  RemoteRateLimitPolicy,
  SimpleLocalRateLimitPolicy,
  OidcPolicy,
  TransformationPolicy,
  TransformPolicy,
  TrafficRoutePolicy,
} from "../types";
import type { LocalMcpAuthentication, McpGuardrails } from "../gateway-config";

export type PolicyKey = (keyof LlmPolicy | keyof TrafficRoutePolicy) & string;

export type JwtPolicy = GeneratedJwtPolicy;
export type LocalRateLimitConfig = LocalRateLimitPolicy;
export type LocalRateLimitDraft = SimpleLocalRateLimitPolicy;
export type RemoteRateLimitDraft = RemoteRateLimitPolicy;

export type AuthorizationDraft = {
  rules: Array<string | { allow?: string; deny?: string; require?: string }>;
};

export type TargetDraft = { host: string };

export type TransformationDraft = TransformationPolicy;
export type TransformDraft = TransformPolicy;
export type ExtAuthzDraft = ExtAuthzPolicy;
export type ExtProcDraft = ExtProcPolicy;
export type OidcDraft = OidcPolicy;
export type McpAuthenticationDraft = Partial<LocalMcpAuthentication>;
export type McpGuardrailsDraft = Partial<McpGuardrails>;

export type SchemaNode = {
  $ref?: string;
  anyOf?: SchemaNode[];
  oneOf?: SchemaNode[];
  type?: string | string[];
  const?: unknown;
  enum?: unknown[];
  default?: unknown;
  description?: string;
  properties?: Record<string, SchemaNode>;
  items?: SchemaNode;
};
