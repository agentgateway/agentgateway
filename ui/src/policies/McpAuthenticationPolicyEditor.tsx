import { useState } from "react";
import {
  DatabaseZap,
  FileKey2,
  Globe2,
  KeyRound,
  ShieldCheck,
} from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { EnumSelector } from "../components/EnumSelector";
import { Field, FieldGroup, StatusBanner } from "../components/Primitives";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { ListEditor } from "./ListEditor";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty, parseYamlText, toYamlMappingText } from "./policyUtils";
import type { McpAuthenticationDraft } from "./types";
import type { LocalMcpAuthentication } from "../gateway-config";

type AuthMode = "strict" | "optional" | "permissive";
type JwksMode = "url" | "file" | "inline";

type FieldErrors = Partial<Record<"issuer" | "jwks" | "metadata", string>>;

export function McpAuthenticationPolicyEditor(props: {
  formId?: string;
  authentication: McpAuthenticationDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (authentication: McpAuthenticationDraft) => void;
}) {
  const initialJwks = props.authentication?.jwks;
  const initialJwksMode: JwksMode =
    isRecord(initialJwks) && typeof initialJwks.file === "string"
      ? "file"
      : isRecord(initialJwks) && typeof initialJwks.url === "string"
        ? "url"
        : "inline";
  const [mode, setMode] = useState<AuthMode>(
    props.authentication?.mode ?? "strict",
  );
  const [issuer, setIssuer] = useState(props.authentication?.issuer ?? "");
  const [audiences, setAudiences] = useState(
    props.authentication?.audiences ?? [],
  );
  const [clientId, setClientId] = useState(
    props.authentication?.clientId ?? "",
  );
  const [jwksMode, setJwksMode] = useState<JwksMode>(initialJwksMode);
  const [jwksValue, setJwksValue] = useState(() =>
    jwksText(initialJwks, initialJwksMode),
  );
  const [resourceMetadata, setResourceMetadata] = useState(() =>
    toYamlMappingText(props.authentication?.resourceMetadata),
  );
  const [errors, setErrors] = useState<FieldErrors>({});
  const [error, setError] = useState<string | null>(null);
  const preview = safeBuild();

  function build() {
    return cleanEmpty({
      mode,
      issuer: issuer.trim(),
      audiences,
      clientId: clientId.trim() || null,
      jwks: buildJwks(jwksMode, jwksValue),
      resourceMetadata: resourceMetadata.trim()
        ? parseYamlText(resourceMetadata)
        : {},
    }) as McpAuthenticationDraft;
  }

  function safeBuild() {
    try {
      return build();
    } catch {
      return {
        error:
          "Resource metadata must be YAML and inline JWKS must be valid JSON.",
      };
    }
  }

  function save() {
    try {
      const nextErrors: FieldErrors = {};
      if (!issuer.trim()) nextErrors.issuer = "Issuer is required.";
      if (!jwksValue.trim()) nextErrors.jwks = "JWKS source is required.";
      if (jwksMode === "inline") JSON.parse(jwksValue);
      if (resourceMetadata.trim()) {
        const parsed = parseYamlText(resourceMetadata);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
          nextErrors.metadata = "Resource metadata must be a YAML mapping.";
      }
      setErrors(nextErrors);
      if (Object.keys(nextErrors).length) {
        setError("Fix the highlighted fields before saving.");
        return;
      }
      setError(null);
      props.onSave(build());
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : "Invalid MCP authentication policy",
      );
    }
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
        description="Control whether MCP requests must present a valid JWT."
      >
        <FieldGroup
          label="Validation mode"
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "mode",
          )}
        >
          <EnumSelector
            ariaLabel="Validation mode"
            value={mode}
            schema={props.help.node(["$defs", "McpAuthenticationMode"])}
            labels={{
              strict: "Strict",
              optional: "Optional",
              permissive: "Permissive",
            }}
            onChange={(value) => setMode(value as AuthMode)}
          />
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
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "jwks",
          )}
        >
          <div className="segmented-control compact">
            <button
              className={jwksMode === "url" ? "active" : ""}
              type="button"
              onClick={() => {
                setJwksMode("url");
                setJwksValue("");
              }}
            >
              Remote URL
            </button>
            <button
              className={jwksMode === "file" ? "active" : ""}
              type="button"
              onClick={() => {
                setJwksMode("file");
                setJwksValue("");
              }}
            >
              Local file
            </button>
            <button
              className={jwksMode === "inline" ? "active" : ""}
              type="button"
              onClick={() => {
                setJwksMode("inline");
                setJwksValue('{\n  "keys": []\n}');
              }}
            >
              Inline JSON
            </button>
          </div>
        </FieldGroup>
        {jwksMode === "inline" ? (
          <FieldGroup
            label="Inline JWKS"
            className={errors.jwks ? "invalid" : undefined}
            hint={errors.jwks}
          >
            <MiniMonacoEditor
              language="json"
              value={jwksValue}
              invalid={Boolean(errors.jwks)}
              onChange={setJwksValue}
            />
          </FieldGroup>
        ) : (
          <Field
            label={jwksMode === "url" ? "JWKS URL" : "JWKS file"}
            className={errors.jwks ? "invalid" : undefined}
            hint={errors.jwks}
          >
            <input
              value={jwksValue}
              onChange={(event) => setJwksValue(event.target.value)}
              placeholder={
                jwksMode === "url"
                  ? "https://issuer.example.com/.well-known/jwks.json"
                  : "$HOME/.secrets/jwks.json"
              }
            />
          </Field>
        )}
      </PolicySection>

      <PolicySection
        icon={<Globe2 size={17} />}
        title="Token validation"
        description="Restrict accepted MCP tokens by issuer and audience."
      >
        <Field
          label="Issuer"
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "issuer",
          )}
          className={errors.issuer ? "invalid" : undefined}
          hint={errors.issuer}
        >
          <input
            value={issuer}
            aria-invalid={Boolean(errors.issuer)}
            onChange={(event) => setIssuer(event.target.value)}
            placeholder="https://issuer.example.com"
          />
        </Field>
        <ListEditor
          label="Audiences"
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "audiences",
          )}
          values={audiences}
          placeholder="mcp://gateway"
          emptyText="No audience restriction configured."
          onChange={setAudiences}
        />
        <Field
          label="Client ID"
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "clientId",
          )}
        >
          <input
            value={clientId ?? ""}
            onChange={(event) => setClientId(event.target.value)}
            placeholder="optional OAuth client ID"
          />
        </Field>
      </PolicySection>

      <PolicySection
        icon={<KeyRound size={17} />}
        title="Protected resource metadata"
        description="Metadata advertised to MCP clients for OAuth protected resources."
      >
        <FieldGroup
          label="Resource metadata YAML"
          tooltip={props.help.field<LocalMcpAuthentication>(
            "LocalMcpAuthentication",
            "resourceMetadata",
          )}
          className={errors.metadata ? "invalid" : undefined}
          hint={errors.metadata}
        >
          <MiniMonacoEditor
            language="yaml"
            value={resourceMetadata}
            invalid={Boolean(errors.metadata)}
            onChange={setResourceMetadata}
          />
        </FieldGroup>
      </PolicySection>

      <ResultingYaml value={preview} />
      {error ? (
        <StatusBanner state="bad" title="Invalid MCP authentication policy">
          {error}
        </StatusBanner>
      ) : null}
    </form>
  );
}

function buildJwks(mode: JwksMode, value: string) {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (mode === "url") return { url: trimmed };
  if (mode === "file") return { file: trimmed };
  JSON.parse(trimmed);
  return trimmed;
}

function jwksText(value: unknown, mode: JwksMode) {
  if (mode === "url" && isRecord(value) && typeof value.url === "string")
    return value.url;
  if (mode === "file" && isRecord(value) && typeof value.file === "string")
    return value.file;
  if (typeof value === "string") return value;
  return '{\n  "keys": []\n}';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}
