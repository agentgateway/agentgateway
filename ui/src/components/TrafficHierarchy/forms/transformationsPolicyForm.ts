import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Transformations policy form — used for transformations policies on
 * LLM, MCP, MCP targets, and routes.
 *
 * Maps to LocalTransformationConfig:
 *   request?: LocalTransform
 *   response?: LocalTransform
 *
 * Each LocalTransform has:
 *   add?: Record<string, string>
 *   set?: Record<string, string>
 *   remove?: string[]
 *   body?: string
 *   metadata?: Record<string, string>
 */

const localTransformSchema: RJSFSchema = {
  type: "object",
  properties: {
    add: {
      type: "object",
      title: "Add Headers",
      description: "Headers to add (if not already present)",
      additionalProperties: { type: "string" },
    },
    set: {
      type: "object",
      title: "Set Headers",
      description: "Headers to set (overwriting if present)",
      additionalProperties: { type: "string" },
    },
    remove: {
      type: "array",
      title: "Remove Headers",
      description: "Header names to remove",
      items: { type: "string" },
    },
    body: {
      type: "string",
      title: "Body",
      description: "CEL expression to transform the body",
    },
    metadata: {
      type: "object",
      title: "Metadata",
      description: "Metadata key-value pairs",
      additionalProperties: { type: "string" },
    },
  },
};

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    request: {
      title: "Request Transformations",
      ...localTransformSchema,
    },
    response: {
      title: "Response Transformations",
      ...localTransformSchema,
    },
  },
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  request: {
    "ui:label": false,
    add: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "header-name", "ui:valuePlaceholder": "header-value", "ui:label": false, },
    set: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "header-name", "ui:valuePlaceholder": "header-value", "ui:label": false, },
    metadata: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "key", "ui:valuePlaceholder": "value", "ui:label": false, },
    body: {
      "ui:widget": "textarea",
      "ui:placeholder": "e.g., request.body + '{\"extra\": true}'",
      "ui:options": { rows: 3 },
    },
    remove: { "ui:help": "Header names to remove from the request" },
  },
  response: {
    "ui:title": false,
    add: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "header-name", "ui:valuePlaceholder": "header-value", "ui:label": false, },
    set: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "header-name", "ui:valuePlaceholder": "header-value", "ui:label": false, },
    metadata: { "ui:field": "keyValueMap", "ui:keyPlaceholder": "key", "ui:valuePlaceholder": "value", "ui:label": false, },
    body: {
      "ui:widget": "textarea",
      "ui:placeholder": "e.g., response.body",
      "ui:options": { rows: 3 },
    },
    remove: { "ui:help": "Header names to remove from the response" },
  },
};

export const defaultValues = {};

export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
