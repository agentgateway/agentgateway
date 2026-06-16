import { useState } from "react";
import { Fingerprint, Globe2, KeyRound, Save } from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { Dropdown, Field, FieldGroup, StatusBanner } from "../components/Primitives";
import { ListEditor } from "./ListEditor";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty, isRecord } from "./policyUtils";
import type { OidcDraft } from "./types";

type ProviderMode = "discovery" | "explicit";
type SourceMode = "none" | "url" | "file" | "inline";
type TokenEndpointAuth = "clientSecretBasic" | "clientSecretPost";

type SourceDraft = {
  mode: SourceMode;
  value: string;
};

type OidcFieldErrors = Partial<Record<
  "issuer" | "clientId" | "clientSecret" | "redirectURI" | "discovery" | "authorizationEndpoint" | "tokenEndpoint" | "jwks",
  string
>>;

const providerModes: Array<{ value: ProviderMode; label: string; description: string }> = [
  { value: "discovery", label: "Discovery", description: "Use the issuer metadata endpoint unless an override is provided." },
  { value: "explicit", label: "Explicit endpoints", description: "Manually provide authorization, token, and signing-key metadata." },
];

const sourceModes: Array<{ value: SourceMode; label: string }> = [
  { value: "none", label: "None" },
  { value: "url", label: "Remote URL" },
  { value: "file", label: "Local file" },
  { value: "inline", label: "Inline JSON" },
];

const tokenEndpointAuthOptions: Array<{ value: TokenEndpointAuth; label: string; searchText: string }> = [
  { value: "clientSecretBasic", label: "Client secret basic", searchText: "clientSecretBasic basic" },
  { value: "clientSecretPost", label: "Client secret post", searchText: "clientSecretPost post" },
];

const issuerSuggestions = [
  { label: "Google", value: "https://accounts.google.com" },
  { label: "Microsoft Entra", value: "https://login.microsoftonline.com/{tenant}/v2.0" },
  { label: "Okta", value: "https://{yourOktaDomain}/oauth2/default" },
  { label: "Auth0", value: "https://{yourDomain}.auth0.com/" },
  { label: "Keycloak", value: "http://localhost:7080/realms/{realm}" },
];

export function OidcPolicyEditor(props: {
  oidc: OidcDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (oidc: OidcDraft) => void;
}) {
  const hasExplicitProvider = Boolean(props.oidc?.authorizationEndpoint || props.oidc?.tokenEndpoint || props.oidc?.jwks);
  const [providerMode, setProviderMode] = useState<ProviderMode>(hasExplicitProvider ? "explicit" : "discovery");
  const [issuer, setIssuer] = useState(props.oidc?.issuer ?? "");
  const [clientId, setClientId] = useState(props.oidc?.clientId ?? "");
  const [clientSecret, setClientSecret] = useState(props.oidc?.clientSecret ?? "");
  const [redirectURI, setRedirectURI] = useState(props.oidc?.redirectURI ?? "");
  const [scopes, setScopes] = useState(props.oidc?.scopes ?? []);
  const [discovery, setDiscovery] = useState<SourceDraft>(() => sourceFrom(props.oidc?.discovery, "none"));
  const [authorizationEndpoint, setAuthorizationEndpoint] = useState(props.oidc?.authorizationEndpoint ?? "");
  const [tokenEndpoint, setTokenEndpoint] = useState(props.oidc?.tokenEndpoint ?? "");
  const [tokenEndpointAuth, setTokenEndpointAuth] = useState<TokenEndpointAuth>(props.oidc?.tokenEndpointAuth ?? "clientSecretBasic");
  const [jwks, setJwks] = useState<SourceDraft>(() => sourceFrom(props.oidc?.jwks, "url"));
  const [fieldErrors, setFieldErrors] = useState<OidcFieldErrors>({});
  const [error, setError] = useState<string | null>(null);

  const preview = buildOidcPolicy();

  function buildOidcPolicy() {
    return cleanEmpty({
      issuer,
      discovery: providerMode === "discovery" ? sourceToConfig(discovery) : undefined,
      authorizationEndpoint: providerMode === "explicit" ? authorizationEndpoint : undefined,
      tokenEndpoint: providerMode === "explicit" ? tokenEndpoint : undefined,
      tokenEndpointAuth: providerMode === "explicit" ? tokenEndpointAuth : undefined,
      jwks: providerMode === "explicit" ? sourceToConfig(jwks) : undefined,
      clientId,
      clientSecret,
      redirectURI,
      scopes,
    }) as OidcDraft;
  }

  function save() {
    setError(null);
    const validationErrors = validateOidcPolicy();
    setFieldErrors(validationErrors);
    if (Object.keys(validationErrors).length) {
      setError("Fix the highlighted fields before saving.");
      return;
    }
    props.onSave(buildOidcPolicy());
  }

  function validateOidcPolicy() {
    const errors: OidcFieldErrors = {};
    if (!issuer.trim()) errors.issuer = "Issuer is required.";
    if (!clientId.trim()) errors.clientId = "Client ID is required.";
    if (!clientSecret.trim()) errors.clientSecret = "Client secret is required.";
    if (!redirectURI.trim()) errors.redirectURI = "Redirect URI is required.";
    if (providerMode === "discovery") {
      const discoveryError = validateSource(discovery, false);
      if (discoveryError) errors.discovery = discoveryError;
    } else {
      if (!authorizationEndpoint.trim()) errors.authorizationEndpoint = "Authorization endpoint is required.";
      if (!tokenEndpoint.trim()) errors.tokenEndpoint = "Token endpoint is required.";
      const jwksError = validateSource(jwks, true);
      if (jwksError) errors.jwks = jwksError;
    }
    return errors;
  }

  return (
    <div className="policy-editor-stack">
      <PolicySection
        icon={<Fingerprint size={17} />}
        title="Provider"
        description="Configure where browser login starts and how returned ID tokens are validated."
      >
        <Field label="Issuer" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "issuer"])} className={fieldErrors.issuer ? "invalid" : undefined} hint={fieldErrors.issuer}>
          <input
            value={issuer}
            aria-invalid={Boolean(fieldErrors.issuer)}
            onChange={(event) => {
              setIssuer(event.target.value);
              clearFieldError("issuer");
            }}
            placeholder="https://issuer.example.com"
          />
          <div className="suggestion-row">
            {issuerSuggestions.map((suggestion) => (
              <button
                className="table-action"
                type="button"
                key={suggestion.label}
                onClick={() => {
                  setIssuer(suggestion.value);
                  clearFieldError("issuer");
                }}
              >
                {suggestion.label}
              </button>
            ))}
          </div>
        </Field>

        <FieldGroup label="Provider metadata">
          <div className="option-card-grid">
            {providerModes.map((mode) => (
              <button className={providerMode === mode.value ? "option-card active" : "option-card"} type="button" key={mode.value} onClick={() => {
                setProviderMode(mode.value);
                clearProviderErrors();
              }}>
                <strong>{mode.label}</strong>
                <span>{mode.description}</span>
              </button>
            ))}
          </div>
        </FieldGroup>

        {providerMode === "discovery" ? (
          <SourceEditor
            label="Discovery override"
            tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "discovery"])}
            value={discovery}
            fieldError={fieldErrors.discovery}
            optionalText="Default: issuer + /.well-known/openid-configuration"
            onChange={(value) => {
              setDiscovery(value);
              clearFieldError("discovery");
            }}
          />
        ) : (
          <>
            <Field label="Authorization endpoint" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "authorizationEndpoint"])} className={fieldErrors.authorizationEndpoint ? "invalid" : undefined} hint={fieldErrors.authorizationEndpoint}>
              <input
                value={authorizationEndpoint}
                aria-invalid={Boolean(fieldErrors.authorizationEndpoint)}
                onChange={(event) => {
                  setAuthorizationEndpoint(event.target.value);
                  clearFieldError("authorizationEndpoint");
                }}
                placeholder="https://issuer.example.com/oauth2/v1/authorize"
              />
            </Field>
            <Field label="Token endpoint" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "tokenEndpoint"])} className={fieldErrors.tokenEndpoint ? "invalid" : undefined} hint={fieldErrors.tokenEndpoint}>
              <input
                value={tokenEndpoint}
                aria-invalid={Boolean(fieldErrors.tokenEndpoint)}
                onChange={(event) => {
                  setTokenEndpoint(event.target.value);
                  clearFieldError("tokenEndpoint");
                }}
                placeholder="https://issuer.example.com/oauth2/v1/token"
              />
            </Field>
            <FieldGroup label="Token endpoint auth" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "tokenEndpointAuth"])}>
              <Dropdown
                value={tokenEndpointAuth}
                ariaLabel="Token endpoint auth"
                options={tokenEndpointAuthOptions}
                onChange={(value) => setTokenEndpointAuth(value as TokenEndpointAuth)}
              />
            </FieldGroup>
            <SourceEditor
              label="JWKS"
              tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "jwks"])}
              value={jwks}
              fieldError={fieldErrors.jwks}
              onChange={(value) => {
                setJwks(value);
                clearFieldError("jwks");
              }}
            />
          </>
        )}
      </PolicySection>

      <PolicySection
        icon={<KeyRound size={17} />}
        title="Client"
        description="Identify the OAuth2 client used by the gateway during the authorization code flow."
      >
        <Field label="Client ID" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "clientId"])} className={fieldErrors.clientId ? "invalid" : undefined} hint={fieldErrors.clientId}>
          <input
            value={clientId}
            aria-invalid={Boolean(fieldErrors.clientId)}
            onChange={(event) => {
              setClientId(event.target.value);
              clearFieldError("clientId");
            }}
            placeholder="agentgateway-browser"
          />
        </Field>
        <Field label="Client secret" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "clientSecret"])} className={fieldErrors.clientSecret ? "invalid" : undefined} hint={fieldErrors.clientSecret}>
          <input
            type="password"
            value={clientSecret}
            aria-invalid={Boolean(fieldErrors.clientSecret)}
            onChange={(event) => {
              setClientSecret(event.target.value);
              clearFieldError("clientSecret");
            }}
            placeholder="OAuth2 client secret"
          />
        </Field>
        <Field label="Redirect URI" tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "redirectURI"])} className={fieldErrors.redirectURI ? "invalid" : undefined} hint={fieldErrors.redirectURI}>
          <input
            value={redirectURI}
            aria-invalid={Boolean(fieldErrors.redirectURI)}
            onChange={(event) => {
              setRedirectURI(event.target.value);
              clearFieldError("redirectURI");
            }}
            placeholder="http://localhost:4000/oauth/callback"
          />
        </Field>
      </PolicySection>

      <PolicySection
        icon={<Globe2 size={17} />}
        title="Scopes"
        description="Request extra OAuth2 scopes. The gateway always includes openid."
      >
        <ListEditor
          label="Additional scopes"
          tooltip={props.help.description(["$defs", "LocalOidcConfig", "properties", "scopes"])}
          values={scopes}
          placeholder="profile"
          emptyText="No additional scopes configured."
          suggestions={["profile", "email", "offline_access"]}
          onChange={setScopes}
        />
      </PolicySection>

      <ResultingYaml value={preview} />

      {error ? <StatusBanner state="bad" title="Invalid OIDC policy">{error}</StatusBanner> : null}
      <button className="button primary" type="button" disabled={props.saving} onClick={save}>
        <Save size={16} />
        Save OIDC
      </button>
    </div>
  );

  function clearFieldError(field: keyof OidcFieldErrors) {
    setFieldErrors((current) => {
      if (!current[field]) return current;
      const next = { ...current };
      delete next[field];
      return next;
    });
    setError(null);
  }

  function clearProviderErrors() {
    setFieldErrors((current) => {
      const next = { ...current };
      delete next.discovery;
      delete next.authorizationEndpoint;
      delete next.tokenEndpoint;
      delete next.jwks;
      return next;
    });
    setError(null);
  }
}

function SourceEditor(props: {
  label: string;
  value: SourceDraft;
  onChange: (value: SourceDraft) => void;
  fieldError?: string;
  tooltip?: string;
  optionalText?: string;
}) {
  const activeMode = props.value.mode;
  const placeholder = activeMode === "file"
    ? "./manifests/oidc/provider.json"
    : activeMode === "url"
      ? "https://issuer.example.com/.well-known/openid-configuration"
      : "{\n  \"keys\": []\n}";

  return (
    <FieldGroup label={props.label} tooltip={props.tooltip}>
      <div className={props.fieldError ? "oidc-source invalid" : "oidc-source"}>
        <Dropdown
          value={activeMode}
          ariaLabel={props.label}
          options={sourceModes}
          onChange={(mode) => props.onChange({ mode: mode as SourceMode, value: mode === "none" ? "" : props.value.value })}
        />
        {activeMode === "none" ? (
          <div className="empty-inline">{props.optionalText ?? "No source configured."}</div>
        ) : activeMode === "inline" ? (
          <textarea
            className="mono-input"
            rows={6}
            value={props.value.value}
            aria-invalid={Boolean(props.fieldError)}
            onChange={(event) => props.onChange({ ...props.value, value: event.target.value })}
            placeholder={placeholder}
          />
        ) : (
          <input
            value={props.value.value}
            aria-invalid={Boolean(props.fieldError)}
            onChange={(event) => props.onChange({ ...props.value, value: event.target.value })}
            placeholder={placeholder}
          />
        )}
        {props.fieldError ? <small>{props.fieldError}</small> : null}
      </div>
    </FieldGroup>
  );
}

function sourceFrom(value: unknown, emptyMode: SourceMode): SourceDraft {
  if (isRecord(value) && typeof value.url === "string") return { mode: "url", value: value.url };
  if (isRecord(value) && typeof value.file === "string") return { mode: "file", value: value.file };
  if (typeof value === "string") return { mode: "inline", value };
  return { mode: emptyMode, value: "" };
}

function sourceToConfig(source: SourceDraft) {
  const value = source.value.trim();
  if (source.mode === "none" || !value) return undefined;
  if (source.mode === "url") return { url: value };
  if (source.mode === "file") return { file: value };
  return value;
}

function validateSource(source: SourceDraft, required: boolean) {
  if (source.mode === "none") return required ? "Source is required." : undefined;
  if (!source.value.trim()) return "Value is required.";
  return undefined;
}
