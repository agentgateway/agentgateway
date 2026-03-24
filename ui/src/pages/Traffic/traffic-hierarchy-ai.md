# Traffic Hierarchy — Implementation Guide

## Overview

The Traffic page (`ui/src/pages/Traffic/`) visualizes the bind → listener → route → backend hierarchy using manually authored TypeScript/JSON schemas. This gives full control over form UI and validation at the cost of manual maintenance when types change.

Key files:
- [TrafficPage.tsx](TrafficPage.tsx) — metrics + tree layout
- [HierarchyTree.tsx](components/HierarchyTree.tsx) — tree component
- [useTrafficHierarchy.ts](hooks/useTrafficHierarchy.ts) — data transform + validation hook
- [forms/](forms/) — schema definitions
- [ui/src/config.d.ts](../../config.d.ts) — source of truth for types
- [ui/src/api/crud.ts](../../api/crud.ts) — API CRUD operations

---

## Schema Authoring

Each file in `forms/` exports four things. Use this as a template:

```typescript
import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalListener } from "../../../config";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    name: { type: "string", title: "Name", description: "Unique name for this listener" },
    hostname: { type: "string", title: "Hostname", default: "*" },
    protocol: {
      type: "string",
      enum: ["HTTP", "HTTPS", "TLS", "TCP", "HBONE"],
      default: "HTTP",
    },
  },
};

export const uiSchema: UiSchema = {
  name: { "ui:placeholder": "e.g., main-listener" },
  protocol: { "ui:widget": "select" },
};

export const defaultValues: Partial<LocalListener> = {
  hostname: "*",
  protocol: "HTTP",
};

export function isLocalListener(data: unknown): data is LocalListener {
  return typeof data === "object" && data !== null;
}
```

### Schema Patterns

**Conditional fields** (show TLS config only for HTTPS):
```typescript
dependencies: {
  protocol: {
    oneOf: [
      { properties: { protocol: { enum: ["HTTP"] } } },
      {
        properties: {
          protocol: { enum: ["HTTPS"] },
          tls: { type: "object", properties: { cert: { type: "string" }, key: { type: "string" } }, required: ["cert", "key"] },
        },
        required: ["tls"],
      },
    ],
  },
}
```

**Union types** (discriminated backend types):
```typescript
oneOf: [
  { title: "Service Backend", properties: { service: { /* ... */ } } },
  { title: "Host Backend", properties: { host: { type: "string" } } },
]
```

**Arrays and nested objects:**
```typescript
hostnames: { type: "array", items: { type: "string" } },
tls: { type: "object", properties: { cert: { type: "string" } }, required: ["cert"] },
```

### UI Schema Options

| Option | Values | Effect |
|--------|--------|--------|
| `"ui:widget"` | `"select"`, `"textarea"`, `"updown"`, `"radio"` | Override input widget |
| `"ui:placeholder"` | string | Input placeholder text |
| `"ui:help"` | string | Help text below field |
| `"ui:order"` | `["field1", "field2", "*"]` | Field render order |
| `"ui:options": { orderable, addable, removable }` | booleans | Array field controls |

**When types change in `config.d.ts`**: update the corresponding `forms/*.ts` schema manually — fields, enums, defaults, and the type guard. The TypeScript type annotation on `defaultValues` will flag drift at compile time.

---

## `useTrafficHierarchy` Hook

Transforms raw config into a typed tree with validation annotations.

```typescript
interface TrafficHierarchy {
  binds: BindNode[];
  stats: {
    totalBinds: number;
    totalListeners: number;
    totalRoutes: number;
    totalBackends: number;
    totalValidationErrors: number; // sum of all errors + warnings
  };
  isLoading: boolean;
  error: Error | undefined;
}

interface BindNode {
  data: LocalBind;
  port: number;
  listeners: ListenerNode[];
  errors: string[];
  warnings: string[];
}
// ListenerNode → RouteNode → BackendNode follow the same shape
```

**To add a validation rule**: push a string to `node.errors` or `node.warnings` inside the hook's transform function. Stats and badges update automatically.

---

## Validation Rules

All rules live in `useTrafficHierarchy.ts`:

| Level | Rule | Severity |
|-------|------|----------|
| Bind | No listeners | warning |
| Listener | Duplicate hostname+port | warning |
| Listener | HTTP protocol with TCP routes | warning |
| Listener | No routes | warning |
| Route | TCP listener with HTTP match conditions | warning |
| Route | No backends | warning |

---

## Adding CRUD Operations

The page is currently read-only. To add create/edit/delete:

1. **Form modal** — use RJSF with the existing schemas:
   ```typescript
   import Form from "@rjsf/antd";
   import { validator } from "../../utils/validator";
   import { forms } from "./forms";

   <Form
     schema={forms.listener.schema}
     uiSchema={forms.listener.uiSchema}
     formData={selectedListener ?? forms.listener.defaultValues}
     validator={validator}
     onSubmit={({ formData }) => handleSave(formData)}
   />
   ```

2. **Type-safe submit handler**:
   ```typescript
   function handleSave(data: unknown) {
     if (forms.listener.isLocalListener(data)) {
       await api.updateListener(port, name, data); // data is LocalListener here
     }
   }
   ```

3. **Wire up API** (`ui/src/api/crud.ts`):
   ```typescript
   await api.createListener(port, listenerData);
   await api.updateListener(port, name, listenerData);
   await api.removeListener(port, name);
   ```

4. **Add tree actions** — edit `HierarchyTree.tsx` to add edit/delete buttons on node hover.

---

## Adding a New Node Type

1. Create `forms/myTypeForm.ts` using the four-export template above.
2. Re-export from `forms/index.ts`.
3. Add a node interface in `useTrafficHierarchy.ts` and wire it into the transform.
4. Add validation rules in the hook.
5. Add a renderer in `HierarchyTree.tsx`.

---

## Maintenance Notes

- **Schema drift**: `config.d.ts` is generated from the backend schema. After backend type changes, update affected `forms/*.ts` files (fields, enums, defaults).
- **No build step**: schemas are plain TypeScript objects — no codegen needed.
- **Stats**: `totalValidationErrors` is derived from all node `errors` and `warnings` arrays. New rules increment it automatically.
- **Finding usages**: use IDE "Find References" on a form export (e.g., `forms.listener.schema`) to locate all consumers.
