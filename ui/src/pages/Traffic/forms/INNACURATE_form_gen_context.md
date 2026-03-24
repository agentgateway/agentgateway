# JSON Forms — Complete AI Reference

This document is a comprehensive reference for the **JSON Forms** library (v3.7.0), compiled from the official documentation at https://jsonforms.io/docs. It is intended to be consumed by an AI assistant generating or editing forms in this codebase.

---

## Table of Contents

1. [What is JSON Forms?](#1-what-is-json-forms)
2. [Core Concepts](#2-core-concepts)
3. [React Integration](#3-react-integration)
4. [JSON Schema (Data Schema)](#4-json-schema-data-schema)
5. [UI Schema](#5-ui-schema)
   - [Layouts](#51-layouts)
   - [Controls](#52-controls)
   - [Rules (Dynamic Behavior)](#53-rules-dynamic-behavior)
6. [Multiple Choice / Enums](#6-multiple-choice--enums)
7. [Arrays](#7-arrays)
8. [ReadOnly / Disabled](#8-readonly--disabled)
9. [Validation](#9-validation)
10. [i18n / Localization](#10-i18n--localization)
11. [Renderer Sets](#11-renderer-sets)
12. [Custom Renderers](#12-custom-renderers)
13. [Custom Layouts](#13-custom-layouts)
14. [Tutorial: Full App Setup](#14-tutorial-full-app-setup)

---

## 1. What is JSON Forms?

JSON Forms is a declarative framework for building form-based web UIs from JSON schemas. It uses two schema artifacts:

- **JSON Schema** (data schema): defines the shape and types of the data (objects, properties, types, validation).
- **UI Schema**: defines how the data is rendered (layout, order, visibility, grouping, etc.).

Both are interpreted at runtime, producing fully data-bound, validated forms with minimal hand-written HTML or JS.

---

## 2. Core Concepts

### Rendering Pipeline

1. JSON Forms reads the **JSON schema** to understand data types.
2. JSON Forms reads the **UI schema** to determine layout and control configuration.
3. JSON Forms looks up a **renderer** for each UI schema element using its **tester registry** (highest-ranked tester wins).
4. Renderers are React (or Angular/Vue) components that render the appropriate input widget with data binding and validation.

### Testers

Every renderer has an associated **tester**—a function `(uischema, schema) => number` that returns a rank (`-1` / `NOT_APPLICABLE` means skip). The highest non-negative rank wins. Default renderers use rank `1` or `2`; custom renderers should use rank `3+` to override.

---

## 3. React Integration

### The `JsonForms` Component

```jsx
import { JsonForms } from '@jsonforms/react';
import { materialRenderers, materialCells } from '@jsonforms/material-renderers';

<JsonForms
  schema={schema}        // JSON Schema describing data
  uischema={uischema}    // UI Schema describing layout
  data={data}            // Current form data object
  renderers={materialRenderers}
  cells={materialCells}
  onChange={({ data, errors }) => setData(data)}
/>
```

### Props

| Prop | Type | Description |
|---|---|---|
| `schema` | object | JSON Schema describing the underlying data. If omitted, JSON Forms generates one from `data`. |
| `uischema` | object | UI Schema describing layout. If omitted, JSON Forms generates a flat vertical layout. |
| `data` | object | The current data to render and bind. |
| `renderers` | array | Renderer set (e.g. `materialRenderers`). |
| `cells` | array | Cell renderer set for table/simple use cases. |
| `onChange` | function | `({ data, errors }) => void`. Called on every change, including initial validation. |
| `readonly` | boolean | Disables all inputs form-wide. |
| `validationMode` | string | `"ValidateAndShow"` (default), `"ValidateAndHide"`, `"NoValidation"`. |
| `additionalErrors` | array | External AJV-style errors to mix in (e.g. from backend validation). |
| `config` | object | Default options for all UI schema elements (see below). |
| `uischemas` | array | Registered UI schemas used for dynamic dispatch in arrays/objects. |
| `i18n` | object | `{ locale, translate, translateError }` — internationalization. |
| `ajv` | Ajv | Custom AJV instance for validation. |

### `config` Prop Options

```js
{
  restrict: false,               // Restrict number of chars to `maxLength`
  trim: false,                   // Controls grab full width
  showUnfocusedDescription: false, // Show descriptions when not focused
  hideRequiredAsterisk: false    // Hide asterisks on required fields
}
```

UI schema `options` properties take precedence over `config`.

### Minimal Working Example

```jsx
import React, { useState } from 'react';
import { JsonForms } from '@jsonforms/react';
import { materialRenderers, materialCells } from '@jsonforms/material-renderers';

const schema = {
  type: 'object',
  properties: {
    name: { type: 'string', minLength: 1 },
    age: { type: 'integer', minimum: 0 },
  },
  required: ['name'],
};

const uischema = {
  type: 'VerticalLayout',
  elements: [
    { type: 'Control', scope: '#/properties/name' },
    { type: 'Control', scope: '#/properties/age' },
  ],
};

export default function App() {
  const [data, setData] = useState({});
  return (
    <JsonForms
      schema={schema}
      uischema={uischema}
      data={data}
      renderers={materialRenderers}
      cells={materialCells}
      onChange={({ data }) => setData(data)}
    />
  );
}
```

---

## 4. JSON Schema (Data Schema)

The data schema follows the [JSON Schema specification](https://json-schema.org/). JSON Forms supports a broad subset. Key constructs:

### Primitive Types

```json
{
  "type": "string",
  "minLength": 2,
  "maxLength": 100
}
```

```json
{ "type": "integer", "minimum": 0, "maximum": 150 }
```

```json
{ "type": "number" }
```

```json
{ "type": "boolean" }
```

### String Formats

```json
{ "type": "string", "format": "date" }
{ "type": "string", "format": "time" }
{ "type": "string", "format": "date-time" }
```

These render as specialized date/time pickers.

### Enums

```json
{
  "type": "string",
  "enum": ["Option A", "Option B", "Option C"]
}
```

### oneOf Enum (with titles)

```json
{
  "type": "string",
  "oneOf": [
    { "const": "a", "title": "Option A" },
    { "const": "b", "title": "Option B" }
  ]
}
```

### Objects

```json
{
  "type": "object",
  "properties": {
    "street": { "type": "string" },
    "city": { "type": "string" }
  },
  "required": ["city"]
}
```

### Arrays

```json
{
  "type": "array",
  "items": {
    "type": "object",
    "properties": {
      "name": { "type": "string" },
      "message": { "type": "string" }
    }
  }
}
```

### Array of Enums (Multi-select)

```json
{
  "type": "array",
  "uniqueItems": true,
  "items": {
    "type": "string",
    "enum": ["foo", "bar", "baz"]
  }
}
```

### ReadOnly in Schema

```json
{
  "properties": {
    "id": { "type": "string", "readOnly": true }
  }
}
```

Note: Only `readOnly` (camelCase) is respected per the JSON Schema spec. `readonly` is ignored.

### Combining Schemas

- `oneOf`: exactly one subschema must match — renders as tabs or a selector.
- `anyOf`: one or more subschemas can match.
- `allOf`: all subschemas must match.

---

## 5. UI Schema

The UI schema is a plain JSON object describing how to render the form. It is composed of **layouts** and **controls**.

### Common Shape

```json
{
  "type": "<ElementType>",
  "elements": [...],   // for layouts
  "scope": "...",      // for controls
  "label": "...",      // optional label override
  "options": {...},    // renderer-specific options
  "rule": {...}        // dynamic show/hide/enable/disable
}
```

---

### 5.1 Layouts

Layouts arrange their `elements` children. All layouts require an `elements` array.

#### VerticalLayout

Stacks elements top-to-bottom.

```json
{
  "type": "VerticalLayout",
  "elements": [
    { "type": "Control", "scope": "#/properties/name" },
    { "type": "Control", "scope": "#/properties/age" }
  ]
}
```

#### HorizontalLayout

Places elements side-by-side; each child gets equal width (1/n space).

```json
{
  "type": "HorizontalLayout",
  "elements": [
    { "type": "Control", "scope": "#/properties/firstName" },
    { "type": "Control", "scope": "#/properties/lastName" }
  ]
}
```

#### Group

Like `VerticalLayout` but includes a visible **label** (title/header). Label is mandatory.

```json
{
  "type": "Group",
  "label": "Personal Information",
  "elements": [
    { "type": "Control", "scope": "#/properties/name" },
    { "type": "Control", "scope": "#/properties/birthDate" }
  ]
}
```

#### Categorization

Tab-based layout. Children must be of type `Category`. Each `Category` has a `label` and `elements`.

```json
{
  "type": "Categorization",
  "elements": [
    {
      "type": "Category",
      "label": "Personal Data",
      "elements": [
        { "type": "Control", "scope": "#/properties/firstName" },
        { "type": "Control", "scope": "#/properties/lastName" }
      ]
    },
    {
      "type": "Category",
      "label": "Address",
      "elements": [
        { "type": "Control", "scope": "#/properties/street" },
        { "type": "Control", "scope": "#/properties/city" }
      ]
    }
  ]
}
```

Categorizations also support a **stepper** variant and **navigation buttons** (renderer-specific options).

#### Nested Layouts

Layouts can be nested arbitrarily:

```json
{
  "type": "Group",
  "label": "My Group",
  "elements": [
    {
      "type": "HorizontalLayout",
      "elements": [
        {
          "type": "VerticalLayout",
          "elements": [
            { "type": "Control", "scope": "#/properties/firstName" }
          ]
        },
        {
          "type": "VerticalLayout",
          "elements": [
            { "type": "Control", "scope": "#/properties/lastName" }
          ]
        }
      ]
    }
  ]
}
```

---

### 5.2 Controls

Controls render individual form fields. They bind to a property via `scope`.

#### Basic Control

```json
{
  "type": "Control",
  "scope": "#/properties/name"
}
```

The `scope` is a JSON Pointer prefixed with `#`. It navigates the JSON schema structure.

#### Custom Label

```json
{
  "type": "Control",
  "scope": "#/properties/name",
  "label": "First Name"
}
```

Set `"label": false` to suppress the label entirely.

#### Nested Properties

```json
{ "scope": "#/properties/address/properties/street" }
```

#### Default Renderer Mapping (by JSON Schema type)

| JSON Schema type / format | Default rendering |
|---|---|
| `string` | Text input |
| `string` + `"format": "date"` | Date picker |
| `string` + `"format": "time"` | Time picker |
| `string` + `"format": "date-time"` | DateTime picker |
| `string` with `enum` | Dropdown (combo) |
| `string` with `oneOf` (const+title) | Dropdown (combo) |
| `integer` / `number` | Number input |
| `boolean` | Checkbox |
| `object` | Vertical grid of sub-controls |
| `array` of primitives | List |
| `array` of objects | Table |
| `array` with `uniqueItems` + `enum`/`oneOf` | Multi-select checkboxes |
| `oneOf` / `anyOf` / `allOf` | Tab switcher |

#### Control Options

Options are passed via the `options` property on a control:

```json
{
  "type": "Control",
  "scope": "#/properties/description",
  "options": {
    "multi": true
  }
}
```

Common options:

| Option | Type | Description |
|---|---|---|
| `multi` | boolean | Render string as multi-line textarea |
| `format` | `"radio"` \| `"autocomplete"` | Override rendering of enum/oneOf |
| `readonly` | boolean | Disable this specific control |
| `detail` | string \| object | How to render array items detail view. Values: `"DEFAULT"`, `"GENERATED"`, `"REGISTERED"`, or an inline UI schema object |
| `showSortButtons` | boolean | Show up/down sort buttons on array items |
| `elementLabelProp` | string \| string[] | Property path to use as label for array items |

#### Radio Group Example

```json
{
  "type": "Control",
  "scope": "#/properties/gender",
  "options": {
    "format": "radio"
  }
}
```

#### Autocomplete Example

```json
{
  "type": "Control",
  "scope": "#/properties/country",
  "options": {
    "autocomplete": true
  }
}
```

#### Array with Inline Detail UI Schema

```json
{
  "type": "Control",
  "scope": "#/properties/comments",
  "options": {
    "detail": {
      "type": "HorizontalLayout",
      "elements": [
        { "type": "Control", "scope": "#/properties/author" },
        { "type": "Control", "scope": "#/properties/message" }
      ]
    }
  }
}
```

---

### 5.3 Rules (Dynamic Behavior)

Rules allow dynamic show/hide/enable/disable of any UI schema element based on current data.

#### Structure

```json
{
  "type": "Control",
  "scope": "#/properties/someField",
  "rule": {
    "effect": "HIDE",
    "condition": {
      "scope": "#/properties/otherField",
      "schema": { "const": "triggerValue" }
    }
  }
}
```

#### Effects

| Effect | Behavior when condition is true |
|---|---|
| `"SHOW"` | Show the element |
| `"HIDE"` | Hide the element |
| `"ENABLE"` | Enable the element |
| `"DISABLE"` | Disable the element |

#### Condition

The `condition` object has:
- `scope`: JSON Pointer to the property to evaluate against.
- `schema`: A JSON Schema fragment. If the scoped data validates against it, the condition is true.
- `failWhenUndefined` (optional, boolean): If true, condition fails when scope resolves to `undefined`.

#### Common Condition Patterns

**Match exact value:**
```json
{ "scope": "#/properties/type", "schema": { "const": "advanced" } }
```

**Match multiple values:**
```json
{ "scope": "#/properties/status", "schema": { "enum": ["active", "pending"] } }
```

**Negate:**
```json
{ "scope": "#/properties/enabled", "schema": { "not": { "const": true } } }
```

**Numeric range:**
```json
{ "scope": "#/properties/count", "schema": { "minimum": 1, "exclusiveMaximum": 10 } }
```

**Require undefined to fail:**
```json
{
  "scope": "#/properties/count",
  "schema": { "minimum": 1 },
  "failWhenUndefined": true
}
```

**Multi-property condition (scope to root `"#"`):**
```json
{
  "scope": "#",
  "schema": {
    "properties": {
      "tags": { "contains": { "const": "admin" } }
    },
    "required": ["tags", "role"]
  }
}
```

#### Rules on Layouts

Rules can be attached to layouts too, not just controls:

```json
{
  "type": "Group",
  "label": "Shipping Address",
  "elements": [...],
  "rule": {
    "effect": "SHOW",
    "condition": {
      "scope": "#/properties/needsShipping",
      "schema": { "const": true }
    }
  }
}
```

---

## 6. Multiple Choice / Enums

### Single Select — `enum`

```json
{ "type": "string", "enum": ["foo", "bar", "foobar"] }
```

Renders as a dropdown by default.

### Single Select — `oneOf` (with display titles)

```json
{
  "type": "string",
  "oneOf": [
    { "const": "foo", "title": "Foo Label" },
    { "const": "bar", "title": "Bar Label" }
  ]
}
```

### Radio Buttons (for `enum` or `oneOf`)

```json
{
  "type": "Control",
  "scope": "#/properties/size",
  "options": { "format": "radio" }
}
```

### Autocomplete (for `enum` or `oneOf`)

```json
{
  "type": "Control",
  "scope": "#/properties/country",
  "options": { "autocomplete": true }
}
```

### Multi-Select — array of `enum`

```json
{
  "type": "array",
  "uniqueItems": true,
  "items": {
    "type": "string",
    "enum": ["foo", "bar", "foobar"]
  }
}
```

### Multi-Select — array of `oneOf`

```json
{
  "type": "array",
  "uniqueItems": true,
  "items": {
    "oneOf": [
      { "const": "foo", "title": "My Foo" },
      { "const": "bar", "title": "My Bar" }
    ]
  }
}
```

---

## 7. Arrays

### Basic Array (table or list)

Schema:
```json
{
  "type": "array",
  "items": {
    "type": "object",
    "properties": {
      "name": { "type": "string" },
      "message": { "type": "string", "maxLength": 5 },
      "category": { "type": "string", "enum": ["Info", "Warning", "Error"] }
    }
  }
}
```

UI schema:
```json
{
  "type": "Control",
  "scope": "#/properties/comments"
}
```

### Sort Buttons

```json
{
  "type": "Control",
  "scope": "#/properties/items",
  "options": { "showSortButtons": true }
}
```

### Custom Element Label

```json
{
  "type": "Control",
  "scope": "#/properties/items",
  "options": { "elementLabelProp": "name" }
}
```

### Inline Detail Layout

```json
{
  "type": "Control",
  "scope": "#/properties/items",
  "options": {
    "detail": {
      "type": "HorizontalLayout",
      "elements": [
        { "type": "Control", "scope": "#/properties/name" },
        { "type": "Control", "scope": "#/properties/value" }
      ]
    }
  }
}
```

---

## 8. ReadOnly / Disabled

### Form-wide (all inputs disabled)

```jsx
<JsonForms readonly ... />
```

### Per-control via UI Schema option

```json
{
  "type": "Control",
  "scope": "#/properties/id",
  "options": { "readonly": true }
}
```

### Per-property via JSON Schema

```json
{
  "properties": {
    "id": { "type": "string", "readOnly": true }
  }
}
```

### Via Rules (dynamic)

Always disabled:
```json
{
  "rule": {
    "effect": "DISABLE",
    "condition": { "scope": "#", "schema": {} }
  }
}
```

Always enabled (use `not: {}`):
```json
{
  "rule": {
    "effect": "ENABLE",
    "condition": { "scope": "#", "schema": { "not": {} } }
  }
}
```

### Evaluation Order (highest wins)

1. Parent state (inherited if none of below apply)
2. JSON Schema `readOnly: true`
3. UI Schema `options.readonly`
4. `ENABLE` / `DISABLE` rule
5. Form-wide `readonly` prop (highest priority)

---

## 9. Validation

JSON Forms uses [AJV](https://github.com/epoberezkin/ajv) for validation. Validation happens automatically on every data change.

### Validation Mode

```jsx
<JsonForms
  validationMode="ValidateAndShow"  // default
  // or "ValidateAndHide"
  // or "NoValidation"
  ...
/>
```

| Mode | Behavior |
|---|---|
| `ValidateAndShow` | Validate, emit errors, show in UI |
| `ValidateAndHide` | Validate, emit errors via `onChange`, but hide in UI |
| `NoValidation` | Skip validation entirely |

### Custom AJV Instance

```js
import Ajv from 'ajv';
import addFormats from 'ajv-formats';

const ajv = new Ajv({ allErrors: true, verbose: true, strict: false });
addFormats(ajv);

<JsonForms ajv={ajv} ... />
```

JSON Forms also exports `createAjv(options)` for convenience.

### External / Backend Errors

```js
const additionalErrors = [
  {
    instancePath: '/email',   // AJV-style path
    message: 'Email already taken',
    schemaPath: '',
    keyword: '',
    params: {},
  }
];

<JsonForms additionalErrors={additionalErrors} ... />
```

Note: `additionalErrors` are always shown regardless of `validationMode`.

---

## 10. i18n / Localization

### Setup

```tsx
import { useMemo, useState } from 'react';

const createTranslator = (locale) => (key, defaultMessage, context) => {
  const translations = { /* key: translated string */ };
  return translations[key] ?? defaultMessage;
};

const [locale, setLocale] = useState('en');
const translate = useMemo(() => createTranslator(locale), [locale]);

<JsonForms
  i18n={{ locale, translate }}
  ...
/>
```

### Key Structure

Keys are resolved in this order for field errors:

1. `<field>.error.custom` — override all errors for a field with one message
2. `<field>.error.<keyword>` — e.g. `name.error.required`
3. `error.<keyword>` — global error override e.g. `error.required`
4. Default AJV error message

### Enum Translation Keys

- For `enum`: `<fieldPath>.<value>` e.g. `gender.male`
- For `oneOf` with `title`: `<fieldPath>.<title>` e.g. `gender.Male`
- For `oneOf` with `i18n` on entry: `<i18nValue>` directly

### Setting Custom i18n Keys

In UI schema:
```json
{ "type": "Control", "scope": "#/properties/name", "i18n": "customKey" }
```

→ translate is called with `customKey.label` and `customKey.description`.

In JSON schema:
```json
{ "name": { "type": "string", "i18n": "myCustomName" } }
```

### `translateError` Function

Called to extract a single string from an AJV error object:

```ts
(error: ErrorObject, translate: Translator, uischema?: UISchemaElement) => string
```

### Accessing i18n in Custom Renderers

```js
import { useJsonForms } from '@jsonforms/react';

const ctx = useJsonForms();
const { locale, translate, translateError } = ctx.i18n;
```

---

## 11. Renderer Sets

### Available Packages

| Package | Framework | Base |
|---|---|---|
| `@jsonforms/material-renderers` | React | Material UI |
| `@jsonforms/vanilla-renderers` | React | Plain HTML + CSS |
| Angular Material | Angular | Angular Material |
| Vue Vanilla | Vue | Plain HTML |
| Vue Vuetify | Vue | Vuetify |

### React Material (most common)

```js
import { materialRenderers, materialCells } from '@jsonforms/material-renderers';

<JsonForms renderers={materialRenderers} cells={materialCells} ... />
```

### React Vanilla

```js
import { vanillaRenderers, vanillaCells } from '@jsonforms/vanilla-renderers';
```

### Supported JSON Schema Features by React Material

| Feature | Renderer |
|---|---|
| `boolean` | Checkbox, Toggle |
| `integer` / `number` | Number input, Text |
| `string` | Text input, Textarea |
| `enum` | Dropdown, Autocomplete |
| `oneOf` (const+title) | Dropdown, Autocomplete |
| `date` format | Date picker |
| `time` format | Time picker |
| `date-time` format | DateTime picker |
| `object` | Vertical grid |
| `array` of primitives | List |
| `array` of objects | Table, List, List with Detail |
| `array` of enums (uniqueItems) | Multiple Choice checkboxes |
| `oneOf` / `allOf` / `anyOf` | Tabs |

### Supported UI Schema Features

| Type | Renderer |
|---|---|
| `VerticalLayout` | Vertical Grid |
| `HorizontalLayout` | Horizontal Grid |
| `Categorization` | Tabs |
| `Group` | Group with label |
| `Label` | Text label |

---

## 12. Custom Renderers

### Overview

Custom renderers override or supplement how specific form fields are rendered. They must:
1. Be a React component.
2. Be connected to JSON Forms via a HOC (e.g. `withJsonFormsControlProps`).
3. Have an associated **tester** that returns a rank > 0.
4. Be registered in the `renderers` array passed to `JsonForms`.

### Step-by-Step

#### 1. Create the Renderer Component

```tsx
import { withJsonFormsControlProps } from '@jsonforms/react';

interface MyControlProps {
  data: any;
  handleChange(path: string, value: any): void;
  path: string;
}

const MyControl = ({ data, handleChange, path }: MyControlProps) => (
  <input
    value={data ?? ''}
    onChange={(e) => handleChange(path, e.target.value)}
  />
);

export default withJsonFormsControlProps(MyControl);
```

#### 2. Create the Tester

```ts
import { rankWith, scopeEndsWith } from '@jsonforms/core';

export const myControlTester = rankWith(
  3,                        // rank higher than default (1 or 2)
  scopeEndsWith('myField')  // matches controls whose scope ends with "myField"
);
```

Common tester helpers from `@jsonforms/core`:
- `scopeEndsWith(suffix)` — matches scope ending with a string
- `schemaTypeIs(type)` — matches a specific JSON schema type
- `isBooleanControl` — built-in predicate for booleans
- `isIntegerControl` — built-in predicate for integers
- `uiTypeIs(type)` — matches a specific UI schema element type
- `and(...)`, `or(...)` — compose predicates
- `schemaMatches(fn)` — matches based on a custom schema predicate
- `rankWith(rank, predicate)` — wraps predicate into a tester

#### 3. Register the Renderer

```tsx
import MyControl, { myControlTester } from './MyControl';

const renderers = [
  ...materialRenderers,
  { tester: myControlTester, renderer: MyControl },
];

<JsonForms renderers={renderers} ... />
```

### HOC Variants

| HOC | Use case |
|---|---|
| `withJsonFormsControlProps` | Standard control (input, field) |
| `withJsonFormsLayoutProps` | Custom layout renderer |
| `withJsonFormsArrayLayoutProps` | Custom array layout |

### Reusing Existing Renderers

```tsx
import { Unwrapped } from '@jsonforms/material-renderers';
import { ControlProps } from '@jsonforms/core';
import { withJsonFormsControlProps } from '@jsonforms/react';

const { MaterialBooleanControl } = Unwrapped;

const MyBooleanControl = (props: ControlProps) => (
  <div>
    <MaterialBooleanControl {...props} label={`${props.label} (custom)`} />
  </div>
);

export default withJsonFormsControlProps(MyBooleanControl);
```

### Dispatching Child Elements

When a custom renderer needs to render nested JSON Forms elements:

```tsx
import { ResolvedJsonFormsDispatch } from '@jsonforms/react';

// Use ResolvedJsonFormsDispatch (preferred — does NOT re-resolve $refs)
// Use JsonFormsDispatch only if you need external $ref resolution

uischema.elements.map((child, index) => (
  <ResolvedJsonFormsDispatch
    key={index}
    schema={schema}
    uischema={child}
    path={path}
    enabled={enabled}
    renderers={renderers}
    cells={cells}
  />
))
```

---

## 13. Custom Layouts

Custom layouts follow the same pattern as custom renderers, using `withJsonFormsLayoutProps`.

### Example: Custom Group as Accordion

#### Renderer

```jsx
import { MaterialLayoutRenderer } from '@jsonforms/material-renderers';
import { withJsonFormsLayoutProps } from '@jsonforms/react';
import { Accordion, AccordionDetails, AccordionSummary, Hidden, Typography } from '@mui/material';
import ExpandMoreIcon from '@mui/icons-material/ExpandMore';

const MyGroupRenderer = ({ uischema, schema, path, visible, renderers }) => {
  const layoutProps = {
    elements: uischema.elements,
    schema,
    path,
    direction: 'column',
    visible,
    uischema,
    renderers,
  };

  return (
    <Hidden xsUp={!visible}>
      <Accordion>
        <AccordionSummary expandIcon={<ExpandMoreIcon />}>
          <Typography>{uischema.label}</Typography>
        </AccordionSummary>
        <AccordionDetails>
          <MaterialLayoutRenderer {...layoutProps} />
        </AccordionDetails>
      </Accordion>
    </Hidden>
  );
};

export default withJsonFormsLayoutProps(MyGroupRenderer);
```

#### Tester

```js
import { rankWith, uiTypeIs } from '@jsonforms/core';

export const myGroupTester = rankWith(1000, uiTypeIs('Group'));
```

#### Registration

```js
const renderers = [
  ...materialRenderers,
  { tester: myGroupTester, renderer: MyGroupRenderer },
];
```

---

## 14. Tutorial: Full App Setup

### Install Dependencies (React + Material UI)

```bash
npm install --save @jsonforms/core
npm install --save @jsonforms/react
npm install --save @jsonforms/material-renderers
npm install --save @mui/material @mui/icons-material @mui/x-date-pickers
npm install --save @emotion/styled @emotion/react
```

### Full Example App

```tsx
import React, { useState } from 'react';
import { JsonForms } from '@jsonforms/react';
import { materialRenderers, materialCells } from '@jsonforms/material-renderers';

const schema = {
  type: 'object',
  required: ['name', 'due_date'],
  properties: {
    name: { type: 'string', minLength: 1 },
    description: { type: 'string' },
    done: { type: 'boolean' },
    due_date: { type: 'string', format: 'date' },
    rating: { type: 'integer', maximum: 5 },
    recurrence: { type: 'string', enum: ['Never', 'Daily', 'Weekly', 'Monthly'] },
    recurrence_interval: { type: 'integer' },
  },
};

const uischema = {
  type: 'VerticalLayout',
  elements: [
    { type: 'Control', label: false, scope: '#/properties/done' },
    { type: 'Control', scope: '#/properties/name' },
    {
      type: 'HorizontalLayout',
      elements: [
        { type: 'Control', scope: '#/properties/due_date' },
        { type: 'Control', scope: '#/properties/rating' },
      ],
    },
    { type: 'Control', scope: '#/properties/description', options: { multi: true } },
    {
      type: 'HorizontalLayout',
      elements: [
        { type: 'Control', scope: '#/properties/recurrence' },
        {
          type: 'Control',
          scope: '#/properties/recurrence_interval',
          rule: {
            effect: 'HIDE',
            condition: {
              scope: '#/properties/recurrence',
              schema: { const: 'Never' },
            },
          },
        },
      ],
    },
  ],
};

const initialData = { name: 'My Task', done: false, recurrence: 'Never' };

export default function App() {
  const [data, setData] = useState(initialData);
  return (
    <JsonForms
      schema={schema}
      uischema={uischema}
      data={data}
      renderers={materialRenderers}
      cells={materialCells}
      onChange={({ data }) => setData(data)}
    />
  );
}
```

---

## Quick Reference Cheatsheet

### UI Schema Element Types

| Type | Category | Description |
|---|---|---|
| `VerticalLayout` | Layout | Stack elements top-to-bottom |
| `HorizontalLayout` | Layout | Place elements side-by-side |
| `Group` | Layout | VerticalLayout + a label |
| `Categorization` | Layout | Tab-based container |
| `Category` | Layout | Tab within Categorization |
| `Control` | Control | Renders a single field |
| `Label` | Control | Displays a static text label |

### Control Scope Syntax

```
#/properties/<propName>
#/properties/<objProp>/properties/<nestedProp>
#/items/properties/<arrayItemProp>
```

### Rule Effect Summary

```json
"rule": {
  "effect": "SHOW" | "HIDE" | "ENABLE" | "DISABLE",
  "condition": {
    "scope": "#/properties/fieldName",
    "schema": { "const": "value" }
  }
}
```

### Validation Modes

```
ValidateAndShow  →  validate + display errors (default)
ValidateAndHide  →  validate + emit errors only (no display)
NoValidation     →  skip validation entirely
```

### Key HOCs

```js
withJsonFormsControlProps  // for field/input renderers
withJsonFormsLayoutProps   // for layout renderers
```

### Tester Helpers

```js
rankWith(rank, predicate)
scopeEndsWith('fieldName')
uiTypeIs('Group')
schemaTypeIs('string')
isBooleanControl
isIntegerControl
and(pred1, pred2)
or(pred1, pred2)
schemaMatches(schema => boolean)
```

---

*Source: https://jsonforms.io/docs — Compiled for AI use. JSON Forms version 3.7.0.*
