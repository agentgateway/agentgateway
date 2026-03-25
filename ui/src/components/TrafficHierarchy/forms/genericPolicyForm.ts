import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Generic policy form — used for any policy type that doesn't have
 * a dedicated form (e.g., corsPolicyForm). Accepts any JSON object.
 */
export const schema: RJSFSchema = {
  type: "object",
  additionalProperties: true,
};

export const uiSchema: UiSchema = {
  "ui:title": "",
};

export const defaultValues = {};

export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
