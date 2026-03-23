# Traffic Hierarchy — Dev Reference

Route: `/traffic` — read-only visualization of the bind → listener → route → backend hierarchy.

## File Structure

```
ui/src/pages/Traffic/
├── TrafficPage.tsx              # Page entry: metrics dashboard + tree
├── components/
│   └── HierarchyTree.tsx        # Collapsible tree UI
├── hooks/
│   └── useTrafficHierarchy.ts   # Data transform + validation hook
└── forms/                       # Manual TypeScript/JSON schemas
    ├── bindForm.ts              # LocalBind
    ├── listenerForm.ts          # LocalListener
    ├── routeForm.ts             # LocalRoute
    └── backendForm.ts           # LocalRouteBackend
```

## Run Locally

```bash
cd ui && yarn dev
# navigate to http://localhost:5173/traffic
```

## Add to Sidebar

In `MainLayout.tsx`:
```typescript
{ key: "traffic", icon: <Activity />, label: <Link to="/traffic">Traffic</Link> }
```

## Add CRUD

Import forms and wire up the API:
```typescript
import Form from "@rjsf/antd";
import { validator } from "../../utils/validator";
import { forms } from "./forms";
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

## Validation Checks

| Level | Check |
|-------|-------|
| Bind | No listeners |
| Listener | Duplicate hostname+port; HTTP protocol with TCP routes; no routes |
| Route | TCP listener with HTTP match conditions; no backends |

## More Detail

See [traffic-hierarchy-ai.md](traffic-hierarchy-ai.md) for schema authoring patterns, hook internals, and how to extend with new node types.
