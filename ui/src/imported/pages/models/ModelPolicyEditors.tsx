import { Field } from "../../components/Primitives";
import { ListEditor } from "../../policies/ListEditor";
import { KeyValueEditor } from "../../policies/PolicyFormControls";
import type { SchemaHelp } from "../../schemaHelp";
import type { LlmModel } from "../../types";

export function HealthPolicyEditor(props: {
  health: LlmModel["health"] | null | undefined;
  help: SchemaHelp;
  onChange: (value: LlmModel["health"] | null) => void;
}) {
  const health = props.health ?? {};
  const eviction = health.eviction ?? {};

  function patchHealth(value: Partial<NonNullable<LlmModel["health"]>>) {
    props.onChange({ ...health, ...value });
  }

  function patchEviction(value: Partial<NonNullable<NonNullable<LlmModel["health"]>["eviction"]>>) {
    patchHealth({ eviction: { ...eviction, ...value } });
  }

  return (
    <div className="policy-editor-stack compact">
      <Field
        label="Unhealthy expression"
        tooltip={props.help.description(["$defs", "LocalHealthPolicy", "properties", "unhealthyExpression"])}
        hint="Leave empty to use default 5xx and connection failure handling."
      >
        <textarea
          className="mono-input"
          rows={4}
          value={health.unhealthyExpression ?? ""}
          onChange={(event) => patchHealth({ unhealthyExpression: event.target.value || null })}
          placeholder="response.status >= 500"
        />
      </Field>
      <div className="form-grid">
        <Field label="Eviction duration" tooltip={props.help.description(["$defs", "LocalEviction", "properties", "duration"])}>
          <input value={eviction.duration ?? ""} onChange={(event) => patchEviction({ duration: event.target.value || null })} placeholder="30s" />
        </Field>
        <Field label="Consecutive failures" tooltip={props.help.description(["$defs", "LocalEviction", "properties", "consecutiveFailures"])}>
          <input
            type="number"
            min="1"
            value={eviction.consecutiveFailures ?? ""}
            onChange={(event) => patchEviction({ consecutiveFailures: optionalNumber(event.target.value) })}
            placeholder="3"
          />
        </Field>
        <Field label="Health threshold" tooltip={props.help.description(["$defs", "LocalEviction", "properties", "healthThreshold"])}>
          <input
            type="number"
            value={eviction.healthThreshold ?? ""}
            onChange={(event) => patchEviction({ healthThreshold: optionalNumber(event.target.value) })}
            placeholder="0.5"
          />
        </Field>
        <Field label="Restore health" tooltip={props.help.description(["$defs", "LocalEviction", "properties", "restoreHealth"])}>
          <input
            type="number"
            value={eviction.restoreHealth ?? ""}
            onChange={(event) => patchEviction({ restoreHealth: optionalNumber(event.target.value) })}
            placeholder="1"
          />
        </Field>
      </div>
    </div>
  );
}

export function YamlMappingEditor(props: {
  label: string;
  tooltip?: string;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
}) {
  return (
    <Field label={props.label} tooltip={props.tooltip}>
      <textarea
        className="mono-input"
        rows={6}
        value={props.value}
        onChange={(event) => props.onChange(event.target.value)}
        placeholder={props.placeholder}
      />
    </Field>
  );
}

export function HeaderModifierEditor(props: {
  value: LlmModel["requestHeaders"] | LlmModel["responseHeaders"] | null | undefined;
  help: SchemaHelp;
  onChange: (value: LlmModel["requestHeaders"] | null) => void;
}) {
  const value = props.value ?? {};
  return (
    <div className="policy-editor-stack compact">
      <KeyValueEditor
        label="Add headers"
        tooltip={props.help.description(["$defs", "HeaderModifier", "properties", "add"])}
        values={value.add ?? {}}
        keyPlaceholder="x-header"
        valuePlaceholder="value"
        onChange={(add) => props.onChange({ ...value, add })}
      />
      <KeyValueEditor
        label="Set headers"
        tooltip={props.help.description(["$defs", "HeaderModifier", "properties", "set"])}
        values={value.set ?? {}}
        keyPlaceholder="x-header"
        valuePlaceholder="value"
        onChange={(set) => props.onChange({ ...value, set })}
      />
      <ListEditor
        label="Remove headers"
        tooltip={props.help.description(["$defs", "HeaderModifier", "properties", "remove"])}
        values={value.remove ?? []}
        placeholder="x-header"
        onChange={(remove) => props.onChange({ ...value, remove })}
      />
    </div>
  );
}

export function PromptCachingEditor(props: {
  value: LlmModel["promptCaching"] | null | undefined;
  help: SchemaHelp;
  onChange: (value: LlmModel["promptCaching"] | null) => void;
}) {
  const value = props.value ?? {};

  function patch(next: Partial<NonNullable<LlmModel["promptCaching"]>>) {
    props.onChange({ ...value, ...next });
  }

  return (
    <div className="policy-editor-stack compact">
      <div className="form-grid">
        <label className="config-option-row">
          <input type="checkbox" checked={Boolean(value.cacheSystem)} onChange={(event) => patch({ cacheSystem: event.target.checked || undefined })} />
          <span><strong>System prompt</strong><small>{props.help.description(["$defs", "PromptCachingConfig", "properties", "cacheSystem"])}</small></span>
        </label>
        <label className="config-option-row">
          <input type="checkbox" checked={Boolean(value.cacheMessages)} onChange={(event) => patch({ cacheMessages: event.target.checked || undefined })} />
          <span><strong>Messages</strong><small>{props.help.description(["$defs", "PromptCachingConfig", "properties", "cacheMessages"])}</small></span>
        </label>
        <label className="config-option-row">
          <input type="checkbox" checked={Boolean(value.cacheTools)} onChange={(event) => patch({ cacheTools: event.target.checked || undefined })} />
          <span><strong>Tools</strong><small>{props.help.description(["$defs", "PromptCachingConfig", "properties", "cacheTools"])}</small></span>
        </label>
      </div>
      <div className="form-grid">
        <Field label="Minimum tokens" tooltip={props.help.description(["$defs", "PromptCachingConfig", "properties", "minTokens"])}>
          <input type="number" value={value.minTokens ?? ""} onChange={(event) => patch({ minTokens: optionalNumber(event.target.value) })} placeholder="1024" />
        </Field>
        <Field label="Message offset" tooltip={props.help.description(["$defs", "PromptCachingConfig", "properties", "cacheMessageOffset"])}>
          <input type="number" value={value.cacheMessageOffset ?? ""} onChange={(event) => patch({ cacheMessageOffset: optionalNumber(event.target.value) ?? undefined })} placeholder="0" />
        </Field>
      </div>
    </div>
  );
}

export function healthSummary(health: LlmModel["health"] | null | undefined) {
  if (!health) return "No health policy configured";
  const parts = [
    health.unhealthyExpression ? "custom expression" : null,
    health.eviction?.duration ? `evict ${health.eviction.duration}` : null,
    health.eviction?.consecutiveFailures ? `${health.eviction.consecutiveFailures} failures` : null,
  ].filter(Boolean);
  return parts.join(", ") || "Default unhealthy detection configured";
}

export function headerModifierSummary(value: LlmModel["requestHeaders"] | LlmModel["responseHeaders"] | null | undefined, label: "request" | "response") {
  const count = Object.keys(value?.add ?? {}).length + Object.keys(value?.set ?? {}).length + (value?.remove?.length ?? 0);
  if (count === 0) return `No ${label} header changes configured`;
  return `${count} ${count === 1 ? "header change" : "header changes"} configured`;
}

export function promptCachingSummary(value: LlmModel["promptCaching"] | null | undefined) {
  if (!value) return "No prompt caching configured";
  const scopes = [
    value.cacheSystem ? "system" : null,
    value.cacheMessages ? "messages" : null,
    value.cacheTools ? "tools" : null,
  ].filter(Boolean);
  return scopes.length ? `Cache ${scopes.join(", ")}` : "Prompt caching configured";
}

function optionalNumber(value: string) {
  return value === "" ? null : Number(value);
}
