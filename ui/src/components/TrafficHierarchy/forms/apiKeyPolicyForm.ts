import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: { 
        keys: { 
            type: "array",
            title: "API Keys",
            items: { 
                type: "object",
                properties: { 
                    key: { 
                        type: "string",
                        title: "Key",
                    }
                }
            }
        },
        mode: { 
            type: "string",
            title: "Mode",
            enum: ["strict", "optional"],
            default: "strict",
        },
    },
};

export const uiSchema: UiSchema = { 
    "ui:title": "",
    keys: { 
        "ui:help": "List of API keys accepted for authentication",
        items: { 
            key: { 
                "ui:placeholder": "e.g., my-secret-api-key",
                "ui:widget": "password",
            }
        }
    },
    mode: { 
        "ui:widget": "select",
        "ui:help": "strict = reject requests with missing/invalid API keys, optional = allow unauthenticated requests",
    }
}

export const defaultValues = { 
    keys: [],
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}