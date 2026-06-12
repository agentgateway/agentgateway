# TrafficHierarchy Maintenance Guide

## Quick overview

The TrafficHierarchy component renders a tree of agentgateway config: binds → listeners → routes → backends/policies, plus top-level LLM, MCP, and frontend policy configs. Each node can be selected, edited, and deleted.

## Key files

| File | What it does |
|------|-------------|
| `hooks/useTrafficHierarchy.ts` | Parses raw config into typed node tree (BindNode, LLMNode, MCPNode, etc.) |
| `HierarchyTree.tsx` | Renders the Ant Design tree with context menus |
| `NodeDetailView.tsx` | Renders the detail/edit panel for the selected node |
| `policyTypes.ts` | **Single source of truth** for all policy types, labels, and scopes |
| `forms/` | RJSF form schemas for each node type |
| `types.ts` | URL param types for routing |

## Adding a new policy type

1. Add one entry to `POLICY_TYPES` in `policyTypes.ts`
2. Done — menus, tree nodes, and detail views pick it up automatically

## Adding a dedicated form for a policy type

Currently all policies use a generic JSON editor. To add a proper form:

1. Create `forms/myPolicyForm.ts` (copy `corsPolicyForm.ts` as a starting point)
2. Register it in `forms/index.ts`
3. In `NodeDetailView.tsx`, add a check in the `llmPolicy`/`mcpPolicy`/`policy` rendering sections to use your form when `policyType === "myPolicy"`

## How config is saved

- LLM/MCP policies: the handler reconstructs the full config object (config + models + all policies), updates the changed policy, and calls `api.createOrUpdateLLM()` / `api.createOrUpdateMCP()`
- Route policies: the handler reconstructs the parent route's `policies` object and calls `api.updateRouteByIndex()`
- The `policies` field is stripped from the parent form (LLM/MCP root) since policies are managed as child nodes

## Common gotchas

- **Child collections are NOT in form schemas**: Forms only contain fields the user edits directly. Child collections (e.g., `listeners` on bind, `routes`/`tcpRoutes` on listener, `models`/`policies` on LLM, `targets`/`policies` on MCP) are managed via the hierarchy tree context menus. The form schema must **not** include these fields, and `NodeDetailView` strips them from `formData` before rendering and merges them back before saving. Do **not** re-add these fields to form schemas.
- **`useTrafficHierarchy` strips children from `.config`**: The hook destructures child arrays out of the raw config (e.g., `const { targets, policies, ...config } = mcpConfig`). When reconstructing the full config for API calls, you must add children back: `{ ...hierarchy.mcp.config, targets: hierarchy.mcp.targets.map(t => t.target), policies: ... }`.
- **MCP `mcp` field type**: Changed from `unknown | null` to `MCPNode | null` in the hierarchy. Any code reading `hierarchy.mcp` now gets the typed `MCPNode` with `.config` and `.policies`.
- **URL patterns**: Policy URLs are `{basePath}/llm/policy/{policyType}` and `{basePath}/mcp/policy/{policyType}`. The `urlToSelectedKey` function in HierarchyTree must match these before the generic `/llm` or `/mcp` patterns.
