import {
  Shield,
  Lock,
  Globe,
  Edit,
  Timer,
  RotateCcw,
  Key,
  Zap,
  Network,
  FileText,
  ArrowRight,
  Users,
} from "lucide-react";

export type PolicyType =
  | "jwtAuth"
  | "mcpAuthentication"
  | "mcpAuthorization"
  | "cors"
  | "backendTLS"
  | "backendAuth"
  | "localRateLimit"
  | "remoteRateLimit"
  | "timeout"
  | "retry"
  | "requestHeaderModifier"
  | "responseHeaderModifier"
  | "requestRedirect"
  | "urlRewrite"
  | "directResponse"
  | "extAuthz"
  | "ai"
  | "a2a";

export interface PolicyTypeInfo {
  name: string;
  icon: React.ElementType;
  description: string;
  httpOnly?: boolean;
  tcpOnly?: boolean;
}

export const POLICY_TYPES: Record<PolicyType, PolicyTypeInfo> = {
  jwtAuth: {
    name: "JWT Authentication",
    icon: Shield,
    description: "Validate JWT tokens for authentication",
    httpOnly: true,
  },
  mcpAuthentication: {
    name: "MCP Authentication",
    icon: Key,
    description: "Model Context Protocol authentication",
    httpOnly: true,
  },
  mcpAuthorization: {
    name: "MCP Authorization",
    icon: Lock,
    description: "Model Context Protocol authorization rules",
    httpOnly: true,
  },
  cors: {
    name: "CORS",
    icon: Globe,
    description: "Cross-Origin Resource Sharing configuration",
    httpOnly: true,
  },
  backendTLS: {
    name: "Backend TLS",
    icon: Lock,
    description: "TLS configuration for backend connections",
  },
  backendAuth: {
    name: "Backend Auth",
    icon: Key,
    description: "Authentication for backend services",
  },
  localRateLimit: {
    name: "Local Rate Limit",
    icon: Timer,
    description: "Rate limiting at the gateway level",
    httpOnly: true,
  },
  remoteRateLimit: {
    name: "Remote Rate Limit",
    icon: Network,
    description: "Rate limiting using external service",
    httpOnly: true,
  },
  timeout: {
    name: "Timeout",
    icon: Timer,
    description: "Request and backend timeout configuration",
    httpOnly: true,
  },
  retry: {
    name: "Retry",
    icon: RotateCcw,
    description: "Retry configuration for failed requests",
    httpOnly: true,
  },
  requestHeaderModifier: {
    name: "Request Headers",
    icon: Edit,
    description: "Modify request headers",
    httpOnly: true,
  },
  responseHeaderModifier: {
    name: "Response Headers",
    icon: Edit,
    description: "Modify response headers",
    httpOnly: true,
  },
  requestRedirect: {
    name: "Request Redirect",
    icon: ArrowRight,
    description: "Redirect requests to different URLs",
    httpOnly: true,
  },
  urlRewrite: {
    name: "URL Rewrite",
    icon: Edit,
    description: "Rewrite request URLs",
    httpOnly: true,
  },
  directResponse: {
    name: "Direct Response",
    icon: FileText,
    description: "Return direct responses without backend",
    httpOnly: true,
  },
  extAuthz: {
    name: "External Authorization",
    icon: Shield,
    description: "External authorization service integration",
    httpOnly: true,
  },
  ai: {
    name: "AI Policy",
    icon: Zap,
    description: "AI/LLM policy configuration",
    httpOnly: true,
  },
  a2a: {
    name: "Agent-to-Agent",
    icon: Users,
    description: "Mark this traffic as A2A to enable A2A processing and telemetry",
    httpOnly: true,
  },
};

/**
 * Backend policy types (policies that affect backend routing and require exactly 1 backend)
 */
export const BACKEND_POLICY_KEYS: readonly PolicyType[] = [
  "mcpAuthentication",
  "mcpAuthorization",
  "backendTLS",
  "backendAuth",
  "ai",
  "a2a",
] as const;

/**
 * Check if a policy type is a backend policy
 */
export const isBackendPolicy = (policyType: PolicyType): boolean => {
  return BACKEND_POLICY_KEYS.includes(policyType);
};
