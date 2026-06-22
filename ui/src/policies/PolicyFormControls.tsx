import { useRef, useState } from "react";
import { Plus } from "lucide-react";
import { Field, FieldGroup } from "../components/Primitives";
import type { TargetDraft } from "./types";

export type TargetMode = "host" | "service" | "backend";

export function targetMode(value: unknown): TargetMode {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    if ("service" in value) return "service";
    if ("backend" in value) return "backend";
  }
  return "host";
}

export function hasUnsupportedTarget(value: unknown) {
  return targetMode(value) !== "host";
}

export function unsupportedTargetLabel(value: unknown) {
  const mode = targetMode(value);
  if (mode === "service") return "Kubernetes service";
  if (mode === "backend") return "backend reference";
  return "target";
}

export function TargetEditor(props: {
  value: TargetDraft;
  tooltip?: string;
  onChange: (value: TargetDraft) => void;
}) {
  const host = "host" in props.value ? props.value.host : "";

  return (
    <div className="policy-form-section">
      <div className="policy-form-section-header">
        <span className="policy-form-section-icon">
          <Plus size={17} />
        </span>
        <div>
          <h4>Target</h4>
          <p>External service the gateway calls for this policy.</p>
        </div>
      </div>
      <div className="policy-form-section-body">
        <Field label="Host" tooltip={props.tooltip}>
          <input
            value={host}
            onChange={(event) => props.onChange({ host: event.target.value })}
            placeholder="host:port or unix:/path/to/socket"
          />
        </Field>
      </div>
    </div>
  );
}

export function KeyValueEditor(props: {
  label: string;
  tooltip?: string;
  values: Record<string, string>;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  valueKind?: "text" | "cel";
  quickKeys?: string[];
  onChange: (values: Record<string, string>) => void;
}) {
  const [keyDraft, setKeyDraft] = useState("");
  const [valueDraft, setValueDraft] = useState("");
  const valueRef = useRef<HTMLInputElement>(null);
  const entries = Object.entries(props.values);

  function add() {
    const key = keyDraft.trim();
    if (!key) return;
    props.onChange({ ...props.values, [key]: valueDraft.trim() });
    setKeyDraft("");
    setValueDraft("");
  }

  return (
    <FieldGroup label={props.label} tooltip={props.tooltip}>
      <div className="kv-editor">
        {entries.length ? (
          <div className="kv-list">
            {entries.map(([key, value]) => (
              <div className="kv-row" key={key}>
                <code>{key}</code>
                <span>{value}</span>
                <button
                  className="table-action danger"
                  type="button"
                  onClick={() => {
                    const next = { ...props.values };
                    delete next[key];
                    props.onChange(next);
                  }}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty-inline">No values configured.</div>
        )}
        {props.quickKeys?.length ? (
          <div
            className="kv-quick-row"
            aria-label={`${props.label} quick keys`}
          >
            {props.quickKeys.map((key) => (
              <button
                className="choice-pill compact"
                type="button"
                key={key}
                disabled={key in props.values}
                onClick={() => {
                  setKeyDraft(key);
                  window.requestAnimationFrame(() => valueRef.current?.focus());
                }}
              >
                {key}
              </button>
            ))}
          </div>
        ) : null}
        <div className="kv-add-row">
          <input
            value={keyDraft}
            onChange={(event) => setKeyDraft(event.target.value)}
            placeholder={props.keyPlaceholder ?? "name"}
          />
          <input
            ref={valueRef}
            className={props.valueKind === "cel" ? "mono-input" : undefined}
            value={valueDraft}
            onChange={(event) => setValueDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                add();
              }
            }}
            placeholder={
              props.valuePlaceholder ??
              (props.valueKind === "cel" ? "request.path" : "value")
            }
          />
          <button className="button" type="button" onClick={add}>
            Add
          </button>
        </div>
      </div>
    </FieldGroup>
  );
}

export function targetFrom(
  value: unknown,
  fallbackHost = "127.0.0.1:9000",
): TargetDraft {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    if (
      "host" in value &&
      typeof (value as { host?: unknown }).host === "string"
    )
      return { host: (value as { host: string }).host };
  }
  return { host: fallbackHost };
}
