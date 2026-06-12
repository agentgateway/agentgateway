import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    backendType: {
      type: "string",
      title: "Backend Type",
      enum: ["host", "service", "backend"],
      default: "host",
    },
    percentage: {
      type: "number",
      title: "Mirror Percentage",
      description: "Percentage of requests to mirror (0-100)",
    },
  },
  allOf: [
    {
      if: { properties: { backendType: { const: "host" } }, required: ["backendType"] },
      then: {
        properties: {
          host: {
            type: "string",
            title: "Host",
            description: "Hostname or IP address with port (e.g. mirror-svc:8080)",
          },
        },
      },
    },
    {
      if: { properties: { backendType: { const: "service" } }, required: ["backendType"] },
      then: {
        properties: {
          serviceName: { type: "string", title: "Service Name" },
          serviceNamespace: { type: "string", title: "Service Namespace" },
          servicePort: { type: "number", title: "Service Port" },
        },
      },
    },
    {
      if: { properties: { backendType: { const: "backend" } }, required: ["backendType"] },
      then: {
        properties: {
          backendRef: {
            type: "string",
            title: "Backend Name",
            description: "Must be defined in the top-level backends list",
          },
        },
      },
    },
  ],
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  backendType: {
    "ui:widget": "select",
    "ui:enumNames": ["Host", "Service", "Backend"],
  },
  host: { "ui:placeholder": "mirror-svc:8080" },
  percentage: { "ui:help": "Percentage of traffic to mirror (0-100)" },
};

export const defaultValues = {
  backendType: "host",
  host: "",
  percentage: 100,
};

export function transformForForm(data: unknown): unknown {
  const d = data as any;
  const result: any = { percentage: d?.percentage ?? 100, backendType: "host" };

  const b = d?.backend;
  if (b === "invalid" || b === null) {
    result.backendType = "host";
  } else if (typeof b === "object") {
    if ("host" in b) {
      result.backendType = "host";
      result.host = b.host;
    } else if ("service" in b) {
      result.backendType = "service";
      result.serviceName = b.service?.name?.hostname ?? "";
      result.serviceNamespace = b.service?.name?.namespace ?? "";
      result.servicePort = b.service?.port;
    } else if ("backend" in b) {
      result.backendType = "backend";
      result.backendRef = b.backend;
    }
  }

  return result;
}

export function transformBeforeSubmit(data: unknown): unknown {
  const d = data as any;
  let backend: any;

  if (d.backendType === "host") {
    backend = { host: d.host ?? "" };
  } else if (d.backendType === "service") {
    backend = {
      service: {
        name: {
          hostname: d.serviceName ?? "",
          namespace: d.serviceNamespace ?? "",
        },
        port: Number(d.servicePort ?? 0),
      },
    };
  } else if (d.backendType === "backend") {
    backend = { backend: d.backendRef ?? "" };
  }

  return {
    backend,
    percentage: Number(d.percentage ?? 100),
  };
}