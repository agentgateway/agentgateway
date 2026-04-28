import type { RJSFSchema, UiSchema } from "@rjsf/utils";

  export const schema: RJSFSchema = {
    type: "object",
    properties: {
      htpasswd: {
        type: "string",
        title: "htpasswd",
        description: "htpasswd file contents or file path",
      },
      realm: {
        type: "string",
        title: "Realm",
        description: "Realm name for the WWW-Authenticate header",
      },
      mode: {
        type: "string",
        title: "Mode",
        enum: ["strict", "optional"],
        default: "strict",
      },
    },
  };

  export const uiSchema: UiSchema = {
    "ui:title": "",
    htpasswd: {
      "ui:widget": "textarea",
      "ui:placeholder": "/path/to/.htpasswd  or  paste htpasswd contents",
      "ui:options": { rows: 4 },
      "ui:help": "Provide a file path or paste inline htpasswd-formatted credentials",
    },
    realm: {
      "ui:placeholder": "e.g., My API",
      "ui:help": "Sent to clients in the WWW-Authenticate response header",
    },
    mode: {
      "ui:widget": "select",
      "ui:help": "strict = reject requests with missing/invalid credentials, optional = allow unauthenticated requests",
    },
  };

  export const defaultValues = {
    htpasswd: "",
  };

  export function transformBeforeSubmit(data: unknown): unknown {
    return data;
  }