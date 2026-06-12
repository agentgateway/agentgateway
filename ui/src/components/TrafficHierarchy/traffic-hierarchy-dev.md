# Traffic Hierarchy — Dev Reference

Reusable component set for visualizing and editing the bind → listener → route → backend hierarchy. Currently embedded in `TrafficPage` at `/traffic`, but not hardcoded to that route — all navigation is derived from `useLocation()` at runtime.

## File Structure

```
ui/src/components/TrafficHierarchy/
├── HierarchyTree.tsx            # Collapsible tree UI
├── NodeDetailView.tsx           # Detail panel for a selected node
├── TopLevelDrawer.tsx           # Drawer for top-level resource editing
├── TopLevelEditForm.tsx         # RJSF form wrapper for top-level resources
├── ResourceIcon.tsx             # Icon resolver by resource type
├── StyledButton.tsx             # Shared styled button
├── types.ts                     # Shared UrlParams interface
├── index.ts                     # Barrel export
├── hooks/
│   ├── useTrafficHierarchy.ts   # Data transform + validation hook
│   └── useNodePolling.ts        # Polls config until a newly created node appears
└── forms/                       # Manual TypeScript/JSON schemas
    ├── index.ts                 # Barrel export + resourceTypes/resourceLabels
    ├── bindForm.ts              # LocalBind
    ├── listenerForm.ts          # LocalListener
    ├── routeForm.ts             # LocalRoute
    ├── backendForm.ts           # LocalRouteBackend
    ├── topLevelBackendForm.ts   # Top-level backend resource
    ├── llmForm.ts               # LLM config
    ├── mcpForm.ts               # MCP config
    ├── modelForm.ts             # LLM model
    ├── policyForm.ts            # Policy base
    ├── routePolicyForm.ts       # Route-level policy
    ├── frontendPoliciesForm.ts  # Frontend policies
    ├── corsPolicyForm.ts        # CORS policy
    ├── requestHeaderModifierPolicyForm.ts
    └── responseHeaderModifierPolicyForm.ts

ui/src/pages/Traffic/
└── TrafficPage.tsx              # Page entry: metrics dashboard + tree (route: /traffic)

ui/src/pages/RawConfigPage/
├── RawConfigEditor.tsx          # Monaco-based raw config editor
└── RawConfigPage.tsx            # Page wrapper (base path derived from URL, not hardcoded)
```

## Run Locally

```bash
cd ui && yarn dev
# navigate to http://localhost:5173/traffic
```

## Use on a New Page

Import from the barrel and mount — navigation is relative to the page's current URL:

```typescript
import { HierarchyTree, NodeDetailView, useTrafficHierarchy } from "../../components/TrafficHierarchy";
```

`HierarchyTree` and `NodeDetailView` both call `useLocation()` internally to derive their base path. No props or config needed to make links relative.

## Add CRUD

Import forms and wire up the API:
```typescript
import Form from "@rjsf/antd";
import validator from "@rjsf/validator-ajv8";
import { forms } from "../../components/TrafficHierarchy/forms";
import * as api from "../../api/crud";

<Form
  schema={forms.listener.schema}
  uiSchema={forms.listener.uiSchema}
  formData={initialData ?? forms.listener.defaultValues}
  validator={validator}
  onSubmit={({ formData }) => handleSave(formData)}
/>

await api.createListener(port, listenerData);
await api.updateListener(port, name, listenerData);
await api.removeListener(port, name);
```

## Key Types

- `useTrafficHierarchy` returns `TrafficHierarchy`: binds tree + stats + loading/error state
- Each node has: original data, `errors[]`, `warnings[]`, children, metadata
- Each form exports: `schema`, `uiSchema`, `defaultValues`, type guard (`isLocalXxx`)
- `UrlParams` (from `types.ts`) — URL search params shape for node selection

## Validation Checks

| Level | Check |
|-------|-------|
| Bind | No listeners |
| Listener | Duplicate hostname+port; HTTP protocol with TCP routes; no routes |
| Route | TCP listener with HTTP match conditions; no backends |

## More Detail

See [traffic-hierarchy-ai.md](traffic-hierarchy-ai.md) for schema authoring patterns, hook internals, and how to extend with new node types.
