import { useState } from "react";
import { ShieldCheck } from "lucide-react";
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

type AuthKind = "passthrough";

const authKindOptions: Array<EnumSelectorOption<AuthKind>> = [
  {
    value: "passthrough",
    label: "Passthrough",
    description: "Forward the validated incoming JWT to the backend.",
    icon: <ShieldCheck size={16} />,
  },
];

export function BackendAuthPolicyEditor(props: {
  formId?: string;
  backendAuth: BackendAuth | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: BackendAuth) => void;
}) {
  const initial = draftFromBackendAuth(props.backendAuth);
  if (initial.kind === "unsupported") {
    return (
      <UnsupportedBackendAuthFields
        formId={props.formId}
        value={props.backendAuth}
        help={props.help}
        onSave={props.onSave}
      />
    );
  }
  return <BackendAuthFields {...props} initial={initial.passthrough} />;
}

function UnsupportedBackendAuthFields(props: {
  formId?: string;
  value: unknown;
  help: SchemaHelp;
  onSave: (value: BackendAuth) => void;
}) {
  const [yamlText, setYamlText] = useState(() => initialYamlText(props.value));
  const [error, setError] = useState<string | null>(null);
  const schema = props.help.node(["$defs", "BackendAuth"]);

  function save() {
    try {
      setError(null);
      props.onSave(parseSchemaYamlEditorValue(yamlText) as BackendAuth);
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
      <StatusBanner state="warn" title="Unsupported backend auth shape">
        This backend auth policy uses a shape the visual editor does not support
        yet. Edit the raw YAML below — it still must match one of the
        BackendAuth methods.
      </StatusBanner>
      {error ? (
        <StatusBanner state="bad" title="Invalid YAML">
          {error}
        </StatusBanner>
      ) : null}
      <FieldGroup label="Backend auth YAML">
        <SchemaYamlEditor
          path="agentgateway-policy-backend-auth-unsupported.yaml"
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
    </form>
  );
}

function initialYamlText(value: unknown) {
  if (
    !value ||
    (typeof value === "object" && Object.keys(value).length === 0)
  ) {
    return "";
  }
  return toYamlText(value);
}

function BackendAuthFields(props: {
  formId?: string;
  help: SchemaHelp;
  saving: boolean;
  initial: PassthroughDraft;
  onSave: (value: BackendAuth) => void;
}) {
  // Only one auth method exists today, so this isn't stateful yet — the
  // selector is kept in place so the next auth method added is a one-line
  // addition to authKindOptions rather than a restructure.
  const kind: AuthKind = "passthrough";
  const [passthrough, setPassthrough] = useState(props.initial);

  function build(): BackendAuth {
    switch (kind) {
      case "passthrough":
        return {
          passthrough: passthroughDraftToValue(passthrough),
        } as BackendAuth;
    }
  }

  const preview = build();

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        props.onSave(preview);
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
          showSelectedDescription
          onChange={() => {}}
        />
      </FieldGroup>

      {kind === "passthrough" ? (
        <PassthroughFields value={passthrough} onChange={setPassthrough} />
      ) : null}

      <ResultingYaml value={preview} />
    </form>
  );
}

// -- Draft parsing --

type Draft =
  | { kind: "passthrough"; passthrough: PassthroughDraft }
  | { kind: "unsupported"; raw: unknown };

// Anything the structured form can't fully represent is routed to the raw-YAML
// editor, which round-trips the whole object untouched on save. Every non-
// passthrough BackendAuth method (key, gcp, aws, azure, copilot, oauth,
// crossAppAccess) falls here today — they land as structured editors in
// follow-up PRs.
function rawFallback(value: unknown): Draft {
  return { kind: "unsupported", raw: value };
}

function draftFromBackendAuth(value: BackendAuth | null | undefined): Draft {
  if (!value || typeof value !== "object") {
    return { kind: "passthrough", passthrough: emptyPassthroughDraft() };
  }
  const v = value as Record<string, unknown>;
  if (v.passthrough !== undefined) {
    return {
      kind: "passthrough",
      passthrough: passthroughDraftFromValue(
        v.passthrough as Record<string, unknown>,
      ),
    };
  }
  return rawFallback(value);
}
