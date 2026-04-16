/**
 * Traffic Forms Index
 *
 * Central export point for all manually configured form definitions.
 * Unlike Traffic2, these schemas are NOT generated from JSON files,
 * but are handcrafted TypeScript schemas that use the types from config.d.ts.
 */

import * as apiKeyPolicyForm from "./apiKeyPolicyForm";
import * as authorizationPolicyForm from "./authorizationPolicyForm";
import * as backendForm from "./backendForm";
import * as basicAuthPolicyForm from "./basicAuthPolicyForm";
import * as bindForm from "./bindForm";
import * as corsPolicyForm from "./corsPolicyForm";
import * as frontendPoliciesForm from "./frontendPoliciesForm";
import * as genericPolicyForm from "./genericPolicyForm";
import * as jwtAuthPolicyForm from "./jwtAuthPolicyForm";
import * as listenerForm from "./listenerForm";
import * as llmForm from "./llmForm";
import * as mcpAuthenticationPolicyForm from "./mcpAuthenticationPolicyForm";
import * as mcpForm from "./mcpForm";
import * as mcpTargetForm from "./mcpTargetForm";
import * as modelForm from "./modelForm";
import * as policyForm from "./policyForm";
import * as requestHeaderModifierPolicyForm from "./requestHeaderModifierPolicyForm";
import * as responseHeaderModifierPolicyForm from "./responseHeaderModifierPolicyForm";
import * as routeForm from "./routeForm";
import * as routePolicyForm from "./routePolicyForm";
import * as topLevelBackendForm from "./topLevelBackendForm";
import * as transformationsPolicyForm from "./transformationsPolicyForm";

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
  llmPolicy: genericPolicyForm,
  mcpPolicy: genericPolicyForm,
  mcpTarget: mcpTargetForm,
  authorizationPolicy: authorizationPolicyForm,
  transformationsPolicy: transformationsPolicyForm,
  jwtAuthPolicy: jwtAuthPolicyForm,
  basicAuthPolicy: basicAuthPolicyForm,
  apiKeyPolicy: apiKeyPolicyForm,
  mcpAuthenticationPolicy: mcpAuthenticationPolicyForm,
};

const POLICY_FORM_MAP: Partial<Record<string, keyof typeof forms>> = { 
  cors: "corsPolicy",
  requestHeaderModifier: "requestHeaderModifierPolicy",
  responseHeaderModifier: "responseHeaderModifierPolicy",
  authorization: "authorizationPolicy",
  mcpAuthorization: "authorizationPolicy",
  transformations: "transformationsPolicy",
  jwtAuth: "jwtAuthPolicy",
  basicAuth: "basicAuthPolicy",
  apiKey: "apiKeyPolicy",
  mcpAuthentication: "mcpAuthenticationPolicy",
}

export function getFormForPolicy(policyType: string, fallback: keyof typeof forms = "routePolicy") { 
  const key = POLICY_FORM_MAP[policyType] ?? fallback;
  return forms[key];
}

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
  llmPolicy: { singular: "LLM Policy", plural: "LLM Policies" },
  mcpPolicy: { singular: "MCP Policy", plural: "MCP Policies" },
  mcpTarget: { singular: "MCP Target", plural: "MCP Targets" },
  authorizationPolicy: { singular: "Authorization Policy", plural: "Authorization Policies" },
  transformationsPolicy: { singular: "Transformations Policy", plural: "Transformations Policies" },
  jwtAuthPolicy: { singular: "JWT Auth Policy", plural: "JWT Auth Policies" },
  basicAuthPolicy: { singular: "Basic Auth Policy", plural: "Basic Auth Policies" },
  apiKeyPolicy: { singular: "API Key Policy", plural: "API Key Policies" },
  mcpAuthenticationPolicy: { singular: "MCP Authentication Policy", plural: "MCP Authentication Policies" },
};
