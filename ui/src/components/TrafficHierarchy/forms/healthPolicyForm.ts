import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
      unhealthyExpression: {
        type: "string",
        title: "Unhealthy Expression",
        description: "CEL expression; true means unhealthy. E.g. response.code >= 500. Leave blank to use default (any 5xx or connection failure).",
      },
      eviction: {
        type: "object",
        title: "Eviction",
        properties: {
          duration: {
            type: "string",
            title: "Duration",
            description: "How long to evict the backend. Must include a unit (e.g. 30s, 1m)",
          },
          consecutiveFailures: {
            type: "number",
            title: "Consecutive Failures",
            description: "Number of consecutive failures before eviction",
          },
          restoreHealth: {
            type: "number",
            title: "Restore Health",
            description: "Number of successful requests to restore the backend",
          },
          healthThreshold: {
            type: "number",
            title: "Health Threshold",
            description: "Percentage of healthy requests to consider the backend healthy (0-100)",
          },
        },
      },
    },
}

export const uiSchema: UiSchema = { 
    "ui:title": "",
    unhealthyExpression: {
      "ui:widget": "textarea",
      "ui:options": { rows: 2 },
      "ui:placeholder": "response.code >= 500",
    },
    eviction: { 
        "ui:label": false,
        duration: { "ui:placeholder": "30s" },
    }
}

export const defaultValues = {};

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}