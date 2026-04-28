import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
        defaults: { 
            type: "object",
            title: "Defaults",
            description: "Default parameters sent to the LLM (e.g. temperate, max_tokens)",
            additionalProperties: true,
        },
        overrides: { 
            type: "object",
            title: "Overrides",
            description: "Parameters that override any user-supplied values",
            additionalProperties: true,
        },
        modelAliases: { 
            type: "object",
            title: "Model Aliases",
            description: "Map incoming model namese to provider model names",
            additionalProperties: { 
                type: "string",
            }
        }
    },
};

export const uiSchema: UiSchema = { 
    "ui:title": "",
    defaults: { 
        "ui:label": false,
        "ui:field": "keyValueMap",
        "ui:keyPlaceholder": "parameter-name",
        "ui:valuePlaceholder": "parameter-value",
    },
    overrides: { 
        "ui:label": false,
        "ui:field": "keyValueMap",
        "ui:keyPlaceholder": "parameter-name",
        "ui:valuePlaceholder": "parameter-value",
    },
    modelAliases: { 
        "ui:label": false,
        "ui:field": "keyValueMap",
        "ui:keyPlaceholder": "incoming-model-name",
        "ui:valuePlaceholder": "provider-model-name",
    }
};

export const defaultValues = {};

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}