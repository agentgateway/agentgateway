# Array Fields in Policy Forms

## Problem
When using the generic policy form, RJSF cannot render array fields properly because it lacks the `items` definition in the schema. This causes "Unsupported field schema: Missing items definition" errors.

## Solution
Create dedicated form definitions for policies with array fields, providing proper schema with `items` definitions.

## Example: Authorization Policy (rules array)

The authorization policy has a `rules` field that is an array of strings. We created `authorizationPolicyForm.ts` with:

```typescript
export const schema: RJSFSchema = {
  type: "object",
  required: ["rules"],
  properties: {
    rules: {
      type: "array",
      title: "Authorization Rules",
      description: "CEL expressions that evaluate to true for authorized requests",
      items: {
        type: "string",
        title: "Rule",
      },
      default: [],
    },
  },
};

export const uiSchema: UiSchema = {
  rules: {
    "ui:options": {
      orderable: false,
      addable: true,
      removable: true,
    },
    items: {
      "ui:widget": "textarea",
      "ui:placeholder": "e.g., request.headers['x-user-role'] == 'admin'",
      "ui:options": {
        rows: 2,
      },
    },
  },
};
```

## How to Add Array Support for Other Policies

### Step 1: Create a Form Definition
Create a new file in `forms/` directory (e.g., `myPolicyForm.ts`):

```typescript
import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    myArrayField: {
      type: "array",
      title: "My Array Field",
      items: {
        // For string arrays:
        type: "string",
        // For object arrays:
        // type: "object",
        // properties: { ... }
      },
      default: [],
    },
  },
};

export const uiSchema: UiSchema = {
  myArrayField: {
    items: {
      // Customize how each item is rendered
      "ui:widget": "textarea", // or "text", "select", etc.
    },
  },
};
```

### Step 2: Register the Form
Add it to `forms/index.ts`:

```typescript
import * as myPolicyForm from "./myPolicyForm";

export const forms = {
  // ... existing forms
  myPolicy: myPolicyForm,
};
```

### Step 3: Use in NodeDetailView
Update the form selection logic in `NodeDetailView.tsx`:

```typescript
const formSchema = 
  policyType === "myPolicy"
    ? forms.myPolicy
    : forms.llmPolicy; // fallback to generic
```

## Array Types

### String Arrays
```typescript
items: {
  type: "string"
}
```

### Object Arrays
```typescript
items: {
  type: "object",
  properties: {
    name: { type: "string" },
    value: { type: "string" }
  },
  required: ["name"]
}
```

### Enum Arrays
```typescript
items: {
  type: "string",
  enum: ["option1", "option2", "option3"]
}
```

## UI Customization

The `ArrayFieldTemplate` component (in `FormTemplates/ArrayFieldTemplate.tsx`) automatically renders:
- Each item in a Card with a remove button
- An "Add" button at the bottom
- Proper spacing and styling

Customize array behavior via `ui:options`:
```typescript
{
  "ui:options": {
    orderable: false,  // Hide move up/down buttons
    addable: true,     // Show add button
    removable: true,   // Show remove buttons
  }
}
```

Customize individual items:
```typescript
{
  items: {
    "ui:widget": "textarea",
    "ui:placeholder": "Enter value...",
    "ui:options": {
      rows: 3
    }
  }
}
```
