export type PolicyScope = "llm" | "mcp" | "route" | "mcpTarget";

export interface PolicyTypeInfo {
  key: string;
  label: string;
  description: string;
  scopes: PolicyScope[];
}

/**
 * Centralized registry of all policy types.
 *
 * `scopes` controls where the policy appears in "Add Policy" menus:
 * - "llm"       → LLM config policies (LocalLLMPolicy)
 * - "mcp"       → MCP config policies (FilterOrPolicy)
 * - "route"     → Route-level policies (FilterOrPolicy)
 * - "mcpTarget" → MCP target-level policies (MCPLocalBackendPolicies)
 *
 * To add a new policy type, add one entry here. All menus, labels, and
 * tree renderers read from this list.
 */
export const POLICY_TYPES: PolicyTypeInfo[] = [
  // --- Authentication ---
  { key: "jwtAuth", label: "JWT Auth", description: "Authenticate incoming JWT requests", scopes: ["llm", "mcp", "route"] },
  { key: "basicAuth", label: "Basic Auth", description: "Authenticate with HTTP Basic Authentication", scopes: ["llm", "mcp", "route"] },
  { key: "apiKey", label: "API Key Auth", description: "Authenticate with API keys", scopes: ["llm", "mcp", "route"] },
  { key: "mcpAuthentication", label: "MCP Authentication", description: "Authentication for MCP clients", scopes: ["mcp"] },

  // --- Authorization ---
  { key: "authorization", label: "Authorization", description: "Authorization policies for HTTP access", scopes: ["llm", "mcp", "route"] },
  { key: "mcpAuthorization", label: "MCP Authorization", description: "Authorization policies for MCP access", scopes: ["mcp", "mcpTarget"] },

  // --- External Services ---
  { key: "extAuthz", label: "External Auth", description: "Authenticate via external authorization server", scopes: ["llm", "mcp", "route"] },
  { key: "extProc", label: "External Processor", description: "Extend with an external processor", scopes: ["llm", "mcp", "route"] },

  // --- Request/Response Modification ---
  { key: "transformations", label: "Transformations", description: "Modify requests and responses", scopes: ["llm", "mcp", "route", "mcpTarget"] },
  { key: "requestHeaderModifier", label: "Request Headers", description: "Modify headers in requests", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "responseHeaderModifier", label: "Response Headers", description: "Modify headers in responses", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "cors", label: "CORS", description: "Handle CORS preflight requests", scopes: ["mcp", "route"] },
  { key: "urlRewrite", label: "URL Rewrite", description: "Modify the URL path or authority", scopes: ["mcp", "route"] },
  { key: "requestRedirect", label: "Request Redirect", description: "Respond with a redirect", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "requestMirror", label: "Request Mirror", description: "Mirror incoming requests to another destination", scopes: ["mcp", "route"] },
  { key: "directResponse", label: "Direct Response", description: "Respond with a static response", scopes: ["mcp", "route"] },

  // --- Rate Limiting ---
  { key: "localRateLimit", label: "Local Rate Limit", description: "Rate limit with local state", scopes: ["mcp", "route"] },
  { key: "remoteRateLimit", label: "Remote Rate Limit", description: "Rate limit with remote state server", scopes: ["mcp", "route"] },

  // --- Backend ---
  { key: "backendTLS", label: "Backend TLS", description: "Send TLS to the backend", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "backendTunnel", label: "Backend Tunnel", description: "Tunnel to the backend", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "backendAuth", label: "Backend Auth", description: "Authenticate to the backend", scopes: ["mcp", "route", "mcpTarget"] },
  { key: "health", label: "Health Policy", description: "Backend outlier detection and health checks", scopes: ["mcpTarget"] },

  // --- Other ---
  { key: "a2a", label: "A2A", description: "Enable A2A processing and telemetry", scopes: ["mcp", "route"] },
  { key: "ai", label: "AI", description: "Enable LLM processing", scopes: ["mcp", "route"] },
  { key: "csrf", label: "CSRF", description: "CSRF protection via origin validation", scopes: ["mcp", "route"] },
  { key: "timeout", label: "Timeout", description: "Timeout requests exceeding configured duration", scopes: ["mcp", "route"] },
  { key: "retry", label: "Retry", description: "Retry matching requests", scopes: ["mcp", "route"] },
];

const typeMap = new Map(POLICY_TYPES.map((t) => [t.key, t]));

export function getPolicyTypesForScope(scope: PolicyScope): PolicyTypeInfo[] {
  return POLICY_TYPES.filter((t) => t.scopes.includes(scope));
}

export function getPolicyLabel(key: string): string {
  return typeMap.get(key)?.label ?? key.replace(/([A-Z])/g, " $1").replace(/^./, (s) => s.toUpperCase()).trim();
}

export function getPolicyDescription(key: string): string {
  return typeMap.get(key)?.description ?? "";
}

/**
 * Get default value for a policy type.
 * Provides sensible defaults with required fields populated.
 */
export function getDefaultPolicyValue(policyType: string): Record<string, unknown> {
  switch (policyType) {
    // Authorization policies require rules array
    case "authorization":
    case "mcpAuthorization":
      return { rules: [] };
    
    // Authentication policies have complex required fields - return minimal structure
    // JWT Auth: LocalJwtConfig (single-provider shorthand)
    // jwks is FileInlineOrRemote = { file: string } | string | { url: string }
    case "jwtAuth":
      return {
        issuer: "",
        audiences: [],
        jwks: '{"keys":[]}',
      };

    case "basicAuth":
      return {
        htpasswd: "",
      };

    case "apiKey":
      return {
        keys: [],
      };

    case "mcpAuthentication":
      return {
        issuer: "",
        audiences: [],
        resourceMetadata: {},
        jwks: '{"keys":[]}',
      };

    // ExtAuthz = { policies?, protocol?, failureMode?, ... } & ExtAuthz1
    // ExtAuthz1 = "invalid" | { service: { name, port } } | { host: string } | { backend: string }
    case "extAuthz":
      return {
        host: "",
      };

    // ExtProc = { policies?, failureMode?, ... } & ExtProc1
    // ExtProc1 = "invalid" | { service: { name, port } } | { host: string } | { backend: string }
    case "extProc":
      return {
        host: "",
      };

    // Rate limiting
    case "localRateLimit":
      return {
        spec: {
          fillInterval: "1s",
          tokensPerFill: 100,
        },
      };

    case "remoteRateLimit":
      return {
        host: "",
        spec: {
          fillInterval: "1s",
          tokensPerFill: 100,
        },
      };

    // Backend TLS
    case "backendTLS":
      return {
        hostname: "",
      };

    // Backend tunnel
    case "backendTunnel":
      return {
        proxy: {
          host: "",
        },
      };

    // BackendAuth = { passthrough: {} } | { key: ... } | { gcp: ... } | { aws: ... } | { azure: ... }
    case "backendAuth":
      return {
        passthrough: {},
      };
    
    // Most other policies can start empty or with minimal structure
    default:
      return {};
  }
}
