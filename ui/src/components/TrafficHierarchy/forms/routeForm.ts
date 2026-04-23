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
            additionalProperties: true,
            required: ["pathType"],
            properties: {
              pathType: {
                type: "string",
                title: "Match Type",
                enum: ["pathPrefix", "exact", "regex"],
                default: "pathPrefix",
              },
            },
            dependencies: {
              pathType: {
                oneOf: [
                  {
                    properties: {
                      pathType: { const: "pathPrefix" },
                      pathPrefix: {
                        type: "string",
                        title: "Path Prefix",
                        default: "/",
                      },
                    },
                    required: ["pathPrefix"],
                  },
                  {
                    properties: {
                      pathType: { const: "exact" },
                      exact: {
                        type: "string",
                        title: "Exact Path",
                      },
                    },
                    required: ["exact"],
                  },
                  {
                    properties: {
                      pathType: { const: "regex" },
                      regex: {
                        type: "string",
                        title: "Regex",
                      },
                    },
                    required: ["regex"],
                  },
                ],
              },
            },
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
        "ui:label": false,
        "ui:help": "Select how to match the request path",
        pathType: {
          "ui:widget": "select",
        },
        pathPrefix: {
          "ui:placeholder": "/",
        },
        exact: {
          "ui:placeholder": "/exact/path",
        },  
        regex: { 
          "ui:placeholder": "^/api/.*$",
        }
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
  const { backends: _backends, policies: _policies, ...rest } = data as Record<
    string,
    unknown
  > & {
    backends?: unknown;
    policies?: unknown;
  };
  const routeData: Record<string, unknown> = { ...rest };
  if (Array.isArray(routeData.matches)) {
    routeData.matches = routeData.matches.map((match) => {
      if (!match || typeof match !== "object") return match;
      const m = match as Record<string, unknown>;
      if (!m.path || typeof m.path !== "object") return match;
      const p = m.path as Record<string, unknown>;
      const pathType = p.pathType;
      const { pathType: _pathType, ...pathWithoutType } = p;
      // Convert UI discriminator shape back to API shape.
      let nextPath: Record<string, unknown> = pathWithoutType;
      if (pathType === "exact") {
        nextPath = { exact: p.exact };
      } else if (pathType === "pathPrefix") {
        nextPath = { pathPrefix: p.pathPrefix };
      } else if (pathType === "regex") {
        nextPath = { regex: p.regex };
      }
      return { ...m, path: nextPath };
    });
  }
  return routeData;
}
