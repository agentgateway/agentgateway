import { PolicyType } from "./policy-constants";

export function getDefaultPolicyData(type: PolicyType) {
  switch (type) {
    case "jwtAuth":
      return {
        issuer: "",
        audiences: [],
        jwks: { url: "" },
      };
    case "mcpAuthentication":
      return {
        issuer: "",
        scopes: [],
        audience: "",
        provider: null,
      };
    case "mcpAuthorization":
      return {
        rules: [],
      };
    case "cors":
      return {
        allowCredentials: false,
        allowHeaders: [],
        allowMethods: [],
        allowOrigins: [],
        exposeHeaders: [],
        maxAge: null,
      };
    case "backendTLS":
      return {
        cert: null,
        key: null,
        root: null,
        insecure: false,
        insecureHost: false,
      };
    case "backendAuth":
      return {
        passthrough: {},
      };
    case "localRateLimit":
      return [];
    case "remoteRateLimit":
      return {
        target: "",
        descriptors: {},
      };
    case "timeout":
      return {
        requestTimeout: null,
        backendRequestTimeout: null,
      };
    case "retry":
      return {
        attempts: 1,
        backoff: null,
        codes: [],
      };
    case "requestHeaderModifier":
    case "responseHeaderModifier":
      return {
        add: [],
        set: [],
        remove: [],
      };
    case "requestRedirect":
      return {
        scheme: null,
        authority: null,
        path: null,
        status: null,
      };
    case "urlRewrite":
      return {
        authority: null,
        path: null,
      };
    case "directResponse":
      return {
        body: "",
        status: 200,
      };
    case "extAuthz":
      return {
        target: "",
        context: {},
      };
    case "ai":
      return {
        provider: null,
        hostOverride: null,
        pathOverride: null,
        promptGuard: null,
      };
    case "a2a":
      return {};
    default:
      return {};
  }
}
