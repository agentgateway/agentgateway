import { tr } from "../i18n";
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
    get label() {
      return tr("copy.fullDuplexStreamed");
    },
    get description() {
      return tr("copy.streamTheFullBodyThroughTheExternalProcessor");
    },
  },
  {
    value: "buffered",
    get label() {
      return tr("copy.buffered");
    },
    get description() {
      return tr("copy.bufferTheFullBodyBeforeSendingItToTheProcessor");
    },
  },
  {
    value: "bufferedPartial",
    get label() {
      return tr("copy.bufferedPartial");
    },
    get description() {
      return tr("copy.sendABoundedBodyBufferAndAllowTruncation");
    },
  },
  {
    value: "none",
    get label() {
      return tr("copy.none_deku7v");
    },
    get description() {
      return tr("copy.doNotSendTheBodyToTheProcessor");
    },
  },
];

const sendModes: Array<EnumSelectorOption<SendMode>> = [
  {
    value: "send",
    get label() {
      return tr("copy.send");
    },
    get description() {
      return tr("copy.sendThisPhaseToTheExternalProcessor");
    },
  },
  {
    value: "skip",
    get label() {
      return tr("copy.skip");
    },
    get description() {
      return tr("copy.doNotSendThisPhaseToTheExternalProcessor");
    },
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
        title={tr("copy.unsupportedTargetType")}
        value={props.extProc ?? {}}
        schema={props.help.node(["$defs", "ExtProc"])}
        help={props.help}
      >
        {tr("copy.thisPolicyUsesA")}
        {unsupportedTargetLabel(props.extProc)}
        {tr("copy.targetTheVisualEditorCurrentlySupportsHostTargetsOnly")}
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
        title={tr("copy.processingBehavior")}
        description={tr(
          "copy.chooseFailureBehaviorAndWhichRequestResponsePhasesAreSent",
        )}
      >
        <FieldGroup
          label={tr("copy.failureMode")}
          tooltip={props.help.field<ExtProc>("ExtProc", "failureMode")}
        >
          <EnumSelector
            ariaLabel="Failure mode"
            value={failureMode}
            options={[
              { value: "failClosed", label: tr("copy.failClosed") },
              { value: "failOpen", label: tr("copy.failOpen") },
            ]}
            schema={props.help.node(["$defs", "FailureMode5"])}
            onChange={setFailureMode}
          />
        </FieldGroup>
        <div className="form-grid">
          <ModeSelect
            label={tr("copy.requestBody")}
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestBodyMode",
            )}
            value={requestBodyMode}
            options={bodyModes}
            onChange={setRequestBodyMode}
          />
          <ModeSelect
            label={tr("copy.responseBody")}
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "responseBodyMode",
            )}
            value={responseBodyMode}
            options={bodyModes}
            onChange={setResponseBodyMode}
          />
          <ModeSelect
            label={tr("copy.requestHeaders")}
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestHeaderMode",
            )}
            value={requestHeaderMode}
            options={sendModes}
            onChange={setRequestHeaderMode}
          />
          <ModeSelect
            label={tr("copy.responseHeaders")}
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "responseHeaderMode",
            )}
            value={responseHeaderMode}
            options={sendModes}
            onChange={setResponseHeaderMode}
          />
          <ModeSelect
            label={tr("copy.requestTrailers")}
            tooltip={props.help.field<ProcessingOptions>(
              "ProcessingOptions",
              "requestTrailerMode",
            )}
            value={requestTrailerMode}
            options={sendModes}
            onChange={setRequestTrailerMode}
          />
          <ModeSelect
            label={tr("copy.responseTrailers")}
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
            <strong>{tr("copy.allowModeOverride")}</strong>
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
        title={tr("copy.attributes")}
        description={tr("copy.celExpressionsSentAsAttributesToTheProcessor")}
      >
        <KeyValueEditor
          label={tr("copy.requestAttributes")}
          tooltip={props.help.field<ExtProc>("ExtProc", "requestAttributes")}
          values={requestAttributes}
          keyPlaceholder="key"
          valuePlaceholder="CEL expression"
          valueKind="cel"
          onChange={setRequestAttributes}
        />
        <KeyValueEditor
          label={tr("copy.responseAttributes")}
          tooltip={props.help.field<ExtProc>("ExtProc", "responseAttributes")}
          values={responseAttributes}
          keyPlaceholder="key"
          valuePlaceholder="CEL expression"
          valueKind="cel"
          onChange={setResponseAttributes}
        />
        <FieldGroup
          label={tr("copy.metadataContextYaml")}
          tooltip={props.help.field<ExtProc>("ExtProc", "metadataContext")}
          className={metadataError ? "invalid" : undefined}
          hint={metadataError ?? undefined}
        >
          <MiniMonacoEditor
            language="yaml"
            value={metadataText}
            onChange={setMetadataText}
            placeholder={tr("copy.namespaceKeyCelExpression")}
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
