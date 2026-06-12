import type { RJSFSchema, UiSchema } from "@rjsf/utils";

export const schema: RJSFSchema = { 
    type: "object",
    properties: {
        authType: { type: "string", title: "Auth Type", enum: ["passthrough", "key", "gcp", "aws", "azure"], default: "passthrough" },
        keyType: { type: "string", title: "Key Source", enum: ["inline", "file"], default: "inline" },
        keyValue: { type: "string", title: "Key Value" },
        gcpTokenType: { type: "string", title: "Token Type", enum: ["idToken", "accessToken"], default: "idToken" },
        gcpAudience: { type: "string", title: "Audience" },
        awsMode: { type: "string", title: "Credentials", enum: ["ambient", "explicit"], default: "ambient" },
        awsAccessKeyId: { type: "string", title: "Access Key ID" },
        awsSecretAccessKey: { type: "string", title: "Secret Access Key" },
        awsRegion: { type: "string", title: "Region" },
        awsSessionToken: { type: "string", title: "Session Token" },
        azureMode: { type: "string", title: "Azure Mode", enum: ["implicit", "developerImplicit", "explicitClientSecret"], default: "implicit" },
        azureTenantId: { type: "string", title: "Tenant ID" },
        azureClientId: { type: "string", title: "Client ID" },
        azureClientSecret: { type: "string", title: "Client Secret" },
    },
    allOf: [
      {
        if: { properties: { authType: { const: "key" } }, required: ["authType"] },
        then: { required: ["keyValue"] },
      },
      {
        if: {
          properties: { authType: { const: "gcp" }, gcpTokenType: { const: "idToken" } },
          required: ["authType"],
        },
        then: {},
      },
      {
        if: {
          properties: { authType: { const: "aws" }, awsMode: { const: "explicit" } },
          required: ["authType"],
        },
        then: { required: ["awsAccessKeyId", "awsSecretAccessKey"] },
      },
      {
        if: {
          properties: { authType: { const: "azure" }, azureMode: { const:
  "explicitClientSecret" } },
          required: ["authType"],
        },
        then: { required: ["azureTenantId", "azureClientId", "azureClientSecret"] },
      },
    ],
};

export const uiSchema: UiSchema = {
    "ui:title": "",
    authType: { "ui:widget": "select" },
    keyType: { "ui:widget": "select" },
    keyValue: { "ui:placeholder": "your-api-key-or-path" },
    gcpTokenType: { "ui:widget": "select" },
    awsMode: { "ui:widget": "select" },
    awsSecretAccessKey: { "ui:widget": "password" },
    azureMode: {
      "ui:widget": "select",
      "ui:enumNames": ["Implicit (DefaultAzureCredential)", "Developer Implicit", "Explicit Client Secret"],
    },
    azureClientSecret: { "ui:widget": "password" },
};

export const defaultValues = {
    authType: "passthrough",
    keyType: "inline",
    gcpTokenType: "idToken",
    awsMode: "ambient",
    azureMode: "implicit",
};

export function transformForForm(data: unknown): unknown {
    const d = data as any;
    if ("passthrough" in d) return { authType: "passthrough" };
    if ("key" in d) {
        const k = d.key;
        return typeof k === "string"
        ? { authType: "key", keyType: "inline", keyValue: k }
        : { authType: "key", keyType: "file", keyValue: k.file };
    }
    if ("gcp" in d) {
        const g = d.gcp;
        return { authType: "gcp", gcpTokenType: g.type ?? "accessToken", gcpAudience: g.audience
    ?? "" };
    }
    if ("aws" in d) {
        const a = d.aws;
        if (!a.accessKeyId) return { authType: "aws", awsMode: "ambient" };
        return {
        authType: "aws", awsMode: "explicit",
        awsAccessKeyId: a.accessKeyId, awsSecretAccessKey: a.secretAccessKey,
        awsRegion: a.region ?? "", awsSessionToken: a.sessionToken ?? "",
        };
    }
    if ("azure" in d) {
        const az = d.azure;
        if ("implicit" in az) return { authType: "azure", azureMode: "implicit" };
        if ("developerImplicit" in az) return { authType: "azure", azureMode: "developerImplicit"
    };
        if ("explicitConfig" in az) {
        const cs = az.explicitConfig?.clientSecret ?? {};
        return {
            authType: "azure", azureMode: "explicitClientSecret",
            azureTenantId: cs.tenant_id ?? "", azureClientId: cs.client_id ?? "",
            azureClientSecret: cs.client_secret ?? "",
        };
        }
    }
    return { authType: "passthrough" };
}

export function transformBeforeSubmit(data: unknown): unknown {
    const d = data as any;
    switch (d.authType) {
        case "passthrough": return { passthrough: {} };
        case "key":
        return { key: d.keyType === "file" ? { file: d.keyValue } : d.keyValue };
        case "gcp":
        return d.gcpTokenType === "idToken"
            ? { gcp: { type: "idToken", ...(d.gcpAudience ? { audience: d.gcpAudience } : {}) } }
            : { gcp: { type: "accessToken" } };
        case "aws":
        return d.awsMode === "ambient"
            ? { aws: {} }
            : { aws: { accessKeyId: d.awsAccessKeyId, secretAccessKey: d.awsSecretAccessKey,
                ...(d.awsRegion ? { region: d.awsRegion } : {}),
                ...(d.awsSessionToken ? { sessionToken: d.awsSessionToken } : {}),
            }};
        case "azure":
        if (d.azureMode === "implicit") return { azure: { implicit: {} } };
        if (d.azureMode === "developerImplicit") return { azure: { developerImplicit: {} } };
        return { azure: { explicitConfig: { clientSecret: {
            tenant_id: d.azureTenantId, client_id: d.azureClientId, client_secret:
    d.azureClientSecret,
        }}}};
        default: return { passthrough: {} };
    }
}