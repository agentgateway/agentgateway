import { useState } from "react";
import { FileCode2, ShieldCheck } from "lucide-react";
import {
  EnumSelector,
  type EnumSelectorOption,
} from "../../components/EnumSelector";
import {
  SchemaYamlEditor,
  parseSchemaYamlEditorValue,
} from "../../components/SchemaYamlEditor";
import { FieldGroup, StatusBanner } from "../../components/Primitives";
import type { SchemaHelp } from "../../schemaHelp";
import type { BackendAuth } from "../../gateway-config";
import { toYamlText } from "../policyUtils";
import { ResultingYaml } from "../ResultingYaml";
import {
  PassthroughFields,
  emptyPassthroughDraft,
  passthroughDraftFromValue,
  passthroughDraftToValue,
  type PassthroughDraft,
} from "./passthrough";

type AuthKind = "passthrough" | "raw";

const authKindOptions: Array<EnumSelectorOption<AuthKind>> = [
  {
    value: "passthrough",
    label: "Passthrough",
    description: "Forward the validated incoming JWT to the backend.",
    icon: <ShieldCheck size={16} />,
  },
  {
    // "Raw YAML" is a permanent option, not just a fallback for shapes the
    // structured editors can't handle: it keeps schema-autocompleted YAML
    // authoring available for every method that doesn't have a structured
    // editor yet (key, gcp, aws, azure, copilot, oauth, crossAppAccess), and
    // stays as the escape hatch even once those land.
    value: "raw",
    label: "Raw YAML",
    description:
      "Edit the policy YAML directly, with schema autocompletion — for methods without a structured editor yet: key, AWS, GCP, Azure, Copilot, OAuth, cross-app access.",
    icon: <FileCode2 size={16} />,
  },
];

export function BackendAuthPolicyEditor(props: {
  formId?: string;
  backendAuth: BackendAuth | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: BackendAuth) => void;
}) {
  const [initial] = useState(() => draftFromBackendAuth(props.backendAuth));
  const [kind, setKind] = useState<AuthKind>(initial.kind);
  const [passthrough, setPassthrough] = useState<PassthroughDraft>(
    initial.passthrough,
  );
  const [yamlText, setYamlText] = useState(() =>
    initialYamlText(props.backendAuth),
  );
  const [error, setError] = useState<string | null>(null);

  // Deliberately validates/autocompletes against the canonical BackendAuth
  // schema, not the BackendAuthCompat shim. The proxy still accepts the legacy
  // `key: <string|{file}>` shorthand, but the editor flags it so users are
  // steered toward the current `key.value` form — don't switch this to Compat.
  const schema = props.help.node(["$defs", "BackendAuth"]);
  const passthroughPreview = {
    passthrough: passthroughDraftToValue(passthrough),
  } as BackendAuth;

  function save() {
    if (kind === "passthrough") {
      props.onSave(passthroughPreview);
      return;
    }
    try {
      setError(null);
      const parsed = parseSchemaYamlEditorValue(yamlText);
      if (isEmptyValue(parsed)) {
        setError("Backend auth cannot be empty.");
        return;
      }
      props.onSave(parsed as BackendAuth);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid YAML");
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
      <FieldGroup
        label="Auth method"
        tooltip={props.help.definition(
          "BackendAuth",
          "Select how the gateway authenticates to the backend.",
        )}
      >
        <EnumSelector
          ariaLabel="Auth method"
          value={kind}
          options={authKindOptions}
          onChange={(next) => {
            setKind(next);
            setError(null);
          }}
        />
      </FieldGroup>

      {kind === "passthrough" ? (
        <>
          <PassthroughFields
            value={passthrough}
            help={props.help}
            onChange={setPassthrough}
          />
          <ResultingYaml value={passthroughPreview} />
        </>
      ) : (
        <>
          {error ? (
            <StatusBanner state="bad" title="Invalid YAML">
              {error}
            </StatusBanner>
          ) : null}
          <FieldGroup label="Backend auth YAML">
            <SchemaYamlEditor
              path="agentgateway-policy-backend-auth-raw.yaml"
              schema={schema ?? {}}
              showLineNumbers={false}
              invalid={Boolean(error)}
              value={yamlText}
              onChange={(value) => {
                setYamlText(value);
                if (error) setError(null);
              }}
              onSave={save}
            />
          </FieldGroup>
        </>
      )}
    </form>
  );
}

function isEmptyValue(value: unknown): boolean {
  return (
    !value || (typeof value === "object" && Object.keys(value).length === 0)
  );
}

function initialYamlText(value: unknown) {
  return isEmptyValue(value) ? "" : toYamlText(value);
}

// -- Draft parsing --

type Draft = { kind: AuthKind; passthrough: PassthroughDraft };

// A brand-new policy (null/undefined) opens in the passthrough form. Anything
// already configured opens in the editor that can represent it: a recognized
// passthrough shape gets the structured form, everything else (key, gcp, aws,
// azure, the string-valued `copilot`, oauth, crossAppAccess) opens in the Raw
// YAML editor, which round-trips the value untouched, until its own structured
// editor lands in a follow-up PR.
function draftFromBackendAuth(value: BackendAuth | null | undefined): Draft {
  if (value && typeof value === "object") {
    const v = value as Record<string, unknown>;
    if (v.passthrough !== undefined) {
      return {
        kind: "passthrough",
        passthrough: passthroughDraftFromValue(
          v.passthrough as Record<string, unknown>,
        ),
      };
    }
    return { kind: "raw", passthrough: emptyPassthroughDraft() };
  }
  return {
    kind: value === null || value === undefined ? "passthrough" : "raw",
    passthrough: emptyPassthroughDraft(),
  };
}
