import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: { 
        requestTimeout: { 
            type: "string",
            title: "Request Timeout",
            description: "Timeout for the full request (e.g. 30s, 1m).  Must include a unit",
        },
        backendRequestTimeout: { 
            type: "string",
            title: "Backend Request Timeout",
            description: "Timeout for the backend request (e.g. 30s, 1m).  Must include a unit",
        }
    }    
};

export const uiSchema: UiSchema = { 
    "ui:title": "",
    requestTimeout: { 
        "ui:placeholder": "30s",
    },
    backendRequestTimeout: { 
        "ui:placeholder": "30s",
    }
};

export const defaultValues = {};

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}