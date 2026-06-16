import { Save } from "lucide-react";
import { Field } from "../components/Primitives";
import { ListEditor } from "./ListEditor";
import { KeyValueEditor } from "./PolicyFormControls";
import { CollapsiblePolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty } from "./policyUtils";
import type { SchemaHelp } from "../schemaHelp";
import type { TransformDraft, TransformationDraft } from "./types";
import { useState } from "react";

export function TransformationsPolicyEditor(props: {
  transformations: TransformationDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: TransformationDraft) => void;
}) {
  const [request, setRequest] = useState<TransformDraft>(props.transformations?.request ?? {});
  const [response, setResponse] = useState<TransformDraft>(props.transformations?.response ?? {});
  const preview = cleanEmpty({ request, response }) as TransformationDraft;

  return (
    <div className="policy-editor-stack">
      <TransformSection title="Request transformations" label="request" value={request} help={props.help} onChange={setRequest} />
      <TransformSection title="Response transformations" label="response" value={response} help={props.help} onChange={setResponse} />
      <ResultingYaml value={preview} />
      <button className="button primary" type="button" disabled={props.saving} onClick={() => props.onSave(preview)}>
        <Save size={16} />
        Save transformations
      </button>
    </div>
  );
}

function TransformSection(props: {
  title: string;
  label: "request" | "response";
  value: TransformDraft;
  help: SchemaHelp;
  onChange: (value: TransformDraft) => void;
}) {
  const summary = transformSummary(props.value, props.label);

  return (
    <CollapsiblePolicySection
      icon={<Save size={17} />}
      title={props.title}
      description={summary}
      defaultOpen={hasTransformContent(props.value)}
    >
      <KeyValueEditor
        label="Add headers"
        tooltip={props.help.description(["$defs", "LocalTransform", "properties", "add"])}
        values={props.value.add ?? {}}
        keyPlaceholder="header name"
        valuePlaceholder="CEL expression"
        valueKind="cel"
        onChange={(add) => props.onChange({ ...props.value, add })}
      />
      <KeyValueEditor
        label="Set headers"
        tooltip={props.help.description(["$defs", "LocalTransform", "properties", "set"])}
        values={props.value.set ?? {}}
        keyPlaceholder="header name"
        valuePlaceholder="CEL expression"
        valueKind="cel"
        onChange={(set) => props.onChange({ ...props.value, set })}
      />
      <ListEditor
        label="Remove headers"
        tooltip={props.help.description(["$defs", "LocalTransform", "properties", "remove"])}
        values={props.value.remove ?? []}
        placeholder="header name"
        onChange={(remove) => props.onChange({ ...props.value, remove })}
      />
      <Field label="Body expression" tooltip={props.help.description(["$defs", "LocalTransform", "properties", "body"])}>
        <textarea className="mono-input" rows={4} value={props.value.body ?? ""} onChange={(event) => props.onChange({ ...props.value, body: event.target.value })} placeholder="CEL expression" />
      </Field>
      <KeyValueEditor
        label="Metadata"
        tooltip={props.help.description(["$defs", "LocalTransform", "properties", "metadata"])}
        values={props.value.metadata ?? {}}
        keyPlaceholder="metadata key"
        valuePlaceholder="CEL expression"
        valueKind="cel"
        onChange={(metadata) => props.onChange({ ...props.value, metadata })}
      />
    </CollapsiblePolicySection>
  );
}

function hasTransformContent(value: TransformDraft) {
  return countTransformOperations(value) > 0;
}

function transformSummary(value: TransformDraft, label: "request" | "response") {
  const count = countTransformOperations(value);
  if (count === 0) return `No ${label} transformations configured.`;
  return `${count} ${count === 1 ? "operation" : "operations"} configured.`;
}

function countTransformOperations(value: TransformDraft) {
  return Object.keys(value.add ?? {}).length +
    Object.keys(value.set ?? {}).length +
    (value.remove?.length ?? 0) +
    Object.keys(value.metadata ?? {}).length +
    (value.body ? 1 : 0);
}
