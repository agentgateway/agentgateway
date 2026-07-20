import { tr } from "../i18n";
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
  apiKey: {
    get title() {
      return tr("copy.apiKeys");
    },
    icon: KeyRound,
  },
  authorization: {
    get title() {
      return tr("copy.authorization");
    },
    icon: ShieldCheck,
    customEditor: "authorization",
  },
  basicAuth: {
    get title() {
      return tr("copy.basicAuth");
    },
    icon: LockKeyhole,
  },
  cors: { title: "CORS", icon: Workflow, customEditor: "cors" },
  extAuthz: {
    get title() {
      return tr("copy.externalAuthz");
    },
    icon: CircuitBoard,
    customEditor: "extAuthz",
  },
  extProc: {
    get title() {
      return tr("copy.externalProcessor");
    },
    icon: SlidersHorizontal,
    customEditor: "extProc",
  },
  jwtAuth: {
    get title() {
      return tr("copy.jwtAuth");
    },
    icon: FileKey2,
    customEditor: "jwtAuth",
  },
  localRateLimit: {
    get title() {
      return tr("copy.localRateLimit");
    },
    icon: Timer,
    customEditor: "localRateLimit",
  },
  mcpAuthentication: {
    get title() {
      return tr("copy.mcpAuthentication");
    },
    icon: KeyRound,
    customEditor: "mcpAuthentication",
  },
  mcpAuthorization: {
    get title() {
      return tr("copy.mcpAuthorization");
    },
    icon: ShieldCheck,
    customEditor: "mcpAuthorization",
  },
  mcpGuardrails: {
    get title() {
      return tr("copy.mcpGuardrails");
    },
    icon: Shield,
    customEditor: "mcpGuardrails",
  },
  oidc: { title: "OIDC", icon: Fingerprint, customEditor: "oidc" },
  remoteRateLimit: {
    get title() {
      return tr("copy.remoteRateLimit");
    },
    icon: Braces,
    customEditor: "remoteRateLimit",
  },
  transformations: {
    get title() {
      return tr("copy.transformations");
    },
    icon: Shield,
    customEditor: "transformations",
  },
};
