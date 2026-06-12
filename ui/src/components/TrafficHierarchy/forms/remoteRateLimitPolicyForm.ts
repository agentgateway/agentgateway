import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    domain: {
      type: "string",
      title: "Domain",
      description: "Rate limit domain identifier",
    },
    backendType: {
      type: "string",
      title: "Backend Type",
      enum: ["host", "service", "backend"],
      default: "host",
    },
    failureMode: {
      type: "string",
      title: "Failure Mode",
      enum: ["failClosed", "failOpen"],
      default: "failClosed",
    },
    descriptors: {
      type: "array",
      title: "Descriptors",
      items: {
        type: "object",
        properties: {
          type: {
            type: "string",
            title: "Type",
            enum: ["requests", "tokens"],
          },
          entries: {
            type: "array",
            title: "Entries",
            items: {
              type: "object",
              properties: {
                key: { type: "string", title: "Key" },
                value: { type: "string", title: "Value" },
              },
            },
          },
        },
      },
    },
  },
  allOf: [
    {
      if: {
        properties: { backendType: { const: "host" } },
        required: ["backendType"],
      },
      then: {
        properties: {
          host: { type: "string", title: "Host" },
        },
      },
    },
    {
      if: {
        properties: { backendType: { const: "service" } },
        required: ["backendType"],
      },
      then: {
        properties: {
          serviceName: { type: "string", title: "Service Name" },
          serviceNamespace: { type: "string", title: "Service Namespace" },
          servicePort: { type: "number", title: "Service Port" },
        },
      },
    },
    {
      if: {
        properties: { backendType: { const: "backend" } },
        required: ["backendType"],
      },
      then: {
        properties: {
          backendRef: { type: "string", title: "Backend Name" },
        },
      },
    },
  ],
}

export const uiSchema: UiSchema = { 
    "ui:title": "",
    backendType: { 
        "ui:widget": "select",
        "ui:enumNames": ["Host", "Service", "Backend"],
    },
    failureMode: { 
        "ui:widget": "select",
        "ui:enumNames": ["Fail Closed", "Fail Open"],
    },
    host: { "ui:placeholder": "localhost:9001" },
}

export const defaultValues = { 
    domain: "",
    backendType: "host",
    host: "localhost:9001",
    failureMode: "failClosed",
    descriptors: [],
}

export function transformForForm(data: unknown): unknown { 
    const d = data as any;
    const result: any = { 
        domain: d?.domain ?? "",
        failureMode: d?.failureMode ?? "failClosed",
        descriptors: d?.descriptors ?? [],
        backendType: "host",
    }

    if (d?.host) { 
        result.backendType = "host"; 
        result.host = d.host;
    } else if (d?.service) { 
        result.backendType = "service";
        result.serviceName = d.service?.name?.hostname ?? "";
        result.serviceNamespace = d.service?.name?.namespace ?? "";
        result.servicePort = d.service?.port;
    } else if (d?.backend) { 
        result.backendType = "backend";
        result.backendRef = d.backend;
    }

    return result;
}

export function transformBeforeSubmit(data: unknown): unknown { 
    const d = data as any;
    const result: any = { 
        domain: d.domain ?? "",
        failureMode: d.failureMode,
        descriptors: d.descriptors ?? [],
    }

    if (d.backendType === "host") { 
        result.host = d.host ?? "";
    } else if (d.backendType === "service") { 
        result.service = {
            name: { 
                hostname: d.serviceName ?? "",
                namespace: d.serviceNamespace ?? "",
            },
            port: Number(d.servicePort ?? 0),
        }
    } else if (d.backendType === "backend") { 
        result.backend = d.backendRef ?? "";
    }

    return result;
}