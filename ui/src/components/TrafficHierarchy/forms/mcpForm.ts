import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalSimpleMcpConfig } from "../../../config";

/**
 * MCP Configuration form.
 * Targets and policies are managed as child nodes in the tree.
 */
export const schema: RJSFSchema = {
  type: "object",
  required: [],
  additionalProperties: true,
  properties: {
    port: {
      type: "number",
      title: "Port",
      description: "Port for MCP gateway (optional)",
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
  },
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  port: {
    "ui:placeholder": "8081",
    "ui:help": "Leave empty to use the main gateway port",
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

export const defaultValues: Partial<LocalSimpleMcpConfig> = {
  statefulMode: "stateless",
};

export function isLocalSimpleMcpConfig(
  data: unknown,
): data is LocalSimpleMcpConfig {
  return typeof data === "object" && data !== null;
}

export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
