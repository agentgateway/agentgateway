import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
    type: "object",
    properties: {
        targetType: {
            type: "string",
            title: "Target Type",
            enum: ["host", "service", "backend"],
            default: "host",
        },
        host: { 
            type: "string",
            title: "Host",
        },
        serviceName: { 
            type: "string",
            title: "Service Name",
        },
        servicePort: { 
            type: "number",
            title: "Service Port",
        },
        backend: { 
            type: "string",
            title: "Backend",
        },
        protocol: { 
            type: "string",
            title: "Protocol",
            enum: ["grpc", "http"],
            default: "grpc",
        },
        failureMode: { 
            type: "string",
            title: "Failure Mode",
            enum: ["allow", "deny"],
            default: "deny",
        }
    },
};

export const uiSchema: UiSchema = {
    "ui:title": "",
    targetType: { 
        "ui:widget": "select",
        "ui:help": "How to specify the authorization service endpoint",
    },
    host: { 
        "ui:placeholder": "authz.example.com:9001",
        "ui:help": "Hostname or IP address of the authorization service",
    },
    serviceName: { 
        "ui:placeholder": "authz-service.namespace.svc.cluster.local",
        "ui:help": "Service name for the authorization service",
    },
    servicePort: {
        "ui:help": "Port for the authorization service",
    },
    backend: { 
        "ui:placeholder": "my-authz-backend",
        "ui:help": "Name of a backend defined in the top-level backends list",
    },
    protocol: { 
        "ui:widget": "select",
        "ui:help": "gRPC is recommended unless the server only supports HTTP",
    },
    failureMode: { 
        "ui:widget": "select",
        "ui:help": "Behavior when the authorization serivce is unavailable",
    },
};

export const defaultValues = {
    targetType: "host",
    host: "",
    protocol: "grpc",
    failureMode: "deny",
};

export function transformForForm(data: unknown): unknown { 
    const d = data as Record<string, unknown>;
    const result: Record<string, unknown> = {};

    if (typeof d.host === "string") { 
        result.targetType = "host";
        result.host = d.host; 
    } else if (d.service) { 
        const svc = d.service as Record<string, unknown>;
        result.targetType = "service";
        result.serviceName = svc.name;
        result.servicePort = svc.port;
    } else if (typeof d.backend === "string") { 
        result.targetType = "backend";
        result.backend = d.backend;
    } else { 
        result.targetType = "host";
    }

    if (d.protocol && typeof d.protocol === "object") { 
        result.protocol = "grpc" in (d.protocol as object) ? "grpc" : "http";
    } else { 
        result.protocol = "grpc";
    }

    result.failureMode = d.failureMode ?? "deny";

    return result;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    const d = data as Record<string, unknown>;
    const result: Record<string, unknown> = {};

    if (d.targetType === "host" && d.host) {
      result.host = d.host;
    } else if (d.targetType === "service" && d.serviceName) {
      result.service = { name: d.serviceName, port: d.servicePort };
    } else if (d.targetType === "backend" && d.backend) {
      result.backend = d.backend;
    }

    result.protocol = { [d.protocol as string ?? "grpc"]: {} };

    if (d.failureMode) {
      result.failureMode = d.failureMode;
    }

    return result;
}