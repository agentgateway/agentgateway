import { Save, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { Dropdown, Field, FieldGroup, StatusBanner, YamlBlock } from "../components/Primitives";
import { hasUnsupportedTarget, KeyValueEditor, TargetEditor, targetFrom, unsupportedTargetLabel } from "./PolicyFormControls";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty, parseYamlText, toYamlText } from "./policyUtils";
import type { SchemaHelp } from "../schemaHelp";
import type { ExtProcDraft } from "./types";

type BodyMode = "none" | "buffered" | "bufferedPartial" | "fullDuplexStreamed";
type SendMode = "send" | "skip";

const bodyModes: Array<{ value: BodyMode; label: string }> = [
  { value: "fullDuplexStreamed", label: "Full duplex streamed" },
  { value: "buffered", label: "Buffered" },
  { value: "bufferedPartial", label: "Buffered partial" },
  { value: "none", label: "None" },
];

const sendModes: Array<{ value: SendMode; label: string }> = [
  { value: "send", label: "Send" },
  { value: "skip", label: "Skip" },
];

export function ExtProcPolicyEditor(props: {
  extProc: ExtProcDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: ExtProcDraft) => void;
}) {
  const unsupportedTarget = hasUnsupportedTarget(props.extProc);
  const [target, setTarget] = useState(() => targetFrom(props.extProc));
  const [failureMode, setFailureMode] = useState<"failClosed" | "failOpen">(props.extProc?.failureMode ?? "failClosed");
  const options = props.extProc?.processingOptions ?? {};
  const [requestBodyMode, setRequestBodyMode] = useState<BodyMode>(options.requestBodyMode ?? "fullDuplexStreamed");
  const [responseBodyMode, setResponseBodyMode] = useState<BodyMode>(options.responseBodyMode ?? "fullDuplexStreamed");
  const [requestHeaderMode, setRequestHeaderMode] = useState<SendMode>(options.requestHeaderMode ?? "send");
  const [responseHeaderMode, setResponseHeaderMode] = useState<SendMode>(options.responseHeaderMode ?? "send");
  const [requestTrailerMode, setRequestTrailerMode] = useState<SendMode>(options.requestTrailerMode ?? "send");
  const [responseTrailerMode, setResponseTrailerMode] = useState<SendMode>(options.responseTrailerMode ?? "send");
  const [allowModeOverride, setAllowModeOverride] = useState(Boolean(options.allowModeOverride));
  const [requestAttributes, setRequestAttributes] = useState(props.extProc?.requestAttributes ?? {});
  const [responseAttributes, setResponseAttributes] = useState(props.extProc?.responseAttributes ?? {});
  const [metadataText, setMetadataText] = useState(toYamlText(props.extProc?.metadataContext ?? {}));
  const [metadataError, setMetadataError] = useState<string | null>(null);
  const preview = buildExtProc();

  if (unsupportedTarget) {
    return (
      <div className="policy-editor-stack">
        <StatusBanner state="warn" title="Unsupported target type">
          This policy uses a {unsupportedTargetLabel(props.extProc)} target. The visual editor currently supports host targets only.
        </StatusBanner>
        <YamlBlock value={props.extProc ?? {}} />
      </div>
    );
  }

  function buildExtProc() {
    let metadataContext: unknown;
    try {
      metadataContext = metadataText.trim() ? parseYamlText(metadataText) : undefined;
    } catch {
      metadataContext = undefined;
    }
    return cleanEmpty({
      ...target,
      failureMode,
      processingOptions: {
        requestBodyMode,
        responseBodyMode,
        requestHeaderMode,
        responseHeaderMode,
        requestTrailerMode,
        responseTrailerMode,
        allowModeOverride: allowModeOverride ? true : undefined,
      },
      requestAttributes,
      responseAttributes,
      metadataContext,
    }) as ExtProcDraft;
  }

  function save() {
    try {
      if (metadataText.trim()) parseYamlText(metadataText);
      setMetadataError(null);
      props.onSave(buildExtProc());
    } catch (err) {
      setMetadataError(err instanceof Error ? err.message : "Invalid metadata YAML");
    }
  }

  return (
    <div className="policy-editor-stack">
      <TargetEditor value={target} tooltip={props.help.description(["$defs", "ExtProc", "oneOf", 2, "properties", "host"])} onChange={setTarget} />
      <PolicySection
        icon={<SlidersHorizontal size={17} />}
        title="Processing behavior"
        description="Choose failure behavior and which request/response phases are sent."
      >
          <FieldGroup label="Failure mode" tooltip={props.help.description(["$defs", "ExtProc", "properties", "failureMode"])}>
            <Dropdown
              ariaLabel="Failure mode"
              value={failureMode}
              options={[{ value: "failClosed", label: "Fail closed" }, { value: "failOpen", label: "Fail open" }]}
              onChange={(value) => setFailureMode(value as "failClosed" | "failOpen")}
            />
          </FieldGroup>
          <div className="form-grid">
            <ModeSelect label="Request body" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "requestBodyMode"])} value={requestBodyMode} options={bodyModes} onChange={setRequestBodyMode} />
            <ModeSelect label="Response body" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "responseBodyMode"])} value={responseBodyMode} options={bodyModes} onChange={setResponseBodyMode} />
            <ModeSelect label="Request headers" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "requestHeaderMode"])} value={requestHeaderMode} options={sendModes} onChange={setRequestHeaderMode} />
            <ModeSelect label="Response headers" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "responseHeaderMode"])} value={responseHeaderMode} options={sendModes} onChange={setResponseHeaderMode} />
            <ModeSelect label="Request trailers" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "requestTrailerMode"])} value={requestTrailerMode} options={sendModes} onChange={setRequestTrailerMode} />
            <ModeSelect label="Response trailers" tooltip={props.help.description(["$defs", "ProcessingOptions", "properties", "responseTrailerMode"])} value={responseTrailerMode} options={sendModes} onChange={setResponseTrailerMode} />
          </div>
          <label className="config-option-row">
            <input type="checkbox" checked={allowModeOverride} onChange={(event) => setAllowModeOverride(event.target.checked)} />
            <span><strong>Allow mode override</strong><small>{props.help.description(["$defs", "ProcessingOptions", "properties", "allowModeOverride"])}</small></span>
          </label>
      </PolicySection>
      <PolicySection
        icon={<SlidersHorizontal size={17} />}
        title="Attributes"
        description="CEL expressions sent as attributes to the processor."
      >
          <KeyValueEditor label="Request attributes" tooltip={props.help.description(["$defs", "ExtProc", "properties", "requestAttributes"])} values={requestAttributes} keyPlaceholder="key" valuePlaceholder="CEL expression" valueKind="cel" onChange={setRequestAttributes} />
          <KeyValueEditor label="Response attributes" tooltip={props.help.description(["$defs", "ExtProc", "properties", "responseAttributes"])} values={responseAttributes} keyPlaceholder="key" valuePlaceholder="CEL expression" valueKind="cel" onChange={setResponseAttributes} />
          <Field label="Metadata context YAML" tooltip={props.help.description(["$defs", "ExtProc", "properties", "metadataContext"])} className={metadataError ? "invalid" : undefined} hint={metadataError ?? undefined}>
            <textarea className="mono-input" rows={7} value={metadataText} onChange={(event) => setMetadataText(event.target.value)} placeholder={"namespace:\n  key: CEL expression"} />
          </Field>
      </PolicySection>
      <ResultingYaml value={preview} />
      <button className="button primary" type="button" disabled={props.saving} onClick={save}>
        <Save size={16} />
        Save external processor
      </button>
    </div>
  );
}

function ModeSelect<T extends string>(props: {
  label: string;
  tooltip?: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <FieldGroup label={props.label} tooltip={props.tooltip}>
      <Dropdown ariaLabel={props.label} value={props.value} options={props.options} onChange={(value) => props.onChange(value as T)} />
    </FieldGroup>
  );
}
