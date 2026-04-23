import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = {
    type: "object", 
    properties: {
        mode: { 
            type: "string",
            title: "Mode",
            enum: ["strict", "optional", "permissive"],
            default: "strict",
            description: "script = reject missing/invalid tokens, optional = allow mising tokens, permissive = allow invalid tokens",
        },
        issuer: {
            type: "string",
            title: "Issuer",
            description: "Expected issuer (iss claim) of incoming JWTs",
        },
        audiences: { 
            type: "array",
            title: "Audiences",
            items: {
                type: "string",
            },
            description: "Expected audiences (aud claim) of incoming JWTs.  Leave blank to skip audience validation.",
        },
        jwks: {
            type: "string",
            title: "JWKS",
            description: "Public keys used to verifiy tokens.  Provide inline JSON, a URL, or a file path",
        },
    },
 };

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
      "ui:label": false,
      "ui:help": "Expected audiences (aud claim). Leave empty to skip audience validation.",
    },
    jwks: {
      "ui:widget": "textarea",
      "ui:placeholder": '{"keys":[...]}  or  https://.../.well-known/jwks.json  or /path/to/jwks.json',
      "ui:options": { rows: 4 },
      "ui:help": "Public keys used to verify tokens. Provide inline JSON, a URL, or a file path.",
    },
  };

 export const defaultValues = { 
    issuer: "",
    audiences: [],
    jwks: '{"keys":[]}',
 }

 export function transformBeforeSubmit(data: unknown): unknown { 
    const d = data as Record<string, unknown>;
    const providers = d.providers as unknown[] | undefined;
    if (!providers || providers.length === 0) { 
      const { providers: _drop, ...rest } = d;
      return rest;
    }
    return d;
 }