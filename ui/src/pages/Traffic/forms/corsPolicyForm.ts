import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * CORS Policy Form
 * For route.policies.cors field
 */
export const schema: RJSFSchema = {
  type: "object",
  properties: {
    allowCredentials: {
      type: "boolean",
      title: "Allow Credentials",
      description: "Whether to allow credentials in CORS requests",
    },
    allowHeaders: {
      type: "array",
      title: "Allow Headers",
      description: "HTTP headers that can be used during the actual request",
      items: { type: "string" },
    },
    allowMethods: {
      type: "array",
      title: "Allow Methods",
      description: "HTTP methods allowed when accessing the resource",
      items: { type: "string" },
    },
    allowOrigins: {
      type: "array",
      title: "Allow Origins",
      description: "Origins that are allowed to access the resource",
      items: { type: "string" },
    },
    exposeHeaders: {
      type: "array",
      title: "Expose Headers",
      description: "Headers that browsers are allowed to access",
      items: { type: "string" },
    },
    maxAge: {
      type: "string",
      title: "Max Age",
      description: "How long the results of a preflight request can be cached (e.g., 24h)",
    },
  },
};

/**
 * UI Schema
 */
export const uiSchema: UiSchema = {
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
  exposeHeaders: {
    "ui:help": "Headers that client-side JavaScript can access",
  },
  maxAge: {
    "ui:placeholder": "24h",
    "ui:help": "How long browsers can cache preflight results",
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
