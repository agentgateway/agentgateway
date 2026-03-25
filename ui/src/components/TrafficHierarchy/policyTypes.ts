export type PolicyScope = "llm" | "mcp" | "route";

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
 * - "llm"   → LLM config policies (LocalLLMPolicy)
 * - "mcp"   → MCP config policies (FilterOrPolicy)
 * - "route" → Route-level policies (FilterOrPolicy)
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
  { key: "mcpAuthorization", label: "MCP Authorization", description: "Authorization policies for MCP access", scopes: ["mcp"] },

  // --- External Services ---
  { key: "extAuthz", label: "External Auth", description: "Authenticate via external authorization server", scopes: ["llm", "mcp", "route"] },
  { key: "extProc", label: "External Processor", description: "Extend with an external processor", scopes: ["llm", "mcp", "route"] },

  // --- Request/Response Modification ---
  { key: "transformations", label: "Transformations", description: "Modify requests and responses", scopes: ["llm", "mcp", "route"] },
  { key: "requestHeaderModifier", label: "Request Headers", description: "Modify headers in requests", scopes: ["mcp", "route"] },
  { key: "responseHeaderModifier", label: "Response Headers", description: "Modify headers in responses", scopes: ["mcp", "route"] },
  { key: "cors", label: "CORS", description: "Handle CORS preflight requests", scopes: ["mcp", "route"] },
  { key: "urlRewrite", label: "URL Rewrite", description: "Modify the URL path or authority", scopes: ["mcp", "route"] },
  { key: "requestRedirect", label: "Request Redirect", description: "Respond with a redirect", scopes: ["mcp", "route"] },
  { key: "requestMirror", label: "Request Mirror", description: "Mirror incoming requests to another destination", scopes: ["mcp", "route"] },
  { key: "directResponse", label: "Direct Response", description: "Respond with a static response", scopes: ["mcp", "route"] },

  // --- Rate Limiting ---
  { key: "localRateLimit", label: "Local Rate Limit", description: "Rate limit with local state", scopes: ["mcp", "route"] },
  { key: "remoteRateLimit", label: "Remote Rate Limit", description: "Rate limit with remote state server", scopes: ["mcp", "route"] },

  // --- Backend ---
  { key: "backendTLS", label: "Backend TLS", description: "Send TLS to the backend", scopes: ["mcp", "route"] },
  { key: "backendTunnel", label: "Backend Tunnel", description: "Tunnel to the backend", scopes: ["mcp", "route"] },
  { key: "backendAuth", label: "Backend Auth", description: "Authenticate to the backend", scopes: ["mcp", "route"] },

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
