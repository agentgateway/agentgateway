import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalRoute } from "../../../config";

/**
 * Manually configured JSON Schema for Route
 * Handcrafted to match LocalRoute type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  required: [],
  additionalProperties: false,
  properties: {
    name: {
      type: "string",
      title: "Name",
      description: "Unique name for this route",
    },
    namespace: {
      type: "string",
      title: "Namespace",
      description: "Kubernetes namespace (optional)",
    },
    ruleName: {
      type: "string",
      title: "Rule Name",
      description: "Optional rule name",
    },
    hostnames: {
      type: "array",
      title: "Hostnames",
      description: "List of hostnames to match (can include wildcards)",
      items: {
        type: "string",
      },
    },
    matches: {
      type: "array",
      title: "Route Matches",
      description: "Conditions for matching incoming requests",
      items: {
        type: "object",
        additionalProperties: true, // Allow additional match properties
        properties: {
          path: {
            type: "object",
            title: "Path Match",
            default: {
              pathPrefix: "/",
            },
            oneOf: [
              {
                title: "Exact Path",
                type: "object",
                properties: {
                  exact: {
                    type: "string",
                    title: "Exact Path",
                  },
                },
                required: ["exact"],
                additionalProperties: false,
              },
              {
                title: "Path Prefix",
                type: "object",
                properties: {
                  pathPrefix: {
                    type: "string",
                    title: "Path Prefix",
                  },
                },
                required: ["pathPrefix"],
                additionalProperties: false,
              },
            ],
          },
          method: {
            type: "string",
            title: "HTTP Method",
            enum: ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "CONNECT", "TRACE"],
          },
        },
        required: ["path"],
      },
    },
    // backends removed - managed via hierarchy tree
  },
};

/**
 * UI Schema for Route
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:placeholder": "e.g., api-route",
    "ui:help": "Optional unique identifier for this route",
  },
  namespace: {
    "ui:placeholder": "default",
  },
  ruleName: {
    "ui:placeholder": "e.g., rule-1",
  },
  hostnames: {
    "ui:options": {
      orderable: false,
      addable: true,
      removable: true,
    },
    "ui:help": "e.g., api.example.com, *.example.com",
  },
  matches: {
    "ui:options": {
      orderable: true,
      addable: true,
      removable: true,
    },
    items: {
      path: {
        "ui:help": "Select how to match the request path",
      },
      method: {
        "ui:widget": "select",
        "ui:placeholder": "Any method",
        "ui:help": "Leave empty to match all HTTP methods",
      },
    },
  },
};

/**
 * Default values for a new route
 */
export const defaultValues: Partial<LocalRoute> = {
  matches: [
    {
      path: {
        pathPrefix: "/",
      },
    },
  ],
};

/**
 * Type guard to validate data matches LocalRoute
 */
export function isLocalRoute(data: unknown): data is LocalRoute {
  return typeof data === "object" && data !== null;
}

/**
 * Transform function to strip UI-only fields before submission
 * The backends and policies fields are managed separately and should not be included in route updates
 */
export function transformBeforeSubmit(data: unknown): unknown {
  if (typeof data !== "object" || data === null) {
    return data;
  }

  const { backends, policies, ...rest } = data as Record<string, unknown> & {
    backends?: unknown;
    policies?: unknown;
  };

  // Don't include backends or policies - they are managed separately
  return rest;
}
