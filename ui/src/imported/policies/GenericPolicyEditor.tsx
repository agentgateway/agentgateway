import { useState } from "react";
import { Save } from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { Dropdown, Field, FieldGroup, StatusBanner } from "../components/Primitives";
import { ResultingYaml } from "./ResultingYaml";
import type { SchemaNode } from "./types";
import {
  cleanEmpty,
  enumOptionDetails,
  isRecord,
  parseYamlText,
  placeholderForSchema,
  resolveSchema,
  schemaObjectProperties,
  schemaType,
  splitList,
  titleFromKey,
  toYamlText,
} from "./policyUtils";

export function GenericPolicyEditor(props: {
  policyKey: string;
  value: unknown;
  help: SchemaHelp;
  saving: boolean;
  schemaRoot?: string;
  onSave: (value: unknown) => void;
}) {
  const schema = resolveSchema(props.help, props.help.node(["$defs", props.schemaRoot ?? "LocalLLMPolicy", "properties", props.policyKey]));
  const initialValue = props.value ?? {};
  const [draft, setDraft] = useState<unknown>(initialValue);
  const [yamlText, setYamlText] = useState(toYamlText(initialValue ?? {}));
  const [error, setError] = useState<string | null>(null);

  const editableObject = schemaObjectProperties(schema);

  function saveYaml() {
    try {
      setError(null);
      props.onSave(parseYamlText(yamlText));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid YAML");
    }
  }

  if (!editableObject) {
    return (
      <div>
        <StatusBanner state="loading" title="Schema fallback">
          This policy shape is too complex for the generic field renderer, so it is editable as YAML for now.
        </StatusBanner>
        {error ? <StatusBanner state="bad" title="Invalid YAML">{error}</StatusBanner> : null}
        <Field label="Policy YAML" tooltip={schema?.description}>
          <textarea className="mono-input" rows={18} value={yamlText} onChange={(event) => setYamlText(event.target.value)} />
        </Field>
        <button className="button primary" type="button" disabled={props.saving} onClick={saveYaml}>
          <Save size={16} />
          Save policy
        </button>
      </div>
    );
  }

  const objectDraft = isRecord(draft) ? draft : {};

  return (
    <div>
      <div className="generic-policy-form">
        {Object.entries(editableObject).map(([fieldName, fieldSchema]) => (
          <SchemaField
            key={fieldName}
            name={fieldName}
            schema={fieldSchema}
            help={props.help}
            value={objectDraft[fieldName]}
            onChange={(value) => setDraft({ ...objectDraft, [fieldName]: value })}
          />
        ))}
      </div>
      <ResultingYaml value={draft ?? null} />
      <button className="button primary" type="button" disabled={props.saving} onClick={() => props.onSave(cleanEmpty(draft))}>
        <Save size={16} />
        Save policy
      </button>
    </div>
  );
}

function SchemaField(props: {
  name: string;
  schema: SchemaNode;
  help: SchemaHelp;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const schema = resolveSchema(props.help, props.schema);
  const label = titleFromKey(props.name);
  const tooltip = schema?.description;
  const type = schemaType(schema);
  const enumValues = enumOptionDetails(schema);
  const placeholder = placeholderForSchema(schema);

  if (enumValues.length > 0) {
    const selected = typeof props.value === "string" ? props.value : "";
    return (
      <FieldGroup label={label} tooltip={tooltip}>
        <Dropdown
          ariaLabel={label}
          value={selected}
          placeholder="Unset"
          options={[
            { value: "", label: "Unset" },
            ...enumValues.map((option) => ({
              value: option.value,
              label: option.description ? (
                <span className="select-option-copy">
                  <strong>{option.label}</strong>
                  <small>{option.description}</small>
                </span>
              ) : option.label,
              searchText: `${option.label} ${option.description ?? ""}`,
            })),
          ]}
          onChange={(value) => props.onChange(value || undefined)}
        />
      </FieldGroup>
    );
  }

  if (type === "boolean") {
    return (
      <label className="toggle-row generic-toggle">
        <input type="checkbox" checked={Boolean(props.value)} onChange={(event) => props.onChange(event.target.checked)} />
        <span>{label}</span>
      </label>
    );
  }

  if (type === "number" || type === "integer") {
    return (
      <Field label={label} tooltip={tooltip}>
        <input
          type="number"
          value={typeof props.value === "number" ? String(props.value) : ""}
          placeholder={placeholder}
          onChange={(event) => props.onChange(event.target.value === "" ? undefined : Number(event.target.value))}
        />
      </Field>
    );
  }

  if (type === "array") {
    const itemSchema = resolveSchema(props.help, schema?.items);
    const itemType = schemaType(itemSchema);
    if (itemType === "string" || !itemSchema) {
      return (
        <Field label={label} tooltip={tooltip}>
          <textarea rows={4} value={Array.isArray(props.value) ? props.value.join("\n") : ""} placeholder={placeholder} onChange={(event) => props.onChange(splitList(event.target.value))} />
        </Field>
      );
    }
    return <YamlField label={label} tooltip={tooltip} value={Array.isArray(props.value) ? props.value : []} rows={7} onChange={props.onChange} />;
  }

  if (type === "object") {
    const properties = schemaObjectProperties(schema);
    if (!properties) {
      return <YamlField label={label} tooltip={tooltip} value={props.value ?? {}} rows={7} onChange={props.onChange} />;
    }
    const value = isRecord(props.value) ? props.value : {};
    return (
      <div className="generic-object">
        <div className="section-heading">
          <h3>{label}</h3>
          {tooltip ? <p>{tooltip}</p> : null}
        </div>
        {Object.entries(properties).map(([childName, childSchema]) => (
          <SchemaField
            key={childName}
            name={childName}
            schema={childSchema}
            help={props.help}
            value={value[childName]}
            onChange={(next) => props.onChange({ ...value, [childName]: next })}
          />
        ))}
      </div>
    );
  }

  return (
    <Field label={label} tooltip={tooltip}>
      <input value={typeof props.value === "string" ? props.value : ""} placeholder={placeholder} onChange={(event) => props.onChange(event.target.value || undefined)} />
    </Field>
  );
}

function YamlField(props: {
  label: string;
  tooltip?: string;
  value: unknown;
  rows: number;
  onChange: (value: unknown) => void;
}) {
  const [text, setText] = useState(toYamlText(props.value));
  const [error, setError] = useState<string | null>(null);

  return (
    <Field label={props.label} tooltip={props.tooltip} hint={error ?? undefined}>
      <textarea
        className="mono-input"
        rows={props.rows}
        value={text}
        onChange={(event) => {
          const next = event.target.value;
          setText(next);
          try {
            setError(null);
            props.onChange(parseYamlText(next));
          } catch {
            setError("Invalid YAML");
          }
        }}
      />
    </Field>
  );
}
