import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalPolicy } from "../../../config";

/**
 * Manually configured JSON Schema for Policy
 * Handcrafted to match LocalPolicy type from config.d.ts
 *
 * Note: This form covers the structure of a LocalPolicy. The `policy` field
 * (FilterOrPolicy) is very complex with many optional fields, so we allow
 * additionalProperties to capture all policy configurations.
 */
export const schema: RJSFSchema = {
  type: "object",
  required: ["name", "target", "policy"],
  additionalProperties: true,
  properties: {
    name: {
      type: "object",
      required: ["name", "namespace"],
      properties: {
        name: {
          type: "string",
          title: "Name",
          description: "Policy name",
        },
        namespace: {
          type: "string",
          title: "Namespace",
          description: "Policy namespace",
          default: "default",
        },
      },
    },
    phase: {
      type: "string",
      title: "Phase",
      enum: ["route", "gateway"],
      default: "route",
      description: "Gateway policies run pre-routing, route policies run post-routing",
    },
    target: {
      type: "object",
      description: "What this policy applies to",
      oneOf: [
        {
          title: "Gateway Target",
          type: "object",
          required: ["gateway"],
          properties: {
            gateway: {
              type: "object",
              title: "Gateway",
              required: ["gatewayName", "gatewayNamespace"],
              properties: {
                gatewayName: {
                  type: "string",
                  title: "Gateway Name",
                },
                gatewayNamespace: {
                  type: "string",
                  title: "Gateway Namespace",
                  default: "default",
                },
                listenerName: {
                  type: "string",
                  title: "Listener Name",
                  description: "Optional specific listener",
                },
              },
            },
          },
        },
        {
          title: "Route Target",
          type: "object",
          required: ["route"],
          properties: {
            route: {
              type: "object",
              title: "Route",
              required: ["name", "namespace"],
              properties: {
                name: {
                  type: "string",
                  title: "Route Name",
                },
                namespace: {
                  type: "string",
                  title: "Route Namespace",
                  default: "default",
                },
                ruleName: {
                  type: "string",
                  title: "Rule Name",
                  description: "Optional specific rule",
                },
                kind: {
                  type: "string",
                  title: "Kind",
                  description: "Route kind (e.g., HTTPRoute, TCPRoute)",
                },
              },
            },
          },
        },
        {
          title: "Backend Target",
          type: "object",
          required: ["backend"],
          properties: {
            backend: {
              type: "object",
              title: "Backend",
              required: ["name", "namespace"],
              properties: {
                name: {
                  type: "string",
                  title: "Backend Name",
                },
                namespace: {
                  type: "string",
                  title: "Backend Namespace",
                  default: "default",
                },
              },
            },
          },
        },
      ],
    },
    policy: {
      type: "object",
      description: "Policy settings (e.g., CORS, rate limiting, authentication, etc.)",
      additionalProperties: true,
      properties: {
        cors: {
          type: "object",
          title: "CORS",
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
            },
          },
        },
        requestHeaderModifier: {
          type: "object",
          title: "Request Header Modifier",
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
          title: "Response Header Modifier",
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
    },
  },
};

/**
 * UI Schema for Policy
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:title": "",
    name: {
      "ui:placeholder": "e.g., my-policy",
    },
    namespace: {
      "ui:placeholder": "default",
    },
  },
  phase: {
    "ui:widget": "select",
    "ui:help": "Route policies apply after routing decisions, gateway policies before",
  },
  target: {
    "ui:title": "",
    "ui:help": "Select what this policy should apply to",
  },
  policy: {
    "ui:title": "",
    "ui:help": "Configure the policy settings (CORS, headers, rate limiting, etc.)",
  },
};

/**
 * Default values for a new policy
 */
export const defaultValues: Partial<LocalPolicy> = {
  name: {
    name: "",
    namespace: "default",
  },
  phase: "route",
  target: {
    route: {
      name: "",
      namespace: "default",
    },
  },
  policy: {},
};

/**
 * Type guard to validate data matches LocalPolicy
 */
export function isLocalPolicy(data: unknown): data is LocalPolicy {
  return (
    typeof data === "object" &&
    data !== null &&
    "name" in data &&
    "target" in data &&
    "policy" in data
  );
}

/**
 * Transform function - policies don't need transformation
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
