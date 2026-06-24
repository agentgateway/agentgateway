import { useMemo, useState } from "react";
import { Gauge, Plus, Trash2 } from "lucide-react";
import { EnumSelector } from "../components/EnumSelector";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { UnsupportedYamlFallback } from "../components/EditorContracts";
import { Field, FieldGroup } from "../components/Primitives";
import type { SchemaHelp } from "../schemaHelp";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import {
  TargetEditor,
  hasUnsupportedTarget,
  targetFrom,
  unsupportedTargetLabel,
} from "./PolicyFormControls";
import { cleanEmpty, isRecord } from "./policyUtils";
import type { RemoteRateLimitDraft, TargetDraft } from "./types";
import type { DescriptorEntry, RemoteRateLimit } from "../gateway-config";

type FailureMode = "failClosed" | "failOpen";
type DescriptorType = "requests" | "tokens";

type DescriptorDraft = {
  entries: Array<{ key: string; value: string }>;
  type: DescriptorType;
  cost: string;
  limitOverride: string;
};

export function RemoteRateLimitPolicyEditor(props: {
  formId?: string;
  remoteRateLimit: RemoteRateLimitDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (remoteRateLimit: RemoteRateLimitDraft) => void;
}) {
  const unsupported =
    props.remoteRateLimit &&
    (isConditional(props.remoteRateLimit) ||
      hasUnsupportedTarget(props.remoteRateLimit));
  const [domain, setDomain] = useState(
    props.remoteRateLimit && !isConditional(props.remoteRateLimit)
      ? (props.remoteRateLimit.domain ?? "")
      : "",
  );
  const [target, setTarget] = useState<TargetDraft>(() =>
    targetFrom(
      props.remoteRateLimit,
      "ratelimit.default.svc.cluster.local:8081",
    ),
  );
  const [failureMode, setFailureMode] = useState<FailureMode>(
    failureModeFrom(
      props.remoteRateLimit && !isConditional(props.remoteRateLimit)
        ? props.remoteRateLimit.failureMode
        : undefined,
    ),
  );
  const [descriptors, setDescriptors] = useState<DescriptorDraft[]>(() =>
    descriptorDrafts(props.remoteRateLimit),
  );
  const [attemptedSave, setAttemptedSave] = useState(false);
  const errors = useMemo(
    () => validate({ domain, target, descriptors }),
    [domain, target, descriptors],
  );

  if (unsupported) {
    return (
      <UnsupportedYamlFallback
        title="Unsupported remote rate limit shape"
        value={props.remoteRateLimit}
        schema={props.help.node(["$defs", "RemoteRateLimit"])}
        help={props.help}
      >
        {isConditional(props.remoteRateLimit)
          ? "This policy uses conditional remote rate limit entries. The visual editor currently supports one explicit remote rate limit."
          : `This policy uses a ${unsupportedTargetLabel(props.remoteRateLimit)} target. The visual editor currently supports host targets only.`}
      </UnsupportedYamlFallback>
    );
  }

  function save() {
    setAttemptedSave(true);
    if (errors.length) return;
    const preserved: Record<string, unknown> =
      isRecord(props.remoteRateLimit) && !isConditional(props.remoteRateLimit)
        ? { ...props.remoteRateLimit }
        : {};
    delete preserved.service;
    delete preserved.backend;
    const next = cleanEmpty({
      ...preserved,
      ...target,
      domain,
      descriptors: descriptors.map((descriptor) =>
        cleanEmpty({
          entries: descriptor.entries,
          type: descriptor.type === "requests" ? undefined : descriptor.type,
          cost: descriptor.cost.trim() || undefined,
          limitOverride: descriptor.limitOverride.trim() || undefined,
        }),
      ),
      failureMode: failureMode === "failClosed" ? undefined : failureMode,
    }) as RemoteRateLimitDraft;
    props.onSave(next);
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
      {attemptedSave && errors.length ? (
        <div className="field-error remote-rate-errors">{errors.join(" ")}</div>
      ) : null}

      <PolicySection
        icon={<Gauge size={17} />}
        title="Service"
        description="Remote rate limit service and domain used when building descriptor checks."
      >
        <div className="form-grid">
          <Field
            label="Domain"
            tooltip={props.help.field<RemoteRateLimit>(
              "RemoteRateLimit",
              "domain",
            )}
            className={attemptedSave && !domain.trim() ? "invalid" : undefined}
          >
            <input
              value={domain}
              onChange={(event) => setDomain(event.target.value)}
              placeholder="agentgateway"
            />
          </Field>
          <FieldGroup
            label="Failure mode"
            tooltip={props.help.field<RemoteRateLimit>(
              "RemoteRateLimit",
              "failureMode",
            )}
          >
            <EnumSelector<FailureMode>
              ariaLabel="Failure mode"
              value={failureMode}
              options={[
                {
                  value: "failClosed",
                  label: "Fail closed",
                  description:
                    "Deny requests when the rate limit service is unavailable.",
                },
                {
                  value: "failOpen",
                  label: "Fail open",
                  description:
                    "Allow requests when the rate limit service is unavailable.",
                },
              ]}
              onChange={setFailureMode}
            />
          </FieldGroup>
        </div>
        <TargetEditor
          value={target}
          tooltip={props.help.field<RemoteRateLimit>("RemoteRateLimit", "host")}
          onChange={setTarget}
        />
      </PolicySection>

      <PolicySection
        icon={<Plus size={17} />}
        title="Descriptors"
        description="Descriptor entries sent to the remote service. Values are CEL expressions evaluated from the request."
      >
        <div className="remote-descriptor-list">
          {descriptors.map((descriptor, index) => (
            <DescriptorEditor
              key={index}
              descriptor={descriptor}
              help={props.help}
              index={index}
              invalid={
                attemptedSave &&
                descriptor.entries.every(
                  (entry) => !entry.key.trim() || !entry.value.trim(),
                )
              }
              onChange={(next) =>
                setDescriptors(
                  descriptors.map((item, itemIndex) =>
                    itemIndex === index ? next : item,
                  ),
                )
              }
              onRemove={() =>
                setDescriptors(
                  descriptors.filter((_, itemIndex) => itemIndex !== index),
                )
              }
            />
          ))}
        </div>
        <button
          className="button"
          type="button"
          onClick={() => setDescriptors([...descriptors, emptyDescriptor()])}
        >
          <Plus size={16} />
          Add descriptor
        </button>
      </PolicySection>

      <ResultingYaml
        value={cleanEmpty({
          ...target,
          domain,
          descriptors: descriptors.map((descriptor) =>
            cleanEmpty({
              entries: descriptor.entries,
              type:
                descriptor.type === "requests" ? undefined : descriptor.type,
              cost: descriptor.cost.trim() || undefined,
              limitOverride: descriptor.limitOverride.trim() || undefined,
            }),
          ),
          failureMode: failureMode === "failClosed" ? undefined : failureMode,
        })}
      />
    </form>
  );
}

function DescriptorEditor(props: {
  descriptor: DescriptorDraft;
  help: SchemaHelp;
  index: number;
  invalid: boolean;
  onChange: (descriptor: DescriptorDraft) => void;
  onRemove: () => void;
}) {
  function updateEntry(
    entryIndex: number,
    patch: Partial<{ key: string; value: string }>,
  ) {
    props.onChange({
      ...props.descriptor,
      entries: props.descriptor.entries.map((entry, index) =>
        index === entryIndex ? { ...entry, ...patch } : entry,
      ),
    });
  }

  return (
    <div
      className={
        props.invalid
          ? "remote-descriptor-card invalid"
          : "remote-descriptor-card"
      }
    >
      <div className="remote-descriptor-header">
        <div>
          <strong>Descriptor {props.index + 1}</strong>
          <small>
            {props.descriptor.entries.length}{" "}
            {props.descriptor.entries.length === 1 ? "entry" : "entries"}
          </small>
        </div>
        <button
          className="icon-button danger"
          type="button"
          aria-label="Remove descriptor"
          onClick={props.onRemove}
        >
          <Trash2 size={16} />
        </button>
      </div>
      <FieldGroup
        label="Type"
        tooltip={props.help.field<DescriptorEntry>("DescriptorEntry", "type")}
      >
        <EnumSelector<DescriptorType>
          ariaLabel={`Descriptor ${props.index + 1} type`}
          value={props.descriptor.type}
          options={[
            {
              value: "requests",
              label: "Requests",
              description:
                "Evaluate request-count descriptors while processing the request.",
            },
            {
              value: "tokens",
              label: "Tokens",
              description:
                "Evaluate token descriptors after the LLM response completes.",
            },
          ]}
          onChange={(type) => props.onChange({ ...props.descriptor, type })}
        />
      </FieldGroup>
      <FieldGroup
        label="Entries"
        tooltip={props.help.field<DescriptorEntry>(
          "DescriptorEntry",
          "entries",
        )}
      >
        <div className="remote-entry-list">
          {props.descriptor.entries.map((entry, index) => (
            <div className="remote-entry-row" key={index}>
              <input
                className="mono-input compact"
                value={entry.key}
                onChange={(event) =>
                  updateEntry(index, { key: event.target.value })
                }
                placeholder="key"
              />
              <MiniMonacoEditor
                className="remote-entry-expression"
                language="cel"
                value={entry.value}
                onChange={(value) => updateEntry(index, { value })}
                placeholder='request.headers["x-user"]'
              />
              <button
                className="icon-button danger"
                type="button"
                aria-label="Remove descriptor entry"
                onClick={() => {
                  const next = props.descriptor.entries.filter(
                    (_, entryIndex) => entryIndex !== index,
                  );
                  props.onChange({
                    ...props.descriptor,
                    entries: next.length ? next : [{ key: "", value: "" }],
                  });
                }}
              >
                <Trash2 size={16} />
              </button>
            </div>
          ))}
        </div>
        <button
          className="button"
          type="button"
          onClick={() =>
            props.onChange({
              ...props.descriptor,
              entries: [...props.descriptor.entries, { key: "", value: "" }],
            })
          }
        >
          <Plus size={16} />
          Add entry
        </button>
      </FieldGroup>
      <div className="form-grid">
        <FieldGroup
          label="Cost expression"
          tooltip={props.help.field<DescriptorEntry>("DescriptorEntry", "cost")}
        >
          <MiniMonacoEditor
            className="mini-monaco-short"
            language="cel"
            value={props.descriptor.cost}
            onChange={(cost) => props.onChange({ ...props.descriptor, cost })}
            placeholder={
              props.descriptor.type === "tokens" ? "llm.totalTokens" : "1"
            }
          />
        </FieldGroup>
        <FieldGroup
          label="Limit override"
          tooltip={props.help.field<DescriptorEntry>(
            "DescriptorEntry",
            "limitOverride",
          )}
        >
          <MiniMonacoEditor
            className="mini-monaco-short"
            language="cel"
            value={props.descriptor.limitOverride}
            onChange={(limitOverride) =>
              props.onChange({ ...props.descriptor, limitOverride })
            }
            placeholder='{"unit":"minute","requestsPerUnit":100}'
          />
        </FieldGroup>
      </div>
    </div>
  );
}

function descriptorDrafts(
  value: RemoteRateLimitDraft | null | undefined,
): DescriptorDraft[] {
  if (!value || isConditional(value)) return [emptyDescriptor()];
  const descriptors = Array.isArray(value.descriptors) ? value.descriptors : [];
  return descriptors.length
    ? descriptors.map((descriptor) => ({
        entries: descriptor.entries?.length
          ? descriptor.entries.map((entry) => ({
              key: entry.key ?? "",
              value: entry.value ?? "",
            }))
          : [{ key: "", value: "" }],
        type: descriptor.type ?? "requests",
        cost: descriptor.cost ?? "",
        limitOverride: descriptor.limitOverride ?? "",
      }))
    : [emptyDescriptor()];
}

function emptyDescriptor(): DescriptorDraft {
  return {
    entries: [{ key: "user", value: 'request.headers["x-user"]' }],
    type: "requests",
    cost: "",
    limitOverride: "",
  };
}

function failureModeFrom(
  value: RemoteRateLimitDraft["failureMode"] | undefined,
): FailureMode {
  return value === "failOpen" || value === "FailOpen"
    ? "failOpen"
    : "failClosed";
}

function isConditional(value: unknown) {
  return isRecord(value) && Array.isArray(value.conditional);
}

function validate(args: {
  domain: string;
  target: TargetDraft;
  descriptors: DescriptorDraft[];
}) {
  const errors: string[] = [];
  if (!args.domain.trim()) errors.push("Domain is required.");
  if (!("host" in args.target) || !args.target.host.trim())
    errors.push("Host is required.");
  if (
    !args.descriptors.length ||
    args.descriptors.every((descriptor) =>
      descriptor.entries.every(
        (entry) => !entry.key.trim() || !entry.value.trim(),
      ),
    )
  ) {
    errors.push("At least one descriptor entry is required.");
  }
  return errors;
}
