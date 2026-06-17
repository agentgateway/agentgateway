import { Save } from "lucide-react";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { FieldGroup } from "../components/Primitives";
import { ListEditor } from "./ListEditor";
import { KeyValueEditor } from "./PolicyFormControls";
import { CollapsiblePolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty } from "./policyUtils";
import type { SchemaHelp } from "../schemaHelp";
import type { TransformDraft, TransformationDraft } from "./types";
import type { LocalTransform } from "../gateway-config";
import { useEffect, useState } from "react";

export function TransformationsPolicyEditor(props: {
  formId?: string;
  transformations: TransformationDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: TransformationDraft) => void;
}) {
  const [request, setRequest] = useState<TransformDraft>(
    props.transformations?.request ?? {},
  );
  const [response, setResponse] = useState<TransformDraft>(
    props.transformations?.response ?? {},
  );
  const preview = cleanEmpty({ request, response }) as TransformationDraft;

  useEffect(() => {
    setRequest(props.transformations?.request ?? {});
    setResponse(props.transformations?.response ?? {});
  }, [props.transformations]);

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        props.onSave(preview);
      }}
    >
      <TransformSection
        key={`request-${hasTransformContent(request)}`}
        title="Request transformations"
        label="request"
        value={request}
        help={props.help}
        onChange={setRequest}
      />
      <TransformSection
        key={`response-${hasTransformContent(response)}`}
        title="Response transformations"
        label="response"
        value={response}
        help={props.help}
        onChange={setResponse}
      />
      <ResultingYaml value={preview} />
    </form>
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
        tooltip={props.help.field<LocalTransform>("LocalTransform", "add")}
        values={props.value.add ?? {}}
        keyPlaceholder="header name"
        valuePlaceholder="CEL expression"
        valueKind="cel"
        onChange={(add) => props.onChange({ ...props.value, add })}
      />
      <KeyValueEditor
        label="Set headers"
        tooltip={props.help.field<LocalTransform>("LocalTransform", "set")}
        values={props.value.set ?? {}}
        keyPlaceholder="header name"
        valuePlaceholder="CEL expression"
        valueKind="cel"
        onChange={(set) => props.onChange({ ...props.value, set })}
      />
      <ListEditor
        label="Remove headers"
        tooltip={props.help.field<LocalTransform>("LocalTransform", "remove")}
        values={props.value.remove ?? []}
        placeholder="header name"
        onChange={(remove) => props.onChange({ ...props.value, remove })}
      />
      <FieldGroup
        label="Body expression"
        tooltip={props.help.field<LocalTransform>("LocalTransform", "body")}
      >
        <MiniMonacoEditor
          language="cel"
          value={props.value.body ?? ""}
          onChange={(body) => props.onChange({ ...props.value, body })}
          placeholder="CEL expression"
        />
      </FieldGroup>
      <KeyValueEditor
        label="Metadata"
        tooltip={props.help.field<LocalTransform>("LocalTransform", "metadata")}
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

function transformSummary(
  value: TransformDraft,
  label: "request" | "response",
) {
  const count = countTransformOperations(value);
  if (count === 0) return `No ${label} transformations configured.`;
  return `${count} ${count === 1 ? "operation" : "operations"} configured.`;
}

function countTransformOperations(value: TransformDraft) {
  return (
    Object.keys(value.add ?? {}).length +
    Object.keys(value.set ?? {}).length +
    (value.remove?.length ?? 0) +
    Object.keys(value.metadata ?? {}).length +
    (value.body ? 1 : 0)
  );
}
