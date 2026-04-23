import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalRouteBackend } from "../../../config";

/**
 * Manually configured JSON Schema for Backend
 * Handcrafted to match LocalRouteBackend type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  additionalProperties: false,
  required: ["backendType"],
  properties: {
    backendType: {
      type: "string",
      title: "Backend Type",
      enum: ["service", "host", "dynamic", "mcp", "ai"],
      default: "service",
      description: "Type of backend to configure",
    },
    weight: {
      type: "integer",
      title: "Weight",
      minimum: 0,
      default: 1,
      description: "Load balancing weight (default: 1)",
    },
  },
  dependencies: {
    backendType: {
      oneOf: [
        {
          properties: {
            backendType: { const: "service" },
            service: {
              type: "object",
              title: "Service Configuration",
              required: ["name", "port"],
              properties: {
                name: {
                  type: "object",
                  title: "Service Name",
                  description: "Namespaced service hostname",
                  required: ["namespace", "hostname"],
                  properties: {
                    namespace: {
                      type: "string",
                      title: "Namespace",
                      description: "Service namespace",
                    },
                    hostname: {
                      type: "string",
                      title: "Hostname",
                      description: "Service hostname",
                    },
                  },
                },
                port: {
                  type: "integer",
                  title: "Port",
                  minimum: 1,
                  maximum: 65535,
                },
              },
            },
          },
          required: ["service"],
        },
        {
          properties: {
            backendType: { const: "host" },
            host: {
              type: "string",
              title: "Host",
              description: "Hostname or IP address with optional port (host:port)",
            },
            tls: {
              type: "object",
              title: "TLS Settings",
              properties: {
                mode: {
                  type: "string",
                  title: "TLS Mode",
                  enum: ["DISABLED", "SIMPLE", "MUTUAL"],
                  default: "DISABLED",
                },
              },
              required: ["mode"],
              dependencies: {
                mode: {
                  oneOf: [
                    {
                      properties: {
                        mode: { enum: ["DISABLED"] },
                      },
                    },
                    {
                      properties: {
                        mode: { enum: ["SIMPLE"] },
                        caCertificates: {
                          type: "string",
                          title: "CA Certificates Path",
                          description: "Path to CA certificates for server verification",
                        },
                        sni: {
                          type: "string",
                          title: "SNI Hostname",
                          description: "Server Name Indication hostname",
                        },
                      },
                    },
                    {
                      properties: {
                        mode: { enum: ["MUTUAL"] },
                        caCertificates: {
                          type: "string",
                          title: "CA Certificates Path",
                          description: "Path to CA certificates for server verification",
                        },
                        clientCertificate: {
                          type: "string",
                          title: "Client Certificate Path",
                          description: "Path to client certificate for mutual TLS",
                        },
                        privateKey: {
                          type: "string",
                          title: "Private Key Path",
                          description: "Path to private key for mutual TLS",
                        },
                        sni: {
                          type: "string",
                          title: "SNI Hostname",
                          description: "Server Name Indication hostname",
                        },
                      },
                      required: ["clientCertificate", "privateKey"],
                    },
                  ],
                },
              },
            },
          },
          required: ["host"],
        },
        {
          properties: {
            backendType: { const: "dynamic" },
            dynamic: {
              type: "object",
              title: "Dynamic Routing Configuration",
              description: "Backend determined dynamically at runtime",
              additionalProperties: false,
            },
          },
          required: ["dynamic"],
        },
        {
          properties: {
            backendType: { const: "mcp" },
            mcp: {
              type: "object",
              title: "MCP Configuration",
              description: "Model Context Protocol backend settings",
              additionalProperties: true,
              properties: {
                targets: {
                  type: "array",
                  title: "MCP Targets",
                  description: "List of MCP server targets",
                  items: {
                    type: "object",
                    required: ["name", "connectionType"],
                    properties: {
                      name: {
                        type: "string",
                        title: "Target Name",
                        description: "Unique name for this MCP target",
                      },
                      connectionType: {
                        type: "string",
                        title: "Connection Type",
                        enum: ["sse", "mcp", "stdio", "openapi"],
                        default: "sse",
                        description: "Type of connection to the MCP server",
                      },
                    },
                    dependencies: {
                      connectionType: {
                        oneOf: [
                          {
                            type: "object",
                            additionalProperties: false,
                            properties: { 
                              name: { 
                                type: "string",
                                title: "Target Name",
                                description: "Unique name for this MCP target",
                              },
                              connectionType: { const: "sse" },
                              sse: { 
                                type: "object",
                                title: "SSE Connection Settings",
                                required: ["host"],
                                properties: { 
                                  host: { type: "string", title: "Host" },
                                  port: { type: "integer", title: "Port", minimum: 1, maximum: 65535 },
                                  path: { type: "string", title: "Path", default: "/sse" },
                                }
                              }
                            },
                            required: ["name", "connectionType", "sse"],
                          },
                          {
                            type: "object",
                            additionalProperties: false,
                            properties: {
                              name: {
                                type: "string",
                                title: "Target Name",
                                description: "Unique name for this MCP target",
                              },
                              connectionType: { const: "mcp" },
                              mcp: {
                                type: "object",
                                title: "MCP Connection Settings",
                                required: ["host"],
                                properties: {
                                  host: { type: "string", title: "Host" },
                                  port: { type: "integer", title: "Port", minimum: 1, maximum: 65535 },
                                  path: { type: "string", title: "Path" },
                                },
                              },
                            },
                            required: ["name", "connectionType", "mcp"],
                          },
                          {
                            type: "object",
                            additionalProperties: false,
                            properties: {
                              name: {
                                type: "string",
                                title: "Target Name",
                                description: "Unique name for this MCP target",
                              },
                              connectionType: { const: "stdio" },
                              stdio: {
                                type: "object",
                                title: "STDIO Connection Settings",
                                required: ["cmd"],
                                properties: {
                                  cmd: { type: "string", title: "Command" },
                                  args: {
                                    type: "array",
                                    title: "Arguments",
                                    items: { type: "string" },
                                  },
                                  env: {
                                    type: "object",
                                    title: "Environment Variables",
                                    additionalProperties: { type: "string" },
                                  },
                                },
                              },
                            },
                            required: ["name", "connectionType", "stdio"],
                          },
                          {
                            type: "object",
                            additionalProperties: false,
                            properties: {
                              name: {
                                type: "string",
                                title: "Target Name",
                                description: "Unique name for this MCP target",
                              },
                              connectionType: { const: "openapi" },
                              openapi: {
                                type: "object",
                                title: "OpenAPI Connection Settings",
                                required: ["host", "schema"],
                                properties: {
                                  host: { type: "string", title: "Host" },
                                  port: { type: "integer", title: "Port", minimum: 1, maximum: 65535 },
                                  path: { type: "string", title: "Path" },
                                  schema: { type: "string", title: "OpenAPI Schema" },
                                },
                              },
                            },
                            required: ["name", "connectionType", "openapi"],
                          },
                        ],
                      },
                    },
                  },
                },
                statefulMode: {
                  type: "string",
                  title: "Stateful Mode",
                  enum: ["stateless", "stateful"],
                  default: "stateless",
                },
                prefixMode: {
                  type: "string",
                  title: "Prefix Mode",
                  enum: ["always", "conditional"],
                },
              },
            },
          },
          required: ["mcp"],
        },
        {
          properties: {
            backendType: { const: "ai" },
            ai: {
              type: "object",
              title: "AI Provider Configuration",
              description: "AI/LLM backend settings",
              additionalProperties: true,
              properties: {
                name: {
                  type: "string",
                  title: "Provider Name",
                  description: "Name of the AI provider",
                },
                provider: {
                  type: "string",
                  title: "Provider Type",
                  enum: ["openAI", "gemini", "vertex", "anthropic", "bedrock", "azureOpenAI"],
                  description: "AI provider to use",
                  default: "openAI",
                },
                hostOverride: {
                  type: "string",
                  title: "Host Override",
                  description: "Optional host override for the provider",
                },
                pathOverride: {
                  type: "string",
                  title: "Path Override",
                  description: "Optional path override for the provider",
                },
                tokenize: {
                  type: "boolean",
                  title: "Tokenize",
                  default: false,
                  description: "Enable tokenization",
                },
              },
            },
          },
          required: ["ai"],
        },
      ],
    },
  },
};

/**
 * UI Schema for Backend
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  backendType: {
    "ui:widget": "select",
    "ui:help": "Service: Kubernetes service | Host: Direct hostname/IP | Dynamic: Runtime-determined | MCP: Model Context Protocol | AI: LLM provider",
  },
  weight: {
    "ui:widget": "updown",
    "ui:help": "Higher weights receive more traffic. Default is 1.",
  },
  service: {
    "ui:title": "",
    name: {
      "ui:help": "Namespaced service identifier",
      "ui:title": "",
      namespace: {
        "ui:placeholder": "default",
        "ui:help": "Kubernetes namespace",
      },
      hostname: {
        "ui:placeholder": "my-service",
        "ui:help": "Service hostname (e.g., my-service or my-service.svc.cluster.local)",
      },
    },
    port: {
      "ui:placeholder": "8080",
    },
  },
  dynamic: { 
    "ui:title": "",
  },
  host: {
    "ui:placeholder": "backend.example.com:8080 or 192.168.1.100:8080",
    "ui:help": "Hostname or IP address with optional port",
  },
  mcp: {
    "ui:title": "",
    prefixMode: {
      "ui:widget": "select", // <- fix typo
      "ui:placeholder": "none",
    },
    targets: {
      items: {
        name: { "ui:placeholder": "e.g., my-mcp-server" },
        connectionType: {
          "ui:widget": "select",
          "ui:help": "SSE and MCP are network-based, STDIO runs a local process",
        },
        sse: {
          "ui:title": "",
          host: { "ui:placeholder": "localhost" },
          port: { "ui:placeholder": "8080" },
          path: { "ui:placeholder": "/sse" },
        },
        mcp: {
          "ui:title": "",
          host: { "ui:placeholder": "localhost" },
          port: { "ui:placeholder": "8080" },
        },
        stdio: {
          "ui:title": "",
          cmd: { "ui:placeholder": "/usr/local/bin/my-mcp-server" },
          env: {
            "ui:field": "keyValueMap",
            "ui:keyPlaceholder": "ENV_VAR",
            "ui:valuePlaceholder": "value",
          },
        },
        openapi: {
          "ui:title": "",
          host: { "ui:placeholder": "api.example.com" },
          schema: { "ui:placeholder": "/path/to/openapi.json" },
        },
      },
    },
  },
  ai: { 
    "ui:title": "",
  },
  tls: {
    "ui:title": "",
    mode: {
      "ui:widget": "select",
    },
    caCertificates: {
      "ui:placeholder": "/path/to/ca.pem",
    },
    clientCertificate: {
      "ui:placeholder": "/path/to/client-cert.pem",
    },
    privateKey: {
      "ui:placeholder": "/path/to/client-key.pem",
    },
    sni: {
      "ui:placeholder": "backend.example.com",
    },
  },
};

export function getDefaultBackendValue(backendType: string): Record<string, unknown> {
  switch (backendType) {
    case "service":
      return {
        backendType: "service",
        service: {
          name: { namespace: "default", hostname: "service" },
          port: 8080,
        },
        weight: 1,
      };
    case "host":
      return { backendType: "host", host: "example.com:8080", weight: 1 };
    case "dynamic":
      return { backendType: "dynamic", dynamic: {}, weight: 1 };
    case "mcp":
      return {
        backendType: "mcp",
        mcp: { targets: [], statefulMode: "stateless" },
        weight: 1,
      };
    case "ai":
      return { backendType: "ai", ai: { name: "default", provider: { openAI: {} } }, weight: 1 };
    default:
      return { backendType: "service", ...defaultValues };
  }
}

/**
 * Default values for a new backend
 * Must match one of the oneOf options (Service Backend in this case)
 */
export const defaultValues: Partial<LocalRouteBackend> = {
  backendType: "service",
  service: {
    name: {
      namespace: "default",
      hostname: "service",
    },
    port: 8080,
  },
  weight: 1,
};

/**
 * Type guard to validate data matches LocalRouteBackend
 */
export function isLocalRouteBackend(data: unknown): data is LocalRouteBackend {
  return typeof data === "object" && data !== null;
}

/**
 * Transform function to add UI-only backendType field when loading data
 */
export function transformForForm(data: unknown): unknown {
  if (typeof data !== "object" || data === null) {
    return data;
  }

  const backendData = data as Record<string, unknown>;
  const result: Record<string, unknown> = { ...backendData };

  // Determine backendType based on which field is present
  if ("service" in backendData) {
    result.backendType = "service";
  } else if ("host" in backendData) {
    result.backendType = "host";
  } else if ("dynamic" in backendData) {
    result.backendType = "dynamic";
  } else if ("mcp" in backendData) {
    result.backendType = "mcp";

    const mcpObj = backendData.mcp as Record<string, unknown> | undefined;
    const targets = Array.isArray(mcpObj?.targets) ? mcpObj?.targets : [];
    if (mcpObj && Array.isArray(targets)) {
      result.mcp = {
        ...mcpObj,
        targets: targets.map((t) => {
          if (!t || typeof t !== "object") return t;
          const target = { ...(t as Record<string, unknown>) };
          if ("sse" in target) target.connectionType = "sse";
          else if ("mcp" in target) target.connectionType = "mcp";
          else if ("stdio" in target) target.connectionType = "stdio";
          else if ("openapi" in target) target.connectionType = "openapi";
          else target.connectionType = "sse";
          return target;
        }),
      };
    }
  } else if ("ai" in backendData) {
    result.backendType = "ai";
  }

  const backendTls = (backendData as any)?.policies?.backendTLS;
  if (backendTls && typeof backendTls === "object") {
    const b = backendTls as Record<string, unknown>;
    result.tls = {
      mode: b.cert || b.key ? "MUTUAL" : "SIMPLE",
      caCertificates: b.root ?? "",
      clientCertificate: b.cert ?? "",
      privateKey: b.key ?? "",
      sni: b.hostname ?? "",
    };
  }

  return result;
}

/**
 * Transform function to strip UI-only fields before submission
 * The backendType field is used in the form for UI purposes but should not be submitted.
 * Also strips out all the unused backend type fields (only keeping the selected one).
 */
export function transformBeforeSubmit(data: unknown): unknown {
  if (typeof data !== "object" || data === null) {
    return data;
  }

  const { backendType, service, host, dynamic, mcp, ai, weight, policies, tls, ...otherFields } = data as Record<string, unknown> & {
    backendType?: string;
    service?: unknown;
    host?: unknown;
    dynamic?: unknown;
    mcp?: unknown;
    ai?: unknown;
    weight?: unknown;
    policies?: unknown;
    tls?: unknown;
  };

  // Build the result with only the relevant backend type field
  const result: Record<string, unknown> = { ...otherFields };

  // Add the selected backend type field (no conversion needed - API now uses object format)
  if (backendType === "service" && service !== undefined && service !== null) {
    result.service = service;
  } else if (backendType === "host" && host !== undefined && host !== null) {
    result.host = host;
    const tlsObj = tls as Record<string, unknown> | undefined;
    const mode = typeof tlsObj?.mode === "string" ? tlsObj.mode : "DISABLED";
    if (mode !== "DISABLED") {
      const existingPolicies =
        policies && typeof policies === "object" ? { ...(policies as Record<string, unknown>) } : {};
      existingPolicies.backendTLS = {
        root: tlsObj?.caCertificates || undefined,
        cert: tlsObj?.clientCertificate || undefined,
        key: tlsObj?.privateKey || undefined,
        hostname: tlsObj?.sni || undefined,
      };
      result.policies = existingPolicies;
    } else if (policies !== undefined && policies !== null) {
      result.policies = policies;
    }
  } else if (backendType === "dynamic" && dynamic !== undefined && dynamic !== null) {
    result.dynamic = dynamic;
  } else if (backendType === "mcp" && mcp !== undefined && mcp !== null) {
    const mcpObj = mcp as Record<string, unknown>;
    const targets = Array.isArray(mcpObj.targets) ? mcpObj.targets : [];
    result.mcp = {
      ...mcpObj,
      targets: targets.map((t) => {
        if (!t || typeof t !== "object") return t;
        const { connectionType, sse, mcp, stdio, openapi, ...rest } = t as Record<string, unknown>;
        const clean: Record<string, unknown> = { ...rest };
        if (connectionType === "sse" && sse) clean.sse = sse;
        else if (connectionType === "mcp" && mcp) clean.mcp = mcp;
        else if (connectionType === "stdio" && stdio) clean.stdio = stdio;
        else if (connectionType === "openapi" && openapi) clean.openapi = openapi;
        return clean;
      }),
    };
  } else if (backendType === "ai" && ai !== undefined && ai !== null) {
    result.ai = ai;
  }

  // Add optional common fields
  if (weight !== undefined && weight !== null) {
    result.weight = weight;
  }
  if (policies !== undefined && policies !== null) {
    result.policies = policies;
  }

  return result;
}
