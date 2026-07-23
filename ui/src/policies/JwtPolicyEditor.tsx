import { useState } from "react";
import { DatabaseZap, FileKey2, Globe2, ShieldCheck } from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { Field, FieldGroup, StatusBanner } from "../components/Primitives";
import {
  authorizationLocationFrom,
  authorizationLocationToValue,
  CredentialLocationSetting,
} from "./AuthorizationLocation";
import { ListEditor } from "./ListEditor";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import type { JwtPolicy } from "./types";
import { cleanEmpty, toText } from "./policyUtils";
import type { JWTValidationOptions, LocalJwtConfig } from "../gateway-config";

type JwtMode = "strict" | "optional" | "permissive";
type JwksMode = "file" | "url" | "inline";

type JwtDraft = Omit<JwtPolicy, "location" | "jwtValidationOptions"> & {
  location?: unknown;
  jwtValidationOptions?: {
    requiredClaims?: string[];
  };
};

type JwtFieldErrors = Partial<
  Record<"issuer" | "jwksUrl" | "jwksFile" | "jwksInline", string>
>;

const commonClaims = ["exp", "nbf", "aud", "iss", "sub"] as const;

const modeOptions: Array<{
  value: JwtMode;
  label: string;
  description: string;
}> = [
  {
    value: "strict",
    label: "Strict",
    description: "Reject requests that do not carry a valid token.",
  },
  {
    value: "optional",
    label: "Optional",
    description: "Validate a token when one is present.",
  },
  {
    value: "permissive",
    label: "Permissive",
    description: "Keep serving traffic while surfacing JWT data when possible.",
  },
];

const jwksOptions: Array<{
  value: JwksMode;
  label: string;
  description: string;
}> = [
  {
    value: "url",
    label: "Remote URL",
    description: "Fetch signing keys from the issuer JWKS endpoint.",
  },
  {
    value: "file",
    label: "Local file",
    description: "Read signing keys from a file on the gateway host.",
  },
  {
    value: "inline",
    label: "Inline JSON",
    description: "Paste a JWKS document directly into the policy.",
  },
];

export function JwtPolicyEditor(props: {
  formId?: string;
  jwt: JwtDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (jwt: JwtPolicy) => void;
}) {
  const initialJwks = props.jwt?.jwks;
  const initialJwksMode: JwksMode =
    isRecord(initialJwks) && typeof initialJwks.url === "string"
      ? "url"
      : isRecord(initialJwks) && typeof initialJwks.file === "string"
        ? "file"
        : initialJwks
          ? "inline"
          : "url";

  const [mode, setMode] = useState<JwtMode>(props.jwt?.mode ?? "strict");
  const [location, setLocation] = useState(() =>
    authorizationLocationFrom(props.jwt?.location),
  );
  const [issuer, setIssuer] = useState(props.jwt?.issuer ?? "");
  const [audiences, setAudiences] = useState(props.jwt?.audiences ?? []);
  const [jwksMode, setJwksMode] = useState<JwksMode>(initialJwksMode);
  const [jwksFile, setJwksFile] = useState(
    isRecord(initialJwks) && typeof initialJwks.file === "string"
      ? initialJwks.file
      : "",
  );
  const [jwksUrl, setJwksUrl] = useState(
    isRecord(initialJwks) && typeof initialJwks.url === "string"
      ? initialJwks.url
      : "",
  );
  const [jwksInline, setJwksInline] = useState(
    initialJwksMode === "inline"
      ? toText(initialJwks ?? { keys: [] })
      : '{\n  "keys": []\n}',
  );
  const [requiredClaims, setRequiredClaims] = useState(
    () => new Set(props.jwt?.jwtValidationOptions?.requiredClaims ?? ["exp"]),
  );
  const [fieldErrors, setFieldErrors] = useState<JwtFieldErrors>({});
  const [error, setError] = useState<string | null>(null);

  const preview = safeBuildJwtPolicy();

  function buildJwtPolicy() {
    return cleanEmpty({
      mode,
      location: authorizationLocationToValue(location),
      issuer,
      audiences,
      jwks: buildJwks(),
      jwtValidationOptions: {
        requiredClaims: Array.from(requiredClaims),
      },
    }) as JwtPolicy;
  }

  function buildJwks() {
    if (jwksMode === "file")
      return jwksFile.trim() ? { file: jwksFile.trim() } : undefined;
    if (jwksMode === "url")
      return jwksUrl.trim() ? { url: jwksUrl.trim() } : undefined;
    if (!jwksInline.trim()) return undefined;
    JSON.parse(jwksInline);
    return jwksInline;
  }

  function safeBuildJwtPolicy() {
    try {
      return buildJwtPolicy();
    } catch {
      return {
        error: "Inline JWKS must be valid JSON before it can be saved.",
      };
    }
  }

  function save() {
    try {
      setError(null);
      const validationErrors = validateJwtPolicy();
      setFieldErrors(validationErrors);
      if (Object.keys(validationErrors).length) {
        setError("Fix the highlighted fields before saving.");
        return;
      }
      props.onSave(buildJwtPolicy());
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid JWT policy");
    }
  }

  function validateJwtPolicy() {
    const errors: JwtFieldErrors = {};
    if (!issuer.trim()) errors.issuer = "Issuer is required.";
    if (jwksMode === "url" && !jwksUrl.trim())
      errors.jwksUrl = "JWKS URL is required.";
    if (jwksMode === "file" && !jwksFile.trim())
      errors.jwksFile = "JWKS file is required.";
    if (jwksMode === "inline") {
      if (!jwksInline.trim()) {
        errors.jwksInline = "Inline JWKS is required.";
      } else {
        try {
          JSON.parse(jwksInline);
        } catch {
          errors.jwksInline = "Inline JWKS must be valid JSON.";
        }
      }
    }
    return errors;
  }

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      <PolicySection
        icon={<ShieldCheck size={17} />}
        title="Enforcement"
        description="Choose how the gateway behaves when a request has no token or a token cannot be verified."
      >
        <FieldGroup
          label="Validation mode"
          tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "mode")}
        >
          <div className="option-card-grid">
            {modeOptions.map((option) => (
              <button
                className={
                  mode === option.value ? "option-card active" : "option-card"
                }
                type="button"
                key={option.value}
                onClick={() => setMode(option.value)}
              >
                <strong>{option.label}</strong>
                <span>{option.description}</span>
              </button>
            ))}
          </div>
        </FieldGroup>
      </PolicySection>

      <PolicySection
        icon={
          jwksMode === "url" ? (
            <DatabaseZap size={17} />
          ) : (
            <FileKey2 size={17} />
          )
        }
        title="Signing keys"
        description="Configure the JWKS source used to verify token signatures."
      >
        <FieldGroup
          label="JWKS source"
          tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "jwks")}
        >
          <div className="option-card-grid">
            {jwksOptions.map((option) => (
              <button
                className={
                  jwksMode === option.value
                    ? "option-card active"
                    : "option-card"
                }
                type="button"
                key={option.value}
                onClick={() => {
                  setJwksMode(option.value);
                  clearJwksErrors();
                }}
              >
                <strong>{option.label}</strong>
                <span>{option.description}</span>
              </button>
            ))}
          </div>
        </FieldGroup>
        {jwksMode === "file" ? (
          <Field
            label="JWKS file"
            tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "jwks")}
            className={fieldErrors.jwksFile ? "invalid" : undefined}
            hint={fieldErrors.jwksFile}
          >
            <input
              value={jwksFile}
              aria-invalid={Boolean(fieldErrors.jwksFile)}
              onChange={(event) => {
                setJwksFile(event.target.value);
                clearFieldError("jwksFile");
              }}
              placeholder="./manifests/jwt/pub-key"
            />
          </Field>
        ) : jwksMode === "url" ? (
          <Field
            label="JWKS URL"
            tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "jwks")}
            className={fieldErrors.jwksUrl ? "invalid" : undefined}
            hint={fieldErrors.jwksUrl}
          >
            <input
              value={jwksUrl}
              aria-invalid={Boolean(fieldErrors.jwksUrl)}
              onChange={(event) => {
                setJwksUrl(event.target.value);
                clearFieldError("jwksUrl");
              }}
              placeholder="https://issuer.example.com/.well-known/jwks.json"
            />
          </Field>
        ) : (
          <FieldGroup
            label="Inline JWKS"
            tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "jwks")}
            className={fieldErrors.jwksInline ? "invalid" : undefined}
            hint={fieldErrors.jwksInline}
          >
            <MiniMonacoEditor
              language="json"
              value={jwksInline}
              invalid={Boolean(fieldErrors.jwksInline)}
              onChange={(value) => {
                setJwksInline(value);
                clearFieldError("jwksInline");
              }}
            />
          </FieldGroup>
        )}
      </PolicySection>

      <PolicySection
        icon={<Globe2 size={17} />}
        title="Token validation"
        description="Restrict accepted tokens by issuer, audience, and required claims."
      >
        <Field
          label="Issuer"
          tooltip={props.help.field<LocalJwtConfig>("LocalJwtConfig", "issuer")}
          className={fieldErrors.issuer ? "invalid" : undefined}
          hint={fieldErrors.issuer}
        >
          <input
            value={issuer}
            aria-invalid={Boolean(fieldErrors.issuer)}
            onChange={(event) => {
              setIssuer(event.target.value);
              clearFieldError("issuer");
            }}
            placeholder="https://issuer.example.com"
          />
        </Field>

        <ListEditor
          label="Audiences"
          tooltip={props.help.field<LocalJwtConfig>(
            "LocalJwtConfig",
            "audiences",
          )}
          values={audiences}
          placeholder="api://gateway"
          emptyText="No audience restriction configured."
          onChange={setAudiences}
        />

        <FieldGroup
          label="Required claims"
          tooltip={props.help.field<JWTValidationOptions>(
            "JWTValidationOptions",
            "requiredClaims",
          )}
        >
          <div className="method-grid">
            {commonClaims.map((claim) => (
              <button
                className={
                  requiredClaims.has(claim)
                    ? "choice-pill active"
                    : "choice-pill"
                }
                type="button"
                key={claim}
                onClick={() =>
                  setRequiredClaims((current) => toggleClaim(current, claim))
                }
              >
                {claim}
              </button>
            ))}
          </div>
        </FieldGroup>
      </PolicySection>

      <CredentialLocationSetting
        value={location}
        help={props.help}
        defaultDescription="Default: Authorization: Bearer token"
        description="Override where this policy reads the JWT from."
        onChange={setLocation}
      />

      <ResultingYaml value={preview} />

      {error ? (
        <StatusBanner state="bad" title="Invalid JWT policy">
          {error}
        </StatusBanner>
      ) : null}
    </form>
  );

  function clearFieldError(field: keyof JwtFieldErrors) {
    setFieldErrors((current) => {
      if (!current[field]) return current;
      const next = { ...current };
      delete next[field];
      return next;
    });
    setError(null);
  }

  function clearJwksErrors() {
    setFieldErrors((current) => {
      const next = { ...current };
      delete next.jwksUrl;
      delete next.jwksFile;
      delete next.jwksInline;
      return next;
    });
    setError(null);
  }
}

function toggleClaim(values: Set<string>, value: string) {
  const next = new Set(values);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  return next;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}
