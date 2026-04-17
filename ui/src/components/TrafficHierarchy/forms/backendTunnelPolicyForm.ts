import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    proxyType: {
      type: "string",
      title: "Proxy Type",
      enum: ["host", "service", "backend"],
      default: "host",
    },
  },
  allOf: [
    {
      if: { properties: { proxyType: { const: "host" } }, required: ["proxyType"] },
      then: { properties: { host: { type: "string", title: "Host" } } },
    },
    {
      if: { properties: { proxyType: { const: "service" } }, required: ["proxyType"] },
      then: {
        properties: {
          serviceName: { type: "string", title: "Service Name" },
          serviceNamespace: { type: "string", title: "Service Namespace" },
          servicePort: { type: "number", title: "Service Port" },
        },
      },
    },
    {
      if: { properties: { proxyType: { const: "backend" } }, required: ["proxyType"] },
      then: { properties: { backendRef: { type: "string", title: "Backend Name" } } },
    },
  ],
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  proxyType: {
    "ui:widget": "select",
    "ui:enumNames": ["Host", "Service", "Backend"],
  },
  host: { "ui:placeholder": "proxy-host:8080" },
};

export const defaultValues = {
  proxyType: "host",
  host: "",
};

export function transformForForm(data: unknown): unknown {
  const d = data as any;
  const result: any = { proxyType: "host" };

  const p = d?.proxy;
  if (p === "invalid" || p == null) {
    result.proxyType = "host";
  } else if (typeof p === "object") {
    if ("host" in p) {
      result.proxyType = "host";
      result.host = p.host;
    } else if ("service" in p) {
      result.proxyType = "service";
      result.serviceName = p.service?.name?.hostname ?? "";
      result.serviceNamespace = p.service?.name?.namespace ?? "";
      result.servicePort = p.service?.port;
    } else if ("backend" in p) {
      result.proxyType = "backend";
      result.backendRef = p.backend;
    }
  }

  return result;
}

export function transformBeforeSubmit(data: unknown): unknown {
  const d = data as any;
  let proxy: any;

  if (d.proxyType === "host") {
    proxy = { host: d.host ?? "" };
  } else if (d.proxyType === "service") {
    proxy = {
      service: {
        name: {
          hostname: d.serviceName ?? "",
          namespace: d.serviceNamespace ?? "",
        },
        port: Number(d.servicePort ?? 0),
      },
    };
  } else if (d.proxyType === "backend") {
    proxy = { backend: d.backendRef ?? "" };
  }

  return { proxy };
}