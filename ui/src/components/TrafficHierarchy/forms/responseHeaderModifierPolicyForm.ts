import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Response Header Modifier Policy Form
 * For route.policies.responseHeaderModifier field
 */
export const schema: RJSFSchema = {
  type: "object",
  properties: {
    add: {
      type: "object",
      description: "Headers to add to the response (if not already present)",
      additionalProperties: { type: "string" },
    },
    set: {
      type: "object",
      description: "Headers to set on the response (overwriting if present)",
      additionalProperties: { type: "string" },
    },
    remove: {
      type: "array",
      title: "Remove Headers",
      description: "Header names to remove from the response",
      items: { type: "string" },
    },
  },
};

/**
 * UI Schema
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  add: {
    "ui:field": "keyValueMap",
    "ui:keyPlaceholder": "header-name",
    "ui:valuePlaceholder": "header-value",
  },
  set: {
    "ui:field": "keyValueMap",
    "ui:keyPlaceholder": "header-name",
    "ui:valuePlaceholder": "header-value",
  },
  remove: {
    "ui:help": "Remove these headers from the response",
  },
};

/**
 * Default values
 */
export const defaultValues = {};

/**
 * Transform function
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
