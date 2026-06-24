import { SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import {
  EnumSelector,
  type EnumSelectorOption,
} from "../components/EnumSelector";
import { UnsupportedYamlFallback } from "../components/EditorContracts";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { FieldGroup } from "../components/Primitives";
import {
  hasUnsupportedTarget,
  KeyValueEditor,
  TargetEditor,
  targetFrom,
  unsupportedTargetLabel,
} from "./PolicyFormControls";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty, parseYamlText, toYamlMappingText } from "./policyUtils";
import type { SchemaHelp } from "../schemaHelp";
import type { ExtProcDraft } from "./types";
import type { ExtProc, ProcessingOptions } from "../gateway-config";

type BodyMode = "none" | "buffered" | "bufferedPartial" | "fullDuplexStreamed";
type SendMode = "send" | "skip";

const bodyModes: Array<EnumSelectorOption<BodyMode>> = [
  {
    value: "fullDuplexStreamed",
    label: "Full duplex streamed",
    description: "Stream the full body through the external processor.",
  },
  {
    value: "buffered",
    label: "Buffered",
    description: "Buffer the full body before sending it to the processor.",
  },
  {
    value: "bufferedPartial",
    label: "Buffered partial",
    description: "Send a bounded body buffer and allow truncation.",
  },
  {
    value: "none",
    label: "None",
    description: "Do not send the body to the processor.",
  },
];

const sendModes: Array<EnumSelectorOption<SendMode>> = [
  {
    value: "send",
    label: "Send",
    description: "Send this phase to the external processor.",
  },
  {
    value: "skip",
    label: "Skip",
    description: "Do not send this phase to the external processor.",
  },
];

export function ExtProcPolicyEditor(props: {
  formId?: string;
  extProc: ExtProcDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: ExtProcDraft) => void;
}) {
  const unsupportedTarget = hasUnsupportedTarget(props.extProc);
  const [target, setTarget] = useState(() => targetFrom(props.extProc));
  const [failureMode, setFailureMode] = useState<"failClosed" | "failOpen">(
    props.extProc?.failureMode ?? "failClosed",
  );
  const options = props.extProc?.processingOptions ?? {};
  const [requestBodyMode, setRequestBodyMode] = useState<BodyMode>(
    options.requestBodyMode ?? "fullDuplexStreamed",
  );
  const [responseBodyMode, setResponseBodyMode] = useState<BodyMode>(
    options.responseBodyMode ?? "fullDuplexStreamed",
  );
  const [requestHeaderMode, setRequestHeaderMode] = useState<SendMode>(
    options.requestHeaderMode ?? "send",
  );
  const [responseHeaderMode, setResponseHeaderMode] = useState<SendMode>(
    options.responseHeaderMode ?? "send",
  );
  const [requestTrailerMode, setRequestTrailerMode] = useState<SendMode>(
    options.requestTrailerMode ?? "send",
  );
  const [responseTrailerMode, setResponseTrailerMode] = useState<SendMode>(
    options.responseTrailerMode ?? "send",
  );
  const [allowModeOverride, setAllowModeOverride] = useState(
    Boolean(options.allowModeOverride),
  );
  const [requestAttributes, setRequestAttributes] = useState(
    props.extProc?.requestAttributes ?? {},
  );
  const [responseAttributes, setResponseAttributes] = useState(
    props.extProc?.responseAttributes ?? {},
  );
  const [metadataText, setMetadataText] = useState(
    toYamlMappingText(props.extProc?.metadataContext),
  );
  const [metadataError, setMetadataError] = useState<string | null>(null);
  const preview = buildExtProc();

  if (unsupportedTarget) {
    return (
      <UnsupportedYamlFallback
        title="Unsupported target type"
        value={props.extProc ?? {}}
        schema={props.help.node(["$defs", "ExtProc"])}
        help={props.help}
      >
        This policy uses a {unsupportedTargetLabel(props.extProc)} target. The
        visual editor currently supports host targets only.
      </UnsupportedYamlFallback>
    );
  }

  function buildExtProc() {
    let metadataContext: unknown;
    try {
      metadataContext = metadataText.trim()
        ? parseYamlText(metadataText)
        : undefined;
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
      setMetadataError(
        err instanceof Error ? err.message : "Invalid metadata YAML",
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
      <TargetEditor
        value={target}
        tooltip={props.help.field<ExtProc>("ExtProc", "host")}
        onChange={setTarget}
      />
      <PolicySection
        icon={<SlidersHorizontal size={17} />}
        title="Processing behavior"
        description="Choose failure behavior and which request/response phases are sent."
      >
        <FieldGroup
          label="Failure mode"
          tooltip={props.help.field<ExtProc>("ExtProc", "failureMode")}
        >
          <EnumSelector
            ariaLabel="Failure mode"
            value={failureMode}
            options={[
              { value: "failClosed", label: "Fail closed" },
              { value: "failOpen", label: "Fail open" },
            ]}
            schema={props.help.node(["$defs", "FailureMode5"])}
            onChange={setFailureMode}
          />
        </FieldGroup>
        <div className="form-grid">
          <ModeSelect
            label="Request body"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestBodyMode",
            )}
            value={requestBodyMode}
            options={bodyModes}
            onChange={setRequestBodyMode}
          />
          <ModeSelect
            label="Response body"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "responseBodyMode",
            )}
            value={responseBodyMode}
            options={bodyModes}
            onChange={setResponseBodyMode}
          />
          <ModeSelect
            label="Request headers"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestHeaderMode",
            )}
            value={requestHeaderMode}
            options={sendModes}
            onChange={setRequestHeaderMode}
          />
          <ModeSelect
            label="Response headers"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "responseHeaderMode",
            )}
            value={responseHeaderMode}
            options={sendModes}
            onChange={setResponseHeaderMode}
          />
          <ModeSelect
            label="Request trailers"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestTrailerMode",
            )}
            value={requestTrailerMode}
            options={sendModes}
            onChange={setRequestTrailerMode}
          />
          <ModeSelect
            label="Response trailers"
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "responseTrailerMode",
            )}
            value={responseTrailerMode}
            options={sendModes}
            onChange={setResponseTrailerMode}
          />
        </div>
        <label className="config-option-row">
          <input
            type="checkbox"
            checked={allowModeOverride}
            onChange={(event) => setAllowModeOverride(event.target.checked)}
          />
          <span>
            <strong>Allow mode override</strong>
            <small>
              {props.help.field<ProcessingOptions>(
                "ProcessingOptions",
                "allowModeOverride",
              )}
            </small>
          </span>
        </label>
      </PolicySection>
      <PolicySection
        icon={<SlidersHorizontal size={17} />}
        title="Attributes"
        description="CEL expressions sent as attributes to the processor."
      >
        <KeyValueEditor
          label="Request attributes"
          tooltip={props.help.field<ExtProc>("ExtProc", "requestAttributes")}
          values={requestAttributes}
          keyPlaceholder="key"
          valuePlaceholder="CEL expression"
          valueKind="cel"
          onChange={setRequestAttributes}
        />
        <KeyValueEditor
          label="Response attributes"
          tooltip={props.help.field<ExtProc>("ExtProc", "responseAttributes")}
          values={responseAttributes}
          keyPlaceholder="key"
          valuePlaceholder="CEL expression"
          valueKind="cel"
          onChange={setResponseAttributes}
        />
        <FieldGroup
          label="Metadata context YAML"
          tooltip={props.help.field<ExtProc>("ExtProc", "metadataContext")}
          className={metadataError ? "invalid" : undefined}
          hint={metadataError ?? undefined}
        >
          <MiniMonacoEditor
            language="yaml"
            value={metadataText}
            onChange={setMetadataText}
            placeholder={"namespace:\n  key: CEL expression"}
          />
        </FieldGroup>
      </PolicySection>
      <ResultingYaml value={preview} />
    </form>
  );
}

function ModeSelect<T extends string>(props: {
  label: string;
  tooltip?: string;
  value: T;
  options: Array<EnumSelectorOption<T>>;
  onChange: (value: T) => void;
}) {
  return (
    <FieldGroup label={props.label} tooltip={props.tooltip}>
      <EnumSelector
        ariaLabel={props.label}
        value={props.value}
        options={props.options}
        onChange={props.onChange}
      />
    </FieldGroup>
  );
}
