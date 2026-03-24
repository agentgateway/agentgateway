import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Request Header Modifier Policy Form
 * For route.policies.requestHeaderModifier field
 */
export const schema: RJSFSchema = {
  type: "object",
  properties: {
    add: {
      type: "object",
      description: "Headers to add to the request (if not already present)",
      additionalProperties: { type: "string" },
    },
    set: {
      type: "object",
      description: "Headers to set on the request (overwriting if present)",
      additionalProperties: { type: "string" },
    },
    remove: {
      type: "array",
      title: "Remove Headers",
      description: "Header names to remove from the request",
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
    "ui:title": "",
    "ui:help": "Add headers only if they don't already exist",
  },
  set: {
    "ui:title": "",
    "ui:help": "Set headers, overwriting any existing values",
  },
  remove: {
    "ui:help": "Remove these headers from the request",
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
