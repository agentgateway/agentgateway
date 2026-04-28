import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    required: ["codes"],
    properties: { 
        attempts: { 
            type: "number",
            title: "Attempts",
            description: "Number of retry attempts",
        },
        backoff: { 
            type: "string",
            title: "Backoff",
            description: "Delay between retries.  Must include a unit (e.g. 1s, 500ms)",
        },
        codes: { 
            type: "array",
            title: "Retry on Status Codes",
            description: "HTTP status codes that trigger a retry",
            items: { 
                type: "number",
            },
        },
    },
};

export const uiSchema: UiSchema = { 
    "ui:title": "",
    backoff: { 
        "ui:placeholder": "1s",
    },
    codes: { 
        items: {
            "ui:placeholder": "503",
        }
    }
};

export const defaultValues = {};

export function transformForForm(data: unknown): unknown {
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown {
    return data;
}