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
    },
    dependencies: {
      targetType: {
        oneOf: [
          {
            properties: {
              targetType: { const: "host" },
              host: {
                type: "string",
                title: "Host",
              },
              failureMode: {
                type: "string",
                title: "Failure Mode",
                enum: ["failClosed", "failOpen"],
              },
            },
            required: ["host"],
          },
          {
            properties: {
              targetType: { const: "service" },
              serviceNamespace: {
                type: "string",
                title: "Service Namespace",
              },
              serviceHostname: {
                type: "string",
                title: "Service Hostname",
              },
              servicePort: {
                type: "number",
                title: "Service Port",
              },
              failureMode: {
                type: "string",
                title: "Failure Mode",
                enum: ["failClosed", "failOpen"],
              },
            },
            required: ["serviceNamespace", "serviceHostname", "servicePort"],
          },
          {
            properties: {
              targetType: { const: "backend" },
              backend: {
                type: "string",
                title: "Backend",
              },
              failureMode: {
                type: "string",
                title: "Failure Mode",
                enum: ["failClosed", "failOpen"],
              },
            },
            required: ["backend"],
          },
        ],
      },
    },
  };

export const uiSchema: UiSchema = {
    "ui:title": "",
    targetType: {
      "ui:widget": "select",
      "ui:help": "How to specify the external processor service endpoint",
    },
    host: {
      "ui:placeholder": "ext-proc.example.com:9001",
      "ui:help": "Hostname or IP address of the external processor service",
    },
    serviceNamespace: {
      "ui:placeholder": "default",
      "ui:help": "Kubernetes namespace of the external processor service",
    },
    serviceHostname: {
      "ui:placeholder": "ext-proc-service",
      "ui:help": "Service hostname (e.g., ext-proc-service or ext-proc-service.svc.cluster.local)",
    },
    servicePort: {
      "ui:help": "Port for the external processor service",
    },
    backend: {
      "ui:placeholder": "my-ext-proc-backend",
      "ui:help": "Name of a backend defined in the top-level backends list",
    },
    failureMode: {
      "ui:widget": "select",
      "ui:help": "Behavior when the external processor is unavailable",
    },
};

export const defaultValues = {
    targetType: "host",
    host: "",
    failureMode: "failClosed",
};

export function transformForForm(data: unknown): unknown {
    const d = data as Record<string, unknown>;
    const result: Record<string, unknown> = {};

    if (typeof d.host === "string") {
        result.targetType = "host";
        result.host = d.host;
    } else if (d.service) {
        const svc = d.service as Record<string, unknown>;
        const name = svc.name as Record<string, unknown>;
        result.targetType = "service";
        result.serviceNamespace = name.namespace ?? "default";
        result.serviceHostname = name.hostname ?? "";
        result.servicePort = svc.port;
    } else if (typeof d.backend === "string") {
        result.targetType = "backend";
        result.backend = d.backend;
    } else {
        result.targetType = "host";
    }

    result.failureMode = d.failureMode ?? "failClosed";
    return result;
}

export function transformBeforeSubmit(data: unknown): unknown {
    const d = data as Record<string, unknown>;
    const result: Record<string, unknown> = {};

    if (d.targetType === "host" && d.host) {
      result.host = d.host;
    } else if (d.targetType === "service" && d.serviceNamespace && d.serviceHostname) {
      result.service = {
        name: { namespace: d.serviceNamespace as string, hostname: d.serviceHostname as string },
        port: d.servicePort as number,
      };
    } else if (d.targetType === "backend" && d.backend) {
      result.backend = d.backend;
    }

    if (d.failureMode) result.failureMode = d.failureMode;
    return result;
}