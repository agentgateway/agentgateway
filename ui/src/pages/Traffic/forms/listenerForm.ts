import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalListener } from "../../../config";

/**
 * Manually configured JSON Schema for Listener
 * This schema is NOT auto-generated - it's handcrafted based on TypeScript types
 * from config.d.ts for full control and customization
 */
export const schema: RJSFSchema = {
  type: "object",
  required: [],
  additionalProperties: false,
  properties: {
    name: {
      type: "string",
      title: "Name",
      description: "Unique name for this listener",
    },
    namespace: {
      type: "string",
      title: "Namespace",
      description: "Kubernetes namespace (optional)",
    },
    hostname: {
      type: "string",
      title: "Hostname",
      description: "Hostname to match (use * for wildcard)",
      default: "*",
    },
    protocol: {
      type: "string",
      title: "Protocol",
      enum: ["HTTP", "HTTPS", "TLS", "TCP", "HBONE"],
      default: "HTTP",
      description: "Protocol for this listener",
    },
    // routes and tcpRoutes removed - managed via hierarchy tree
    policies: {
      type: "object",
      description: "Policies applied at the gateway level for this listener",
      additionalProperties: true,
    },
  },
  dependencies: {
    protocol: {
      oneOf: [
        {
          // HTTP - no TLS, uses routes
          properties: {
            protocol: { const: "HTTP" },
          },
        },
        {
          // HTTPS - requires TLS, uses routes
          properties: {
            protocol: { const: "HTTPS" },
            tls: {
              type: "object",
              title: "TLS Configuration",
              description: "Required for HTTPS protocol",
              properties: {
                cert: {
                  type: "string",
                  title: "Certificate Path",
                  description: "Path to TLS certificate file",
                },
                key: {
                  type: "string",
                  title: "Key Path",
                  description: "Path to TLS private key file",
                },
                root: {
                  type: "string",
                  title: "Root CA Path",
                  description: "Path to root CA certificate (for mutual TLS)",
                },
                cipherSuites: {
                  type: "array",
                  title: "Cipher Suites",
                  description: "Allowed cipher suites (order preserved)",
                  items: {
                    type: "string",
                  },
                },
                minTLSVersion: {
                  type: "string",
                  title: "Min TLS Version",
                  enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
                  default: "TLS_V1_2",
                  description: "Minimum TLS version",
                },
                maxTLSVersion: {
                  type: "string",
                  title: "Max TLS Version",
                  enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
                  default: "TLS_V1_3",
                  description: "Maximum TLS version",
                },
              },
              required: ["cert", "key"],
            },
          },
          required: ["tls"],
        },
        {
          // TLS - requires TLS, uses tcpRoutes
          properties: {
            protocol: { const: "TLS" },
            tls: {
              type: "object",
              title: "TLS Configuration",
              description: "Required for TLS protocol",
              properties: {
                cert: {
                  type: "string",
                  title: "Certificate Path",
                  description: "Path to TLS certificate file",
                },
                key: {
                  type: "string",
                  title: "Key Path",
                  description: "Path to TLS private key file",
                },
                root: {
                  type: "string",
                  title: "Root CA Path",
                  description: "Path to root CA certificate (for mutual TLS)",
                },
                cipherSuites: {
                  type: "array",
                  title: "Cipher Suites",
                  description: "Allowed cipher suites (order preserved)",
                  items: {
                    type: "string",
                  },
                },
                minTLSVersion: {
                  type: "string",
                  title: "Min TLS Version",
                  enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
                  default: "TLS_V1_2",
                  description: "Minimum TLS version",
                },
                maxTLSVersion: {
                  type: "string",
                  title: "Max TLS Version",
                  enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
                  default: "TLS_V1_3",
                  description: "Maximum TLS version",
                },
              },
              required: ["cert", "key"],
            },
          },
          required: ["tls"],
        },
        {
          // TCP - no TLS, uses tcpRoutes
          properties: {
            protocol: { const: "TCP" },
          },
        },
        {
          // HBONE - no TLS, uses routes
          properties: {
            protocol: { const: "HBONE" },
          },
        },
      ],
    },
  },
};

/**
 * UI Schema for Listener
 * Customizes the form rendering
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:placeholder": "e.g., main-listener",
    "ui:help": "Optional unique identifier for this listener",
  },
  namespace: {
    "ui:placeholder": "default",
  },
  hostname: {
    "ui:placeholder": "*",
    "ui:help": "Use * for all hosts, or specify a specific hostname",
  },
  protocol: {
    "ui:widget": "select",
    "ui:help": "HTTP/HTTPS for web traffic, TCP/TLS for raw TCP, HBONE for service mesh",
  },
  tls: {
    cert: {
      "ui:placeholder": "/path/to/cert.pem",
    },
    key: {
      "ui:placeholder": "/path/to/key.pem",
    },
    root: {
      "ui:placeholder": "/path/to/ca.pem",
    },
    cipherSuites: {
      "ui:options": {
        orderable: true,
        addable: true,
        removable: true,
      },
    },
    minTLSVersion: {
      "ui:widget": "select",
    },
    maxTLSVersion: {
      "ui:widget": "select",
    },
  },
  policies: {
    "ui:title": "",
    "ui:help": "Advanced: Gateway-level policies (CORS, headers, etc.). See documentation for details.",
  },
};

/**
 * Default values for a new listener
 */
export const defaultValues: Partial<LocalListener> = {
  hostname: "*",
  protocol: "HTTP",
};

/**
 * Type guard to validate data matches LocalListener
 */
export function isLocalListener(data: unknown): data is LocalListener {
  return typeof data === "object" && data !== null;
}

/**
 * Transform function to strip UI-only fields and protocol-specific fields before submission
 * - HTTP/HTTPS/HBONE protocols use 'routes' and should not have 'tcpRoutes'
 * - TCP/TLS protocols use 'tcpRoutes' and should not have 'routes'
 * - Only HTTPS and TLS protocols should have 'tls' configuration
 */
export function transformBeforeSubmit(data: unknown): unknown {
  if (typeof data !== "object" || data === null) {
    return data;
  }

  const { protocol, routes, tcpRoutes, tls, ...rest } = data as Record<string, unknown> & {
    protocol?: string;
    routes?: unknown;
    tcpRoutes?: unknown;
    tls?: unknown;
  };

  const result: Record<string, unknown> = { ...rest };

  // Add protocol
  if (protocol !== undefined) {
    result.protocol = protocol;
  }

  // Determine which route type to include based on protocol
  if (protocol === "HTTP" || protocol === "HTTPS" || protocol === "HBONE") {
    // HTTP-based protocols use routes
    if (routes !== undefined && routes !== null) {
      result.routes = routes;
    }
    // Don't include tcpRoutes
  } else if (protocol === "TCP" || protocol === "TLS") {
    // TCP-based protocols use tcpRoutes
    if (tcpRoutes !== undefined && tcpRoutes !== null) {
      result.tcpRoutes = tcpRoutes;
    }
    // Don't include routes
  }

  // Only include TLS for HTTPS and TLS protocols
  if ((protocol === "HTTPS" || protocol === "TLS") && tls !== undefined && tls !== null) {
    result.tls = tls;
  }

  return result;
}
