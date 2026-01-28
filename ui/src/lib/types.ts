import { z } from "zod";

// Main configuration structure
export interface LocalConfig {
  binds: Bind[];
  workloads?: any[];
  services?: any[];
  appliedPolicies?: AppliedPolicy[];
}

export interface Bind {
  port: number; // uint16, 0-65535
  listeners: Listener[];
}

export interface Listener {
  name?: string | null;
  hostname?: string | null; // Can be a wildcard
  protocol: ListenerProtocol;
  tls?: TlsConfig | null;
  routes?: Route[] | null;
  tcpRoutes?: TcpRoute[] | null;
}

export enum ListenerProtocol {
  HTTP = "HTTP",
  HTTPS = "HTTPS",
  TLS = "TLS",
  TCP = "TCP",
  HBONE = "HBONE",
}

export interface TlsConfig {
  cert: string;
  key: string;
  // Optional mTLS client verification roots (server-side).
  root?: string;
  // Optional overrides
  cipherSuites?: string[];
  minTLSVersion?: TLSVersion;
  maxTLSVersion?: TLSVersion;
}

export enum TLSVersion {
  // rustls only supports TLS 1.2 and 1.3
  TLS_V1_2 = "TLS_V1_2",
  TLS_V1_3 = "TLS_V1_3",
}

export interface Route {
  name?: string | null;
  ruleName?: string | null;
  hostnames: string[]; // Can be wildcards
  matches: Match[];
  policies?: Policies | null;
  backends: Backend[];
}

export interface TcpRoute {
  name?: string | null;
  ruleName?: string | null;
  hostnames: string[]; // Can be wildcards
  policies?: TcpPolicies | null;
  backends: TcpBackend[];
}

export interface Match {
  headers?: HeaderMatch[];
  path: PathMatch;
  method?: MethodMatch | null;
  query?: QueryMatch[];
}

export interface HeaderMatch {
  name: string;
  value: ExactOrRegexMatch;
}

export interface PathMatch {
  exact?: string;
  pathPrefix?: string;
  regex?: [string, number]; // [pattern, flags]
}

export interface MethodMatch {
  method: string;
}

export interface QueryMatch {
  name: string;
  value: ExactOrRegexMatch;
}

export interface ExactOrRegexMatch {
  exact?: string;
  regex?: string;
}

export interface Policies {
  requestHeaderModifier?: HeaderModifier | null;
  responseHeaderModifier?: HeaderModifier | null;
  requestRedirect?: RequestRedirect | null;
  urlRewrite?: UrlRewrite | null;
  requestMirror?: RequestMirror | null;
  directResponse?: DirectResponse | null;
  cors?: CorsPolicy | null;
  mcpAuthorization?: McpAuthorization | null;
  mcpAuthentication?: McpAuthentication | null;
  a2a?: any | null;
  ai?: any;
  backendTLS?: BackendTLS | null;
  backendAuth?: BackendAuth | null;
  localRateLimit?: any;
  remoteRateLimit?: any;
  jwtAuth?: JwtAuth;
  extAuthz?: any;
  timeout?: TimeoutPolicy | null;
  retry?: RetryPolicy | null;
}

// Top-level applied policy entries from config dump
export interface AppliedPolicy {
  key: string;
  name: {
    kind: string;
    name: string;
    namespace: string;
  };
  target: {
    backend?: {
      backend?: { name: string; namespace: string };
      service?: { hostname: string; namespace: string };
    };
    route?: { name: string; namespace: string };
  };
  // Keep flexible to match varying shapes in dump; narrow types over time
  policy: {
    backend?: Partial<Policies>;
    traffic?: Record<string, unknown>;
  };
}

export interface TcpPolicies {
  backendTls?: BackendTLS | null;
}

export interface HeaderModifier {
  add?: [string, string][];
  set?: [string, string][];
  remove?: string[];
}

export interface RequestRedirect {
  scheme?: string | null;
  authority?: AuthorityRewrite | null;
  path?: PathRewrite | null;
  status?: number | null; // uint16, 1-65535
}

export interface UrlRewrite {
  authority?: AuthorityRewrite | null;
  path?: PathRewrite | null;
}

export interface AuthorityRewrite {
  full?: string;
  host?: string;
  port?: number; // uint16, 1-65535
}

export interface PathRewrite {
  full?: string;
  prefix?: string;
}

export interface RequestMirror {
  backend: BackendRef;
  percentage: number;
}

export interface DirectResponse {
  body: number[] | string; // uint8[] or string
  status: number; // uint16, 1-65535
}

export interface CorsPolicy {
  allowCredentials?: boolean;
  allowHeaders?: string[];
  allowMethods?: string[];
  allowOrigins?: string[];
  exposeHeaders?: string[];
  maxAge?: string | null;
}

export interface McpAuthorizationRule {
  allow?: string;
  deny?: string;
}

export interface McpAuthorization {
  rules: (string | McpAuthorizationRule)[];
}

export interface McpAuthentication {
  issuer: string;
  scopes: string[];
  provider: Auth0Provider;
}

export interface Auth0Provider {
  auth0: {
    audience?: string | null;
  };
}

export interface BackendTLS {
  cert?: string | null;
  key?: string | null;
  root?: string | null;
  insecure?: boolean;
  insecureHost?: boolean;
}

export interface BackendAuth {
  passthrough?: any;
  key?: string | { file: string };
  gcp?: any;
  aws?: any;
}

export interface JwtAuth {
  issuer: string;
  audiences: string[];
  jwks: { url: string } | { file: string };
}

export interface TimeoutPolicy {
  requestTimeout?: string | null;
  backendRequestTimeout?: string | null;
}

export interface RetryPolicy {
  attempts?: number; // uint8, 1-255, default: 1
  backoff?: string | null;
  codes: number[]; // uint8[], 1-255
}

export interface Backend {
  weight?: number; // uint, default: 1
  filters?: Filter[];
  // Backend reference - one of these will be set
  service?: ServiceBackend;
  host?: HostBackend;
  dynamic?: DynamicBackend;
  mcp?: McpBackend;
  ai?: AiBackend;
}

export interface TcpBackend {
  weight?: number; // uint, default: 1
  backend: TcpBackendRef;
}

export interface TcpBackendRef {
  service?: ServiceBackend;
  host?: HostBackend;
}

export interface Filter {
  requestHeaderModifier?: HeaderModifier;
  responseHeaderModifier?: HeaderModifier;
  requestRedirect?: RequestRedirect;
  urlRewrite?: UrlRewrite;
  requestMirror?: RequestMirror;
  directResponse?: DirectResponse;
  cors?: CorsPolicy;
}

export interface ServiceBackend {
  name: {
    namespace: string;
    hostname: string;
  };
  port: number; // uint16, 0-65535
}

export interface HostBackend {
  Address?: string;
  Hostname?: [string, number]; // [hostname, port]
  name?: string;
}

export interface DynamicBackend {
  // Empty object
}

// UI-friendly flat structure (matches local config format for write path)
export interface McpBackend {
  // Display-only name from config_dump (namespace/name); absent in local config.
  name?: string;
  targets: McpTarget[];
  statefulMode?: McpStatefulMode;
}

// UI-friendly flat structure (matches local config format for write path)
export interface AiBackend {
  name: string;
  provider: AiProvider;
  hostOverride?: string | null;
  pathOverride?: string | null;
}

export interface AiProvider {
  openAI?: { model?: string | null };
  gemini?: { model?: string | null };
  vertex?: { model?: string | null; region?: string | null; projectId?: string };
  anthropic?: { model?: string | null };
  bedrock?: { model?: string | null; region?: string | null };
  azureOpenAI?: { model?: string | null; host?: string | null; apiVersion?: string | null };
}

export enum McpStatefulMode {
  STATELESS = "stateless",
  STATEFUL = "stateful",
}

export interface McpTarget {
  name: string;
  filters?: TargetFilter[];
  // Target type - one of these will be set
  sse?: StreamHttpTarget;
  mcp?: StreamHttpTarget;
  stdio?: StdioTarget;
  openapi?: OpenApiTarget;
}

export interface TargetFilter {
  matcher: TargetMatcher;
  resource_type: string;
}

export interface TargetMatcher {
  Equals?: string;
  Prefix?: string;
  Suffix?: string;
  Contains?: string;
  Regex?: string;
}

// UI-friendly format for SSE/MCP targets (matches config file format)
export interface StreamHttpTarget {
  host: string;
  port?: number;
  path?: string;
}

export interface StdioTarget {
  cmd: string;
  args?: string[];
  env?: { [key: string]: string };
}

// UI-friendly format for OpenAPI targets (matches config file format)
export interface OpenApiTarget {
  host: string;
  port?: number;
  schema: any; // OpenAPI schema
}

export interface BackendRef {
  service?: ServiceBackend;
  host?: HostBackend;
}

// Legacy types for backward compatibility
export interface Target {
  name: string;
  listeners?: string[];
  filters?: TargetFilter[];
  sse?: StreamHttpTarget;
  mcp?: StreamHttpTarget;
  openapi?: OpenApiTarget;
  stdio?: StdioTarget;
  a2a?: A2aTarget;
}

export interface A2aTarget {
  host: string;
  port: number;
  path: string;
  headers?: Header[];
  auth?: BackendAuth;
  tls?: BackendTLS;
}

export interface Header {
  key: string;
  value: {
    string_value?: string;
    env_value?: string;
  };
}

export type TargetType = "mcp" | "sse" | "openapi" | "stdio" | "a2a" | "unknown";

export interface LocalDataSource {
  file_path?: string;
  inline?: Uint8Array;
}

export interface RemoteDataSource {
  url: string;
}

export interface JwtConfig {
  issuer: string[];
  audience: string[];
  localJwks?: LocalDataSource;
  remoteJwks?: RemoteDataSource;
}

export interface Authn {
  jwt: JwtConfig;
}

export interface SseListener {
  address?: string;
  host?: string;
  port: number;
  tls?: TlsConfig;
  authn?: Authn;
  rbac?: RuleSet[];
}

export interface StdioListener {
  // Empty interface
}

export type Matcher = "EQUALS";
export type ResourceType = "TOOL";

export interface Rule {
  key: string;
  value: string;
  resource: {
    type: ResourceType;
    target: string;
    id: string;
  };
  matcher: Matcher;
}

export interface RBACConfig {
  name: string;
  namespace: string;
  rules: Rule[];
}

export interface RuleSet {
  name: string;
  namespace: string;
  rules: Rule[];
}

type ConfigType = "static";

export interface Config {
  type: ConfigType;
  listeners: Listener[];
  policies?: RBACConfig[];
  targets: TargetWithType[];
}

export type TargetWithType = Target & { type: "mcp" | "a2a" | "openapi" | "stdio" | "sse" };

// Schema specifically for Playground UI representation
export const PlaygroundListenerSchema = z.object({
  name: z.string(),
  protocol: z.nativeEnum(ListenerProtocol).optional(),
  sse: z
    .object({
      port: z.number(),
      host: z.string(),
      tls: z.boolean().optional(),
    })
    .optional(),
});

export type PlaygroundListener = z.infer<typeof PlaygroundListenerSchema>;

export interface ListenerInfo extends Listener {
  displayEndpoint: string;
}
