# RJSF Form Generation Context

This document provides comprehensive guidance for creating and maintaining forms using React JSON Schema Form (RJSF) with Ant Design theme.

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Schema Structure](#schema-structure)
3. [UI Schema](#ui-schema)
4. [Field Types](#field-types)
5. [Dependencies and Conditional Fields](#dependencies-and-conditional-fields)
6. [Arrays](#arrays)
7. [Validation](#validation)
8. [Widgets](#widgets)
9. [Common Patterns](#common-patterns)
10. [Best Practices](#best-practices)

---

## Core Concepts

### What is RJSF?

React JSON Schema Form (RJSF) is a React component that automatically generates forms from JSON Schema definitions.

- **JSON Schema** (RJSFSchema): Defines **what** fields exist and their validation rules
- **UI Schema** (UiSchema): Defines **how** fields are rendered and customized
- **Form Data**: The actual data values in the form

### Basic Form Structure

```typescript
import Form from "@rjsf/antd";
import validator from "@rjsf/validator-ajv8";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    name: { type: "string" }
  }
};

export const uiSchema: UiSchema = {
  name: {
    "ui:placeholder": "Enter name"
  }
};

export const defaultValues = {
  name: ""
};
```

---

## Schema Structure

### Basic Types

```typescript
{
  type: "string"   // Text input
  type: "number"   // Numeric input
  type: "integer"  // Integer input
  type: "boolean"  // Checkbox
  type: "object"   // Nested object
  type: "array"    // List of items
}
```

### Object Properties

```typescript
{
  type: "object",
  required: ["name", "port"],  // Required fields
  additionalProperties: false,  // Don't allow extra fields
  properties: {
    name: {
      type: "string",
      title: "Service Name",           // Field label
      description: "The service name", // Help text
      default: "my-service"            // Default value
    },
    port: {
      type: "integer",
      minimum: 1,                      // Validation: min value
      maximum: 65535,                  // Validation: max value
      default: 8080
    }
  }
}
```

### Enums (Dropdowns)

```typescript
{
  protocol: {
    type: "string",
    title: "Protocol",
    enum: ["HTTP", "HTTPS", "TCP"],    // Available options
    default: "HTTP"
  }
}
```

### String Validation

```typescript
{
  serviceName: {
    type: "string",
    pattern: "^[a-z][a-z0-9-]*$",     // Regex pattern
    minLength: 3,                      // Minimum length
    maxLength: 50                      // Maximum length
  }
}
```

---

## UI Schema

The UI Schema customizes how fields are rendered without changing validation.

### Common UI Options

```typescript
export const uiSchema: UiSchema = {
  "ui:title": "My Form Title",
  "ui:description": "Form description",

  fieldName: {
    "ui:widget": "textarea",           // Override widget type
    "ui:placeholder": "Enter value",   // Placeholder text
    "ui:help": "Additional guidance",  // Help text below field
    "ui:title": "Custom Label",        // Override field label
    "ui:description": "Field desc",    // Override description
    "ui:disabled": true,               // Disable field
    "ui:readonly": true,               // Make read-only
    "ui:autofocus": true,              // Auto-focus on load
    "ui:options": {
      // Widget-specific options
    }
  }
};
```

### Hiding Fields

To hide a field but preserve its value:

```typescript
{
  hiddenField: {
    "ui:widget": "hidden"
  }
}
```

### Field Ordering

Control the order of object properties:

```typescript
{
  "ui:order": ["name", "port", "protocol", "*"]  // "*" = all other fields
}
```

### Customizing Arrays

```typescript
{
  myArray: {
    "ui:options": {
      orderable: true,    // Show move up/down buttons (default: true)
      addable: true,      // Show add button (default: true)
      removable: true,    // Show remove button (default: true)
      copyable: false     // Show copy button (default: false)
    }
  }
}
```

---

## Field Types

### String Fields

```typescript
// Basic text input
{ type: "string" }

// Textarea
{
  type: "string",
  // In uiSchema:
  "ui:widget": "textarea",
  "ui:options": { rows: 5 }
}

// Password
{
  type: "string",
  // In uiSchema:
  "ui:widget": "password"
}

// Email
{
  type: "string",
  format: "email"  // Automatic email validation
}

// URL
{
  type: "string",
  format: "uri"
}

// Date
{
  type: "string",
  format: "date"      // Date picker
}

// DateTime
{
  type: "string",
  format: "date-time" // DateTime picker
}
```

### Number Fields

```typescript
// Standard number input
{
  type: "number",
  minimum: 0,
  maximum: 100,
  multipleOf: 0.1  // Step increment
}

// Integer with spinner
{
  type: "integer",
  // In uiSchema:
  "ui:widget": "updown"
}

// Range slider
{
  type: "number",
  minimum: 0,
  maximum: 100,
  // In uiSchema:
  "ui:widget": "range"
}
```

### Boolean Fields

```typescript
// Checkbox (default)
{ type: "boolean" }

// Radio buttons
{
  type: "boolean",
  // In uiSchema:
  "ui:widget": "radio"
}

// Dropdown
{
  type: "boolean",
  // In uiSchema:
  "ui:widget": "select"
}
```

---

## Dependencies and Conditional Fields

### Property Dependencies

Make fields required based on other fields:

```typescript
{
  type: "object",
  properties: {
    creditCard: { type: "string" },
    billingAddress: { type: "string" }
  },
  dependencies: {
    creditCard: ["billingAddress"]  // If creditCard filled, billingAddress required
  }
}
```

### Schema Dependencies (Conditional Fields)

Show/hide fields based on other field values:

```typescript
{
  type: "object",
  properties: {
    protocol: {
      type: "string",
      enum: ["HTTP", "HTTPS"]
    }
  },
  dependencies: {
    protocol: {
      oneOf: [
        {
          // When protocol is HTTP
          properties: {
            protocol: { const: "HTTP" }
          }
        },
        {
          // When protocol is HTTPS, show TLS fields
          properties: {
            protocol: { const: "HTTPS" },
            tls: {
              type: "object",
              properties: {
                cert: { type: "string" },
                key: { type: "string" }
              },
              required: ["cert", "key"]
            }
          },
          required: ["tls"]
        }
      ]
    }
  }
}
```

### Discriminator Pattern (Recommended)

For complex conditional logic, use a discriminator field:

```typescript
{
  type: "object",
  required: ["type"],
  properties: {
    type: {
      type: "string",
      title: "Backend Type",
      enum: ["service", "host", "dynamic"],
      default: "service"
    }
  },
  dependencies: {
    type: {
      oneOf: [
        {
          properties: {
            type: { const: "service" },
            service: {
              type: "object",
              properties: {
                name: { type: "string" },
                port: { type: "integer" }
              },
              required: ["name", "port"]
            }
          },
          required: ["service"]
        },
        {
          properties: {
            type: { const: "host" },
            host: { type: "string" }
          },
          required: ["host"]
        }
      ]
    }
  }
}
```

**Important:** With oneOf/dependencies, ONLY the fields from the matching branch will appear.

---

## Arrays

### Basic Array

```typescript
{
  tags: {
    type: "array",
    title: "Tags",
    items: {
      type: "string"
    },
    default: []
  }
}
```

### Array of Objects

```typescript
{
  backends: {
    type: "array",
    title: "Backend Services",
    items: {
      type: "object",
      properties: {
        host: { type: "string" },
        port: { type: "integer" }
      },
      required: ["host", "port"]
    },
    default: []
  }
}
```

### Array Constraints

```typescript
{
  type: "array",
  minItems: 1,      // At least 1 item required
  maxItems: 10,     // Maximum 10 items
  uniqueItems: true // No duplicates
}
```

### Multiple Choice (Checkboxes)

```typescript
// Schema
{
  features: {
    type: "array",
    items: {
      type: "string",
      enum: ["feature1", "feature2", "feature3"]
    },
    uniqueItems: true
  }
}

// UI Schema
{
  features: {
    "ui:widget": "checkboxes",
    "ui:options": {
      inline: true  // Display checkboxes inline
    }
  }
}
```

---

## Validation

### Built-in Validation

RJSF automatically validates:
- `type` (string, number, boolean, etc.)
- `required` (required fields)
- `minimum`, `maximum` (number bounds)
- `minLength`, `maxLength` (string length)
- `pattern` (regex validation)
- `format` (email, uri, date, etc.)
- `enum` (allowed values)
- `minItems`, `maxItems` (array length)
- `uniqueItems` (array uniqueness)

### Pattern Validation

```typescript
{
  serviceName: {
    type: "string",
    pattern: "^[a-z][a-z0-9-]*$",  // Lowercase, alphanumeric, hyphens
    title: "Service Name"
  }
}
```

### Custom Error Messages

Use description to provide guidance:

```typescript
{
  port: {
    type: "integer",
    minimum: 1,
    maximum: 65535,
    description: "Port must be between 1 and 65535"
  }
}
```

---

## Widgets

### Available Widgets by Type

**String:**
- `text` (default)
- `textarea`
- `password`
- `email`
- `uri`
- `color`
- `file`
- `hidden`

**Number/Integer:**
- `text` (default)
- `updown` (spinner)
- `range` (slider)
- `radio` (requires enum)

**Boolean:**
- `checkbox` (default)
- `radio`
- `select`

**Enum (any type):**
- `select` (default dropdown)
- `radio` (radio buttons)

### Widget Usage

```typescript
// In uiSchema
{
  description: {
    "ui:widget": "textarea",
    "ui:options": {
      rows: 10
    }
  },
  password: {
    "ui:widget": "password"
  },
  count: {
    "ui:widget": "updown"
  }
}
```

---

## Common Patterns

### Hidden Fields That Preserve Data

Used for fields managed separately (like nested arrays):

```typescript
// Schema
{
  routes: {
    type: "array",
    items: { type: "object" },
    default: []
  }
}

// UI Schema
{
  routes: {
    "ui:widget": "hidden"  // Hides field, preserves value
  }
}
```

### Optional vs Required Fields

```typescript
{
  type: "object",
  required: ["name"],  // name is required
  properties: {
    name: { type: "string" },
    description: { type: "string" }  // description is optional
  }
}
```

### Nested Objects

```typescript
{
  type: "object",
  properties: {
    service: {
      type: "object",
      title: "Service Configuration",
      properties: {
        name: { type: "string" },
        port: { type: "integer" }
      }
    }
  }
}
```

### Transform Data Before Submit

If you need to transform form data before submission (e.g., strip UI-only fields), export a transform function:

```typescript
export function transformBeforeSubmit(data: unknown): unknown {
  // Remove UI-only fields, restructure data, etc.
  const { uiOnlyField, ...rest } = data as any;
  return rest;
}
```

Then use it in the form component before calling the API.

---

## Best Practices

### 1. Set `additionalProperties: false`

Prevents RJSF from showing "Key" fields for undefined properties:

```typescript
{
  type: "object",
  additionalProperties: false,  // Important!
  properties: { ... }
}
```

### 2. Always Define `items` for Arrays

Arrays must have an `items` schema:

```typescript
{
  type: "array",
  items: { type: "object" }  // Required, even if object is empty
}
```

### 3. Use Hidden Widget for Managed Lists

For arrays/objects managed by other forms:

```typescript
{
  backends: {
    "ui:widget": "hidden"
  }
}
```

### 4. Provide Good UX with Placeholders and Help Text

```typescript
{
  port: {
    "ui:placeholder": "8080",
    "ui:help": "The port number this service listens on (1-65535)"
  }
}
```

### 5. Use Discriminators for Complex Conditionals

Instead of trying to show all oneOf options, use a type selector:

```typescript
{
  backendType: {
    type: "string",
    enum: ["service", "host", "ai"]
  },
  dependencies: {
    backendType: { oneOf: [...] }
  }
}
```

### 6. Match Default Values to Schema

Ensure `defaultValues` matches the schema structure:

```typescript
// Schema
{
  service: {
    type: "object",
    properties: {
      name: { type: "string" },
      port: { type: "integer" }
    }
  }
}

// Default Values
{
  service: {
    name: "",
    port: 8080
  }
}
```

### 7. Validate Against Server Schema

When you get validation errors from the server, they indicate the correct structure. Example:

```
Error: "expected string for NamespacedHostname with format namespace/hostname"
Solution: Use type: "string" with pattern validation, not an object
```

### 8. Use `const` in oneOf for Exact Matches

```typescript
{
  oneOf: [
    {
      properties: {
        protocol: { const: "HTTP" }  // Use const, not enum with one value
      }
    }
  ]
}
```

### 9. Test with Real Data

Always test forms with actual server validation to ensure the schema matches the backend expectations.

### 10. Keep Schema and UI Schema Separate

- **Schema**: Validation, structure, data types
- **UI Schema**: Presentation, widgets, help text

Don't put UI concerns in the schema or validation in the UI schema.

---

## Common Gotchas

### Issue: "Key" Labels Appearing

**Problem:** `backends Key` or similar labels showing

**Solution:** Set `additionalProperties: false` in the schema

### Issue: "Unsupported field schema: Missing items definition"

**Problem:** Array field without `items` defined

**Solution:** Add `items: { type: "object" }` to array schema

### Issue: Both oneOf Options Showing

**Problem:** Multiple conditional branches appearing simultaneously

**Solution:** Use `dependencies` with `oneOf` and `const` values correctly

### Issue: Validation Errors on Save

**Problem:** "invalid type: map, expected string"

**Solution:** Server expects a different structure. Check error message for format hints (e.g., "namespace/hostname" means use string with pattern, not object)

### Issue: Form Data Not Saving

**Problem:** Data structure doesn't match backend expectations

**Solution:** Use `transformBeforeSubmit` function to reshape data before API call

---

## References

- [RJSF Documentation](https://rjsf-team.github.io/react-jsonschema-form/docs/)
- [JSON Schema Specification](https://json-schema.org/)
- [Ant Design Theme](https://rjsf-team.github.io/react-jsonschema-form/docs/usage/themes)

---

## Form File Structure

Each form file should export:

```typescript
import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { YourType } from "../../../config";

// JSON Schema definition
export const schema: RJSFSchema = { ... };

// UI customization
export const uiSchema: UiSchema = { ... };

// Default values for new items
export const defaultValues: Partial<YourType> = { ... };

// Optional: Type guard
export function isYourType(data: unknown): data is YourType {
  return typeof data === "object" && data !== null;
}

// Optional: Transform function
export function transformBeforeSubmit(data: unknown): unknown {
  // Transform form data before submission
  return data;
}
```
