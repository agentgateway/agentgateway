import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalSimpleMcpConfig } from "../../../config";

/**
 * Manually configured JSON Schema for MCP Configuration
 * Handcrafted to match LocalSimpleMcpConfig type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  required: ["targets"],
  additionalProperties: true,
  properties: {
    port: {
      type: "number",
      title: "Port",
      description: "Port for MCP gateway (optional)",
    },
    targets: {
      type: "array",
      title: "MCP Targets",
      description: "List of MCP server targets",
      items: {
        type: "object",
        required: ["name"],
        additionalProperties: true,
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
          policies: {
            type: "object",
            title: "Target Policies",
            description: "Optional policies for this specific target",
            additionalProperties: true,
          },
        },
        dependencies: {
          connectionType: {
            oneOf: [
              {
                properties: {
                  connectionType: { const: "sse" },
                  sse: {
                    type: "object",
                    title: "SSE Connection Settings",
                    required: ["host"],
                    properties: {
                      host: {
                        type: "string",
                        title: "Host",
                        description: "SSE server hostname",
                      },
                      port: {
                        type: "integer",
                        title: "Port",
                        minimum: 1,
                        maximum: 65535,
                      },
                      path: {
                        type: "string",
                        title: "Path",
                        default: "/sse",
                        description: "SSE endpoint path",
                      },
                    },
                  },
                },
                required: ["sse"],
              },
              {
                properties: {
                  connectionType: { const: "mcp" },
                  mcp: {
                    type: "object",
                    title: "MCP Connection Settings",
                    required: ["host"],
                    properties: {
                      host: {
                        type: "string",
                        title: "Host",
                        description: "MCP server hostname",
                      },
                      port: {
                        type: "integer",
                        title: "Port",
                        minimum: 1,
                        maximum: 65535,
                      },
                      path: {
                        type: "string",
                        title: "Path",
                        description: "MCP endpoint path",
                      },
                    },
                  },
                },
                required: ["mcp"],
              },
              {
                properties: {
                  connectionType: { const: "stdio" },
                  stdio: {
                    type: "object",
                    title: "STDIO Connection Settings",
                    required: ["cmd"],
                    properties: {
                      cmd: {
                        type: "string",
                        title: "Command",
                        description: "Command to execute for STDIO communication",
                      },
                      args: {
                        type: "array",
                        title: "Arguments",
                        description: "Command-line arguments",
                        items: {
                          type: "string",
                        },
                      },
                      env: {
                        type: "object",
                        title: "Environment Variables",
                        description: "Environment variables for the command",
                        additionalProperties: {
                          type: "string",
                        },
                      },
                    },
                  },
                },
                required: ["stdio"],
              },
              {
                properties: {
                  connectionType: { const: "openapi" },
                  openapi: {
                    type: "object",
                    title: "OpenAPI Connection Settings",
                    required: ["host", "schema"],
                    properties: {
                      host: {
                        type: "string",
                        title: "Host",
                        description: "OpenAPI server hostname",
                      },
                      port: {
                        type: "integer",
                        title: "Port",
                        minimum: 1,
                        maximum: 65535,
                      },
                      path: {
                        type: "string",
                        title: "Path",
                        description: "API base path",
                      },
                      schema: {
                        type: "string",
                        title: "OpenAPI Schema",
                        description: "Path to OpenAPI schema file or URL",
                      },
                    },
                  },
                },
                required: ["openapi"],
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
      description: "Whether to maintain state across requests",
    },
    prefixMode: {
      type: "string",
      title: "Prefix Mode",
      enum: ["always", "conditional"],
      description: "When to use target name as prefix",
    },
    policies: {
      type: "object",
      description: "Global MCP policies",
      additionalProperties: true,
    },
  },
};

/**
 * UI Schema for MCP Configuration
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  port: {
    "ui:placeholder": "8081",
    "ui:help": "Leave empty to use the main gateway port",
  },
  targets: {
    "ui:options": {
      orderable: true,
      addable: true,
      removable: true,
    },
    items: {
      name: {
        "ui:placeholder": "my-mcp-server",
        "ui:help": "Unique identifier for this MCP target",
      },
      connectionType: {
        "ui:widget": "select",
        "ui:help": "SSE for server-sent events, STDIO for local processes, OpenAPI for REST APIs",
      },
      sse: {
        host: {
          "ui:placeholder": "localhost",
        },
        port: {
          "ui:placeholder": "3000",
        },
        path: {
          "ui:placeholder": "/sse",
        },
      },
      mcp: {
        host: {
          "ui:placeholder": "localhost",
        },
        port: {
          "ui:placeholder": "3000",
        },
        path: {
          "ui:placeholder": "/mcp",
        },
      },
      stdio: {
        cmd: {
          "ui:placeholder": "node",
          "ui:help": "Executable command to run",
        },
        args: {
          "ui:help": "Arguments to pass to the command",
        },
        env: {
          "ui:help": "Environment variables as key-value pairs",
        },
      },
      openapi: {
        host: {
          "ui:placeholder": "api.example.com",
        },
        port: {
          "ui:placeholder": "443",
        },
        path: {
          "ui:placeholder": "/v1",
        },
        schema: {
          "ui:placeholder": "/path/to/openapi.json or https://api.example.com/openapi.json",
        },
      },
    },
  },
  statefulMode: {
    "ui:widget": "select",
    "ui:help": "Stateful mode maintains state across requests",
  },
  prefixMode: {
    "ui:widget": "select",
    "ui:help": "Controls when to prefix tool names with target name",
  },
};

/**
 * Default values for a new MCP config
 */
export const defaultValues: Partial<LocalSimpleMcpConfig> = {
  targets: [
    {
      name: "example-mcp-server",
      connectionType: "sse",
      sse: {
        host: "localhost",
        port: 3000,
        path: "/sse",
      },
    },
  ],
  statefulMode: "stateless",
};

/**
 * Type guard to validate data matches LocalSimpleMcpConfig
 */
export function isLocalSimpleMcpConfig(
  data: unknown,
): data is LocalSimpleMcpConfig {
  return (
    typeof data === "object" &&
    data !== null &&
    "targets" in data &&
    Array.isArray((data as any).targets)
  );
}

/**
 * Transform function - no transformation needed
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
