import {
  Braces,
  CircuitBoard,
  FileKey2,
  Fingerprint,
  KeyRound,
  LockKeyhole,
  Shield,
  ShieldCheck,
  SlidersHorizontal,
  Timer,
  Workflow,
} from "lucide-react";
import type { ComponentType } from "react";
import type { PolicyKey } from "./types";

export const policyUi: Partial<
  Record<
    PolicyKey,
    {
      title: string;
      icon: ComponentType<{ size?: number }>;
      customEditor?:
        | "authorization"
        | "cors"
        | "extAuthz"
        | "extProc"
        | "jwtAuth"
        | "localRateLimit"
        | "mcpAuthentication"
        | "mcpAuthorization"
        | "mcpGuardrails"
        | "oidc"
        | "remoteRateLimit"
        | "transformations";
    }
  >
> = {
  apiKey: { title: "API keys", icon: KeyRound },
  authorization: {
    title: "Authorization",
    icon: ShieldCheck,
    customEditor: "authorization",
  },
  basicAuth: { title: "Basic auth", icon: LockKeyhole },
  cors: { title: "CORS", icon: Workflow, customEditor: "cors" },
  extAuthz: {
    title: "External authz",
    icon: CircuitBoard,
    customEditor: "extAuthz",
  },
  extProc: {
    title: "External processor",
    icon: SlidersHorizontal,
    customEditor: "extProc",
  },
  jwtAuth: { title: "JWT auth", icon: FileKey2, customEditor: "jwtAuth" },
  localRateLimit: {
    title: "Local rate limit",
    icon: Timer,
    customEditor: "localRateLimit",
  },
  mcpAuthentication: {
    title: "MCP authentication",
    icon: KeyRound,
    customEditor: "mcpAuthentication",
  },
  mcpAuthorization: {
    title: "MCP authorization",
    icon: ShieldCheck,
    customEditor: "mcpAuthorization",
  },
  mcpGuardrails: {
    title: "MCP guardrails",
    icon: Shield,
    customEditor: "mcpGuardrails",
  },
  oidc: { title: "OIDC", icon: Fingerprint, customEditor: "oidc" },
  remoteRateLimit: {
    title: "Remote rate limit",
    icon: Braces,
    customEditor: "remoteRateLimit",
  },
  transformations: {
    title: "Transformations",
    icon: Shield,
    customEditor: "transformations",
  },
};
