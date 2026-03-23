/**
 * Traffic Forms Index
 *
 * Central export point for all manually configured form definitions.
 * Unlike Traffic2, these schemas are NOT generated from JSON files,
 * but are handcrafted TypeScript schemas that use the types from config.d.ts.
 */

import * as backendForm from "./backendForm";
import * as bindForm from "./bindForm";
import * as corsPolicyForm from "./corsPolicyForm";
import * as frontendPoliciesForm from "./frontendPoliciesForm";
import * as listenerForm from "./listenerForm";
import * as llmForm from "./llmForm";
import * as mcpForm from "./mcpForm";
import * as modelForm from "./modelForm";
import * as policyForm from "./policyForm";
import * as requestHeaderModifierPolicyForm from "./requestHeaderModifierPolicyForm";
import * as responseHeaderModifierPolicyForm from "./responseHeaderModifierPolicyForm";
import * as routeForm from "./routeForm";
import * as routePolicyForm from "./routePolicyForm";
import * as topLevelBackendForm from "./topLevelBackendForm";

export const forms = {
  bind: bindForm,
  listener: listenerForm,
  route: routeForm,
  backend: backendForm,
  policy: policyForm,
  routePolicy: routePolicyForm,
  corsPolicy: corsPolicyForm,
  requestHeaderModifierPolicy: requestHeaderModifierPolicyForm,
  responseHeaderModifierPolicy: responseHeaderModifierPolicyForm,
  topLevelBackend: topLevelBackendForm,
  llm: llmForm,
  model: modelForm,
  mcp: mcpForm,
  frontendPolicies: frontendPoliciesForm,
};

export type ResourceType = keyof typeof forms;

export const resourceTypes: ResourceType[] = [
  "bind",
  "listener",
  "route",
  "backend",
  "policy",
  "topLevelBackend",
  "llm",
  "mcp",
  "frontendPolicies",
];

export const resourceLabels: Record<
  ResourceType,
  { singular: string; plural: string }
> = {
  bind: { singular: "Bind", plural: "Binds" },
  listener: { singular: "Listener", plural: "Listeners" },
  route: { singular: "Route", plural: "Routes" },
  backend: { singular: "Backend", plural: "Backends" },
  policy: { singular: "Policy", plural: "Policies" },
  routePolicy: { singular: "Route Policy", plural: "Route Policies" },
  corsPolicy: { singular: "CORS Policy", plural: "CORS Policies" },
  requestHeaderModifierPolicy: {
    singular: "Request Header Modifier",
    plural: "Request Header Modifiers",
  },
  responseHeaderModifierPolicy: {
    singular: "Response Header Modifier",
    plural: "Response Header Modifiers",
  },
  topLevelBackend: { singular: "Backend", plural: "Backends" },
  llm: { singular: "LLM Config", plural: "LLM Configs" },
  model: { singular: "Model", plural: "Models" },
  mcp: { singular: "MCP Config", plural: "MCP Configs" },
  frontendPolicies: {
    singular: "Frontend Policies",
    plural: "Frontend Policies",
  },
};
