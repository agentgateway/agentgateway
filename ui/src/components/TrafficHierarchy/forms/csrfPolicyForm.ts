import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: { 
        additionalOrigins: { 
            type: "array",
            title: "Additional Origins",
            description: "Extra allowed origins beyond the request host",
            items: { 
                type: "string",
            }
        }
    }
};

export const uiSchema: UiSchema = { 
    "ui:title": "",
    additionalOrigins: {
        items: {
            "ui:placeholder": "https://example.com",
        },
    }    
};

export const defaultValues = {
    additionalOrigins: [],
};

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}