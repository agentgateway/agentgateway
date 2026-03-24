import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalLLMConfig } from "../../../config";

/**
 * Manually configured JSON Schema for LLM Configuration
 * Handcrafted to match LocalLLMConfig type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  required: [],
  additionalProperties: true,
  properties: {
    port: {
      type: "number",
      title: "Port",
      description: "Port for LLM gateway (optional, defaults to main gateway)",
    },
    policies: {
      type: "object",
      description: "Policies for handling incoming requests before model selection",
      additionalProperties: true,
    },
  },
};

/**
 * UI Schema for LLM Configuration
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  port: {
    "ui:placeholder": "8080",
    "ui:help": "Leave empty to use the main gateway port",
  },
};

/**
 * Default values for a new LLM config
 */
export const defaultValues: Partial<LocalLLMConfig> = {
  models: [],
};

/**
 * Type guard to validate data matches LocalLLMConfig
 */
export function isLocalLLMConfig(data: unknown): data is LocalLLMConfig {
  return (
    typeof data === "object" &&
    data !== null
  );
}

/**
 * Transform function - no transformation needed
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
