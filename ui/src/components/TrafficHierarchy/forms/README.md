# Traffic Forms

Manual TypeScript/JSON Schema definitions for the Traffic hierarchy page.

## Files

### Core hierarchy forms
- **bindForm.ts** — `LocalBind` (port bindings)
- **listenerForm.ts** — `LocalListener` (protocol listeners)
- **routeForm.ts** — `LocalRoute` (HTTP/TCP routes)
- **backendForm.ts** — `LocalRouteBackend` (service/host/MCP backends)

### Top-level config forms
- **llmForm.ts** — `LocalLLMConfig` (LLM gateway settings, port only — models and policies are child nodes)
- **mcpForm.ts** — `LocalSimpleMcpConfig` (MCP gateway settings, targets, modes — policies are child nodes)
- **mcpTargetForm.ts** — `LocalMcpTarget` (individual MCP target config with sse/mcp/stdio/openapi variants)
- **frontendPoliciesForm.ts** — `LocalFrontendPolicies` (http/tls/tcp/accessLog/tracing)
- **modelForm.ts** — `LocalLLMModels` (individual LLM model config)

### Policy forms

There are two distinct policy contexts:

1. **Top-level named policies** (`config.policies[]`) — standalone policy objects with name, target, and phase wrapping
   - **policyForm.ts** — form for `LocalPolicy`

2. **Inline policies** — individual policy types shown as child tree nodes under LLM, MCP, or route nodes
   - **genericPolicyForm.ts** — generic JSON editor, used for LLM and MCP policy nodes
   - **routePolicyForm.ts** — fallback for route policy types without a dedicated form
   - **corsPolicyForm.ts** — dedicated CORS form (used for route `cors` policy)
   - **requestHeaderModifierPolicyForm.ts** — dedicated request header form
   - **responseHeaderModifierPolicyForm.ts** — dedicated response header form

### Other
- **topLevelBackendForm.ts** — Top-level backend definitions
- **index.ts** — Central export registry for all forms

## How policies work

Policies are managed as **child nodes** in the hierarchy tree, not as inline fields in the parent form.

- **LLM policies** (`LocalLLMPolicy`) — 7 types: jwtAuth, basicAuth, apiKey, extAuthz, extProc, transformations, authorization
- **MCP policies** (`FilterOrPolicy`) — 25 types: auth, headers, rate limiting, CORS, etc.
- **Route policies** (`FilterOrPolicy`) — same 25 types as MCP

The available policy types for each scope are defined in `../policyTypes.ts`. That file is the single source of truth for policy labels, descriptions, and which scopes each type applies to.

## Adding a new policy type

1. Add an entry to `POLICY_TYPES` in `../policyTypes.ts` with the key, label, description, and scopes
2. That's it — the tree menus, detail views, and forms all read from the registry

To add a **dedicated form** (instead of the generic JSON editor) for a specific policy type:
1. Create a new form file (e.g., `jwtAuthPolicyForm.ts`) following `corsPolicyForm.ts` as a template
2. Register it in `index.ts`
3. Update `NodeDetailView.tsx` to use the dedicated form when `policyType` matches

For schema authoring patterns, UI schema options, and maintenance guidance see [traffic-hierarchy-ai.md](../traffic-hierarchy-ai.md).
