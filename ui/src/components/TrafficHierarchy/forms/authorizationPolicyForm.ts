import type { RJSFSchema, UiSchema } from "@rjsf/utils";

/**
 * Authorization policy form — used for authorization and mcpAuthorization policies.
 * Defines the RuleSet schema with rules as an array of strings.
 */
export const schema: RJSFSchema = {
  type: "object",
  required: ["rules"],
  properties: {
    rules: {
      type: "array",
      title: "Authorization Rules",
      description: "CEL expressions that evaluate to true for authorized requests",
      items: {
        type: "string",
        title: "Rule",
      },
      default: [],
      minItems: 0,
    },
  },
  additionalProperties: false,
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  rules: {
    "ui:options": {
      orderable: false,
      addable: true,
      removable: true,
    },
    items: {
      "ui:widget": "textarea",
      "ui:placeholder": "e.g., request.headers['x-user-role'] == 'admin'",
      "ui:options": {
        rows: 2,
      },
    },
  },
};

export const defaultValues = {
  rules: [],
};

export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
