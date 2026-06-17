import { useEffect, useMemo, useState } from "react";
import {
  SchemaYamlEditor,
  parseSchemaYamlEditorValue,
} from "../components/SchemaYamlEditor";
import { SchemaHelpPanel } from "../components/SchemaHelpPanel";
import { FieldGroup, StatusBanner } from "../components/Primitives";
import type { SchemaHelp } from "../schemaHelp";
import { toYamlText } from "./policyUtils";
import { ResultingYaml } from "./ResultingYaml";

export function GenericPolicyEditor(props: {
  formId?: string;
  policyKey: string;
  value: unknown;
  help: SchemaHelp;
  saving: boolean;
  schemaRoot?: string;
  showSchemaDescription?: boolean;
  onSave: (value: unknown) => void;
}) {
  const initialValue = props.value ?? {};
  const [yamlText, setYamlText] = useState(initialYamlText(initialValue));
  const [error, setError] = useState<string | null>(null);
  const schema = props.help.node([
    "$defs",
    props.schemaRoot ?? "LocalLLMPolicy",
    "properties",
    props.policyKey,
  ]);
  const preview = safeParseYaml(yamlText);
  const editorPath = useMemo(
    () =>
      `agentgateway-policy-${sanitizeEditorPath(props.schemaRoot ?? "LocalLLMPolicy")}-${sanitizeEditorPath(props.policyKey)}.yaml`,
    [props.policyKey, props.schemaRoot],
  );

  function saveYaml() {
    try {
      setError(null);
      props.onSave(parseSchemaYamlEditorValue(yamlText));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid YAML");
    }
  }

  useEffect(() => {
    setYamlText(initialYamlText(props.value ?? {}));
    setError(null);
  }, [props.value]);

  return (
    <form
      id={props.formId}
      className="policy-yaml-fallback"
      onSubmit={(event) => {
        event.preventDefault();
        saveYaml();
      }}
    >
      {error ? (
        <StatusBanner state="bad" title="Invalid YAML">
          {error}
        </StatusBanner>
      ) : null}
      <SchemaHelpPanel
        schema={schema}
        help={props.help}
        showDescription={props.showSchemaDescription}
      />
      <FieldGroup label="Policy YAML" className="policy-yaml-editor-field">
        <SchemaYamlEditor
          className="policy-yaml-editor"
          invalid={Boolean(error)}
          path={editorPath}
          schema={schema ?? {}}
          showLineNumbers={false}
          value={yamlText}
          onChange={(value) => {
            setYamlText(value);
            if (error) setError(null);
          }}
          onSave={saveYaml}
        />
      </FieldGroup>
      <ResultingYaml value={preview} />
    </form>
  );
}

function sanitizeEditorPath(value: string) {
  return value.replace(/[^A-Za-z0-9_-]+/g, "-");
}

function initialYamlText(value: unknown) {
  return isEmptyMapping(value) ? "" : toYamlText(value);
}

function isEmptyMapping(value: unknown) {
  return Boolean(
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.keys(value).length === 0,
  );
}

function safeParseYaml(value: string) {
  try {
    return parseSchemaYamlEditorValue(value);
  } catch {
    return { error: "Invalid YAML" };
  }
}
