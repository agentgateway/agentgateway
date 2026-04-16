import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
  type: "object",
  properties: {
    scheme: {
      type: "string",
      title: "Scheme",
      enum: ["", "http", "https"],
      default: "",
    },
    status: {
      type: "number",
      title: "Status Code",
      description: "HTTP redirect status code",
    },
    authorityType: {
      type: "string",
      title: "Authority Rewrite",
      enum: ["none", "auto", "full", "host", "port"],
      default: "none",
    },
    authorityValue: {
      type: "string",
      title: "Authority Value",
    },
    pathType: {
      type: "string",
      title: "Path Rewrite",
      enum: ["none", "full", "prefix"],
      default: "none",
    },
    pathValue: {
      type: "string",
      title: "Path Value",
    },
  },
};

export const uiSchema: UiSchema = {
  "ui:title": "",
  scheme: {
    "ui:widget": "select",
    "ui:enumNames": ["No change", "http", "https"],
  },
  status: {
    "ui:placeholder": "301",
    "ui:help": "e.g. 301 (permanent), 302 (temporary)",
  },
  authorityType: {
    "ui:widget": "select",
    "ui:enumNames": [
      "No rewrite",
      "Auto",
      "Full hostname",
      "Host only",
      "Port only",
    ],
  },
  authorityValue: { "ui:placeholder": "e.g. example.com or 8080" },
  pathType: {
    "ui:widget": "select",
    "ui:enumNames": ["No rewrite", "Full path", "Prefix replacement"],
  },
  pathValue: { "ui:placeholder": "e.g. /new-path or /api" },
};

export const defaultValues = {
  scheme: "",
  authorityType: "none",
  pathType: "none",
};

export function transformForForm(data: unknown): unknown {
  const d = data as any;
  const result: any = { authorityType: "none", pathType: "none" };

  if (d?.scheme) result.scheme = d.scheme;
  if (d?.status) result.status = d.status;

  if (d?.authority != null) {
    if (d.authority === "auto") {
      result.authorityType = "auto";
    } else if (d.authority === "none") {
      result.authorityType = "none";
    } else if (typeof d.authority === "object") {
      if ("full" in d.authority) {
        result.authorityType = "full";
        result.authorityValue = d.authority.full;
      } else if ("host" in d.authority) {
        result.authorityType = "host";
        result.authorityValue = d.authority.host;
      } else if ("port" in d.authority) {
        result.authorityType = "port";
        result.authorityValue = String(d.authority.port);
      }
    }
  }

  if (d?.path != null && typeof d.path === "object") {
    if ("full" in d.path) {
      result.pathType = "full";
      result.pathValue = d.path.full;
    } else if ("prefix" in d.path) {
      result.pathType = "prefix";
      result.pathValue = d.path.prefix;
    }
  }

  return result;
}

export function transformBeforeSubmit(data: unknown): unknown {
  const d = data as any;
  const result: any = {};

  if (d.scheme) result.scheme = d.scheme;
  if (d.status) result.status = Number(d.status);

  if (d.authorityType === "auto") {
    result.authority = "auto";
  } else if (d.authorityType === "full") {
    result.authority = { full: d.authorityValue ?? "" };
  } else if (d.authorityType === "host") {
    result.authority = { host: d.authorityValue ?? "" };
  } else if (d.authorityType === "port") {
    result.authority = { port: Number(d.authorityValue ?? 0) };
  }

  if (d.pathType === "full") {
    result.path = { full: d.pathValue ?? "" };
  } else if (d.pathType === "prefix") {
    result.path = { prefix: d.pathValue ?? "" };
  }

  return result;
}