export interface UrlParams {
  port?: number;
  li?: number;
  isTcpRoute?: boolean;
  ri?: number;
  bi?: number;
  policyType?: string;
  topLevelType?: "llm" | "mcp" | "frontendPolicies";
  modelIndex?: number;
  llmPolicyType?: string;
  mcpPolicyType?: string;
  mcpTargetIndex?: number;
  mcpTargetPolicyType?: string;
}
