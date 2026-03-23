import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Route Policy Form
 * This is for inline route.policies field (not top-level LocalPolicy)
 * It's just the policy configuration without name/target/phase wrapping
 */
export const schema: RJSFSchema = {
  type: "object",
  additionalProperties: true,
  properties: {
    cors: {
      type: "object",
      properties: {
        allowCredentials: {
          type: "boolean",
          title: "Allow Credentials",
        },
        allowHeaders: {
          type: "array",
          title: "Allow Headers",
          items: { type: "string" },
        },
        allowMethods: {
          type: "array",
          title: "Allow Methods",
          items: { type: "string" },
        },
        allowOrigins: {
          type: "array",
          title: "Allow Origins",
          items: { type: "string" },
        },
        exposeHeaders: {
          type: "array",
          title: "Expose Headers",
          items: { type: "string" },
        },
        maxAge: {
          type: "string",
          title: "Max Age",
          description: "Duration (e.g., 24h)",
        },
      },
    },
    requestHeaderModifier: {
      type: "object",
      properties: {
        add: {
          type: "object",
          title: "Add Headers",
          additionalProperties: { type: "string" },
        },
        set: {
          type: "object",
          title: "Set Headers",
          additionalProperties: { type: "string" },
        },
        remove: {
          type: "array",
          title: "Remove Headers",
          items: { type: "string" },
        },
      },
    },
    responseHeaderModifier: {
      type: "object",
      properties: {
        add: {
          type: "object",
          title: "Add Headers",
          additionalProperties: { type: "string" },
        },
        set: {
          type: "object",
          title: "Set Headers",
          additionalProperties: { type: "string" },
        },
        remove: {
          type: "array",
          title: "Remove Headers",
          items: { type: "string" },
        },
      },
    },
  },
};

/**
 * UI Schema
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  cors: {
    "ui:title": "",
    allowOrigins: {
      "ui:help": "e.g., https://example.com or * for all origins",
    },
    allowMethods: {
      "ui:help": "e.g., GET, POST, PUT, DELETE",
    },
    allowHeaders: {
      "ui:help": "e.g., Content-Type, Authorization",
    },
    maxAge: {
      "ui:placeholder": "24h",
      "ui:help": "How long browsers can cache preflight results",
    },
  },
};

/**
 * Default values
 */
export const defaultValues = {};

/**
 * Transform function - route policies don't need transformation
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
