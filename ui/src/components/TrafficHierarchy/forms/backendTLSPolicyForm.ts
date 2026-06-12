import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: { 
        hostname: { 
            type: "string",
            title: "Hostname",
            description: "SNI hostname to use when connecting",
        },
        insecure: { 
            type: "boolean",
            title: "Insecure",
            description: "Skip certificate verification",
        },
        insecureHost: { 
            type: "boolean",
            title: "Insecure Host",
            description: "Skip hostname verification",
        },
        cert: { 
            type: "string",
            title: "Client Certificate",
            description: "PEM-encoded client certificate",
        },
        key: { 
            type: "string",
            title: "Client Key", 
            description: "PEM-encoded client private key",
        },
        root: { 
            type: "string",
            title: "Root CA",
            description: "PEM-encoded root CA certificate",
        },
        alpn: { 
            type: "array",
            title: "ALPN Protocols",
            items: { 
                type: "string",
            }
        },
        subjectAltNames: { 
            type: "array",
            title: "Subject Alt Names",
            items: { 
                type: "string",
            }
        }
    },
}

export const uiSchema: UiSchema = { 
    "ui:title": "",
    cert: { "ui:widget": "textarea", "ui:options": { rows: 4 }},
    key: { "ui:widget": "textarea", "ui:options": { rows: 4 }},
    root: { "ui:widget": "textarea", "ui:options": { rows: 4 }},
}

export const defaultValues = { 
    hostname: "",
    insecure: false,
    insecureHost: false,
}

export function transformForForm(data: unknown): unknown { 
    return data;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    return data;
}