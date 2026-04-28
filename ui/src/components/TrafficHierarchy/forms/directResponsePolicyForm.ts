import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
        status: {
            type: "number",
            title: "Status Code",
            description: "HTTP status code to return (e.g. 200, 404)",
        },
        body: {
            type: "string",
            title: "Body",
            description: "Response body content",
        },
    },
}

export const uiSchema: UiSchema = {
    "ui:title": "",
    status: { 
        "ui:placeholder": "200",
    },
    body: { 
        "ui:widget": "textarea",
        "ui:options": { rows: 6 },
    },
}

export const defaultValues = { 
    status: 200,
    body: "",
}

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    const d = data as any;
    return { 
        status: Number(d.status),
        body: d.body ?? "",
    };
}
