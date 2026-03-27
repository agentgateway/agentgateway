import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { FullLocalBackend } from "../../../config";

/**
 * Manually configured JSON Schema for top-level Backend
 * Handcrafted to match FullLocalBackend type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  required: ["name", "host"],
  additionalProperties: true,
  properties: {
    name: {
      type: "string",
      title: "Name",
      description: "Unique name for this backend",
    },
    host: {
      type: "string",
      title: "Host",
      description: "Hostname or IP address with optional port (e.g., example.com:8080)",
    },
    policies: {
      type: "object",
      description: "Backend-level policies (advanced)",
      additionalProperties: true,
    },
  },
};

/**
 * UI Schema for top-level Backend
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:placeholder": "e.g., my-backend",
    "ui:help": "Unique identifier for this backend",
  },
  host: {
    "ui:placeholder": "e.g., api.example.com:443",
    "ui:help": "Target host and optional port for this backend",
  },
  policies: {
    "ui:title": "",
    "ui:help": "Optional policies to apply to this backend",
  },
};

/**
 * Default values for a new backend
 */
export const defaultValues: Partial<FullLocalBackend> = {
  name: "",
  host: "",
};

/**
 * Type guard to validate data matches FullLocalBackend
 */
export function isFullLocalBackend(data: unknown): data is FullLocalBackend {
  return (
    typeof data === "object" &&
    data !== null &&
    "name" in data &&
    "host" in data
  );
}

/**
 * Transform function - no transformation needed
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
