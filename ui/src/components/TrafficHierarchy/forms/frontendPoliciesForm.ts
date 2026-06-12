import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalFrontendPolicies } from "../../../config";

/**
 * Manually configured JSON Schema for Frontend Policies
 * Handcrafted to match LocalFrontendPolicies type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  additionalProperties: true,
  properties: {
    http: {
      type: "object",
      description: "Settings for handling incoming HTTP requests",
      additionalProperties: true,
      properties: {
        maxBufferSize: {
          type: "integer",
          title: "Max Buffer Size (bytes)",
          description: "Maximum buffer size for HTTP requests",
          minimum: 0,
          default: 2097152,
        },
        http1MaxHeaders: {
          type: "integer",
          title: "HTTP/1 Max Headers",
          description: "Maximum number of headers allowed in HTTP/1 requests",
          minimum: 0,
        },
        http1IdleTimeout: {
          type: "string",
          title: "HTTP/1 Idle Timeout",
          description: "Idle timeout for HTTP/1 connections (e.g., 10m0s)",
          default: "10m0s",
        },
        http2WindowSize: {
          type: "integer",
          title: "HTTP/2 Window Size",
          description: "HTTP/2 stream window size",
          minimum: 0,
        },
        http2ConnectionWindowSize: {
          type: "integer",
          title: "HTTP/2 Connection Window Size",
          description: "HTTP/2 connection window size",
          minimum: 0,
        },
        http2FrameSize: {
          type: "integer",
          title: "HTTP/2 Frame Size",
          description: "HTTP/2 frame size",
          minimum: 0,
        },
        http2KeepaliveInterval: {
          type: "string",
          title: "HTTP/2 Keepalive Interval",
          description: "HTTP/2 keepalive ping interval (e.g., 30s)",
        },
        http2KeepaliveTimeout: {
          type: "string",
          title: "HTTP/2 Keepalive Timeout",
          description: "HTTP/2 keepalive ping timeout (e.g., 10s)",
        },
      },
    },
    tls: {
      type: "object",
      description: "Settings for handling incoming TLS connections",
      additionalProperties: true,
      properties: {
        handshakeTimeout: {
          type: "string",
          title: "Handshake Timeout",
          description: "TLS handshake timeout (e.g., 15s)",
          default: "15s",
        },
        alpn: {
          type: "array",
          title: "ALPN Protocols",
          description: "Application-Layer Protocol Negotiation protocols (advanced)",
          items: {
            type: "array",
            items: {
              type: "integer",
            },
          },
        },
        minVersion: {
          type: "string",
          title: "Minimum TLS Version",
          enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
        },
        maxVersion: {
          type: "string",
          title: "Maximum TLS Version",
          enum: ["TLS_V1_0", "TLS_V1_1", "TLS_V1_2", "TLS_V1_3"],
        },
        cipherSuites: {
          type: "array",
          title: "Cipher Suites",
          description: "Allowed cipher suites (order matters)",
          items: {
            type: "string",
          },
        },
      },
    },
    tcp: {
      type: "object",
      description: "Settings for handling incoming TCP connections",
      additionalProperties: true,
    },
    accessLog: {
      type: "object",
      description: "Settings for request access logs",
      additionalProperties: true,
      properties: {
        filter: {
          type: "string",
          title: "Filter Expression",
          description: "CEL expression to filter log entries",
        },
        add: {
          type: "object",
          title: "Add Fields",
          description: "Additional fields to add to log entries",
          additionalProperties: {
            type: "string",
          },
        },
        remove: {
          type: "array",
          title: "Remove Fields",
          description: "Fields to remove from log entries",
          items: {
            type: "string",
          },
        },
      },
    },
    tracing: {
      type: "object",
      description: "OpenTelemetry distributed tracing settings",
      additionalProperties: true,
      properties: {
        service: {
          type: "object",
          title: "Service Backend",
          description: "Service to send traces to",
          properties: {
            name: {
              type: "string",
              title: "Service Name",
              description: "Name of the tracing service (e.g., jaeger or namespace/jaeger)",
            },
            port: {
              type: "integer",
              title: "Port",
              description: "Service port",
              minimum: 1,
              maximum: 65535,
            },
          },
        },
        backend: {
          type: "string",
          title: "Backend Reference",
          description: "Reference to a backend defined elsewhere",
        },
        path: {
          type: "string",
          title: "Path",
          description: "Path for tracing endpoint",
          default: "/v1/traces",
        },
        protocol: {
          type: "string",
          title: "Protocol",
          enum: ["grpc", "http"],
          default: "grpc",
          description: "Protocol to use for sending traces",
        },
        attributes: {
          type: "object",
          title: "Attributes",
          description: "CEL expressions to add custom trace attributes",
          additionalProperties: {
            type: "string",
          },
        },
        resources: {
          type: "object",
          title: "Resources",
          description: "CEL expressions to add resource attributes",
          additionalProperties: {
            type: "string",
          },
        },
        remove: {
          type: "array",
          title: "Remove Attributes",
          description: "List of attribute keys to remove from traces",
          items: {
            type: "string",
          },
        },
        randomSampling: {
          type: "string",
          title: "Random Sampling",
          description: "CEL expression for random sampling rate (0.0-1.0)",
        },
        clientSampling: {
          type: "string",
          title: "Client Sampling",
          description: "CEL expression to honor client sampling decisions",
        },
        policies: {
          type: "object",
          title: "Backend Policies",
          description: "Policies for the tracing backend connection",
          additionalProperties: true,
        },
      },
    },
  },
};

/**
 * UI Schema for Frontend Policies
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  http: {
    "ui:title": "",
    maxBufferSize: {
      "ui:placeholder": "2097152",
      "ui:help": "Default: 2097152 bytes (2MB)",
    },
    http1MaxHeaders: {
      "ui:placeholder": "100",
      "ui:help": "Maximum number of HTTP/1 headers",
    },
    http1IdleTimeout: {
      "ui:placeholder": "10m0s",
      "ui:help": "Duration format: 10s, 5m, 1h30m",
    },
    http2WindowSize: {
      "ui:help": "HTTP/2 stream window size in bytes",
    },
    http2ConnectionWindowSize: {
      "ui:help": "HTTP/2 connection window size in bytes",
    },
    http2FrameSize: {
      "ui:help": "HTTP/2 frame size in bytes",
    },
    http2KeepaliveInterval: {
      "ui:placeholder": "30s",
      "ui:help": "How often to send keepalive pings",
    },
    http2KeepaliveTimeout: {
      "ui:placeholder": "10s",
      "ui:help": "Timeout for keepalive ping responses",
    },
  },
  tls: {
    "ui:title": "",
    handshakeTimeout: {
      "ui:placeholder": "15s",
      "ui:help": "Duration format: 10s, 5m, 1h",
    },
    alpn: {
      "ui:help": "Advanced: ALPN protocol bytes (typically leave empty)",
    },
    minVersion: {
      "ui:widget": "select",
      "ui:help": "Minimum TLS version to accept",
    },
    maxVersion: {
      "ui:widget": "select",
      "ui:help": "Maximum TLS version to accept",
    },
    cipherSuites: {
      "ui:options": {
        orderable: true,
        addable: true,
        removable: true,
      },
      "ui:help": "List of allowed cipher suites (order matters)",
    },
  },
  accessLog: {
    "ui:title": "",
    filter: {
      "ui:placeholder": "request.path.startsWith('/api')",
      "ui:help": "CEL expression to filter which requests are logged",
    },
  },
  tcp: {
    "ui:title": "",
  },
  tracing: {
    "ui:title": "",
    service: {
      name: {
        "ui:placeholder": "jaeger or default/jaeger",
        "ui:help": "Service name, optionally namespaced",
      },
      port: {
        "ui:placeholder": "4317",
        "ui:help": "gRPC port is typically 4317, HTTP is 4318",
      },
    },
    backend: {
      "ui:placeholder": "my-tracing-backend",
      "ui:help": "Reference to a backend defined in the backends section",
    },
    path: {
      "ui:placeholder": "/v1/traces",
      "ui:help": "OTLP traces endpoint path",
    },
    protocol: {
      "ui:widget": "select",
      "ui:help": "gRPC (default) or HTTP for OTLP",
    },
    attributes: {
      "ui:help": "CEL expressions: {\"custom.field\": \"request.headers['x-custom']\"}",
    },
    resources: {
      "ui:help": "CEL expressions for resource attributes: {\"service.version\": \"'1.0.0'\"}",
    },
    remove: {
      "ui:help": "List of attribute keys to remove from traces",
    },
    randomSampling: {
      "ui:placeholder": "0.1",
      "ui:help": "CEL expression for sampling rate (e.g., '0.1' for 10%)",
    },
    clientSampling: {
      "ui:help": "CEL expression to honor client sampling decisions",
    },
  },
};

/**
 * Default values for frontend policies
 */
export const defaultValues: Partial<LocalFrontendPolicies> = {
  accessLog: {},
};

/**
 * Type guard to validate data matches LocalFrontendPolicies
 */
export function isLocalFrontendPolicies(
  data: unknown,
): data is LocalFrontendPolicies {
  return typeof data === "object" && data !== null;
}

/**
 * Transform function - no transformation needed
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
