import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
        spec: {
            type: "array",
            title: "Rate Limit Rules",
            items: {
                type: "object",
                required: ["fillInterval"],
                properties: {
                    fillInterval: {
                        type: "string",
                        title: "Fill Interval",
                        description: "How often the bucket refills (e.g. 1s, 1m, 1h)",
                    },
                    tokensPerFill: { 
                        type: "number",
                        title: "Tokens Per Fill",
                    },
                    maxTokens: { 
                        type: "number",
                        title: "Max Tokens",
                    },
                    type: { 
                        type: "string",
                        title: "Type",
                        enum: ["requests", "tokens"],
                    }
                }
            }
        },
    },
}

export const uiSchema: UiSchema = { 
    "ui:title": "",
    spec: { 
        items: { 
            fillInterval: { 
                "ui:placeholder": "1s",
                "ui:help": "e.g. 1s, 1m, 1h",
            },
            type: { 
                "ui:widget": "select",
                "ui:enumNames": ["Requests", "Tokens"],
            },
        },
    },
}

export const defaultValues = { 
    spec: [{ fillInterval: "1s", tokensPerFill: 100, maxTokens: 100 }],
}

export function transformForForm(data: unknown): unknown { 
    return { spec: data };
}

export function transformBeforeSubmit(data: unknown): unknown { 
    const d = data as any;
    return d.spec ?? [];
}