import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
      mode: {
        type: "string",
        title: "Mode",
        enum: ["strict", "optional", "permissive"],
        default: "strict",
      },
      issuer: {
        type: "string",
        title: "Issuer",
      },
      audiences: {
        type: "array",
        title: "Audiences",
        items: { type: "string" },
      },
      jwks: {
        type: "string",
        title: "JWKS",
      },
      provider: {
        type: "string",
        title: "Provider",
        enum: ["none", "auth0", "keycloak"],
        default: "none",
      },
      resourceMetadata: {
        type: "object",
        title: "Resource Metadata",
        additionalProperties: { type: "string" },
      },
    },
}

export const uiSchema: UiSchema = { 
    "ui:title": "",
    mode: {
      "ui:widget": "select",
      "ui:help": "strict = reject missing/invalid tokens, optional = allow missing tokens, permissive = allow invalid tokens",
    },
    issuer: {
      "ui:placeholder": "https://accounts.example.com",
      "ui:help": "Expected issuer (iss claim) of incoming JWTs",
    },
    audiences: {
      "ui:help": "Expected audiences (aud claim). Leave empty to skip audience validation.",
    },
    jwks: {
      "ui:widget": "textarea",
      "ui:placeholder": '{"keys":[...]}  or  https://.../.well-known/jwks.json  or /path/to/jwks.json',
      "ui:options": { rows: 4 },
      "ui:help": "Public keys used to verify tokens. Provide inline JSON, a URL, or a file path.",
    },
    provider: {
      "ui:widget": "select",
      "ui:help": "Optional identity provider integration",
    },
    resourceMetadata: {
      "ui:field": "keyValueMap",
      "ui:keyPlaceholder": "key",
      "ui:valuePlaceholder": "value",
      "ui:help": "Additional metadata about this resource",
    },
}

export const defaultValues = { 
    issuer: "",
    audiences: [],
    jwks: '{"keys":[]}',
    resourceMetadata: {},
};

export function transformBeforeSubmit(data: unknown): unknown { 
    const data_ = data as Record<string, unknown>;

    // convert "none" provider selection back to undefined
    if (data_.provider === "none") {
      const { provider: _, ...rest } = data_;
      return rest;
    }

    // wrap provider value in its object form e.g. { auth0: {} }
    return {
      ...data_,
      provider: { [data_.provider as string]: {} },
    };
}