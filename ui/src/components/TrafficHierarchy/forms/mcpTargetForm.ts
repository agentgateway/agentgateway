import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Form for a single MCP Target (LocalMcpTarget).
 * Extracted from the mcpForm targets array item schema.
 * 
 * Note: The `policies` field is managed separately via the tree hierarchy
 * and is not shown in this form.
 */
export const schema: RJSFSchema = {
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
                  items: { type: "string" },
                },
                env: {
                  type: "object",
                  title: "Environment Variables",
                  description: "Environment variables for the command",
                  additionalProperties: { type: "string" },
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
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:placeholder": "e.g., my-mcp-server",
  },
  connectionType: {
    "ui:widget": "select",
    "ui:help": "SSE and MCP are network-based, STDIO runs a local process",
  },
  sse: {
    host: { "ui:placeholder": "localhost" },
    port: { "ui:placeholder": "8080" },
    path: { "ui:placeholder": "/sse" },
  },
  mcp: {
    host: { "ui:placeholder": "localhost" },
    port: { "ui:placeholder": "8080" },
  },
  stdio: {
    cmd: { "ui:placeholder": "/usr/local/bin/my-mcp-server" },
  },
  openapi: {
    host: { "ui:placeholder": "api.example.com" },
    schema: { "ui:placeholder": "/path/to/openapi.json" },
  },
};

export const defaultValues = {
  name: "",
  connectionType: "sse",
  sse: { host: "localhost" },
};

export function transformForForm(data: unknown): unknown {
  if (typeof data !== "object" || data === null) {
    return data;
  }

  const targetData = data as Record<string, unknown>;
  // Filter out policies - they're managed separately via the tree
  const { policies: _policies, ...result } = targetData;

  // Determine connectionType based on which field is present
  if ("sse" in targetData) {
    result.connectionType = "sse";
  } else if ("mcp" in targetData) {
    result.connectionType = "mcp";
  } else if ("stdio" in targetData) {
    result.connectionType = "stdio";
  } else if ("openapi" in targetData) {
    result.connectionType = "openapi";
  } else {
    // Default to sse if no connection type is found
    result.connectionType = "sse";
  }

  return result;
}

export function transformBeforeSubmit(data: unknown): unknown {
  if (typeof data !== "object" || data === null) return data;
  
  const { connectionType, sse, mcp, stdio, openapi, ...rest } = data as Record<string, any>;
  
  // Remove connectionType (UI helper field) and all variant fields
  // Then add back only the variant field that matches the selected connectionType
  const result: Record<string, any> = { ...rest };
  
  // Keep only the connection variant that matches the selected type
  if (connectionType === "sse" && sse) {
    result.sse = sse;
  } else if (connectionType === "mcp" && mcp) {
    result.mcp = mcp;
  } else if (connectionType === "stdio" && stdio) {
    result.stdio = stdio;
  } else if (connectionType === "openapi" && openapi) {
    result.openapi = openapi;
  }
  
  return result;
}
