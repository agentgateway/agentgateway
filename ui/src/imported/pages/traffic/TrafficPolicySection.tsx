import { ChevronDown, Shield, X } from "lucide-react";
import type { ComponentType } from "react";
import { useMemo, useState } from "react";
import { Tooltip, YamlBlock } from "../../components/Primitives";
import { PolicyEditorBody, type PolicyEditorKind } from "../../policies/PolicyDrawer";
import { policyUi } from "../../policies/registry";
import { policyEnabled, policySummary, titleFromKey } from "../../policies/policyUtils";
import { useSchemaHelp } from "../../schemaHelp";

export function TrafficPolicySection(props: {
  title: string;
  schemaRoot: "LocalGatewayPolicy" | "FilterOrPolicy" | "TCPFilterOrPolicy";
  policies?: Record<string, unknown> | null;
  onChange: (policies: Record<string, unknown> | null) => void;
}) {
  const help = useSchemaHelp();
  const [open, setOpen] = useState(Boolean(props.policies && Object.keys(props.policies).length));
  const [selected, setSelected] = useState<string | null>(null);
  const catalog = useMemo(() => {
    const keys = help.objectProperties(["$defs", props.schemaRoot]);
    return keys.map((key) => {
      const ui = (policyUi as Record<string, { title: string; icon: ComponentType<{ size?: number }>; customEditor?: PolicyEditorKind } | undefined>)[key];
      return {
        key,
        title: ui?.title ?? titleFromKey(key),
        description: help.description(["$defs", props.schemaRoot, "properties", key], "Configured from schema.") ?? "Configured from schema.",
        icon: ui?.icon ?? Shield,
        customEditor: ui?.customEditor,
      };
    });
  }, [help, props.schemaRoot]);
  const selectedMeta = catalog.find((item) => item.key === selected);
  const sorted = catalog
    .map((item) => ({
      ...item,
      enabled: policyEnabled(props.policies, item.key),
      summary: policySummary(props.policies, item.key),
    }))
    .sort((left, right) => Number(right.enabled) - Number(left.enabled) || left.title.localeCompare(right.title));
  const enabledCount = sorted.filter((policy) => policy.enabled).length;

  function setPolicy(key: string, value: unknown) {
    const next = { ...(props.policies ?? {}) };
    if (value === undefined || value === null) delete next[key];
    else next[key] = value;
    props.onChange(Object.keys(next).length ? next : null);
    setSelected(null);
  }

  return (
    <section className={open ? "traffic-policy-section open" : "traffic-policy-section"}>
      <button className="traffic-policy-section-header" type="button" aria-expanded={open} onClick={() => setOpen((current) => !current)}>
        <span className="policy-form-section-icon"><Shield size={17} /></span>
        <span>
          <strong>{props.title}</strong>
          <small>{enabledCount ? `${enabledCount} configured` : "No policies configured"}</small>
        </span>
        <ChevronDown size={17} />
      </button>

      {open ? (
        <div className="traffic-policy-section-body">
          <div className="traffic-policy-grid">
            {sorted.map((policy) => {
              const Icon = policy.icon;
              const active = selected === policy.key;
              return (
                <button
                  className={policy.enabled ? active ? "traffic-policy-card enabled active" : "traffic-policy-card enabled" : active ? "traffic-policy-card active" : "traffic-policy-card"}
                  type="button"
                  key={policy.key}
                  onClick={() => setSelected(policy.key)}
                >
                  <span className="policy-icon"><Icon size={16} /></span>
                  <span>
                    <strong>{policy.title}</strong>
                    <small>{policy.summary || policy.description}</small>
                  </span>
                  {policy.enabled ? <span className="badge ok">Enabled</span> : null}
                </button>
              );
            })}
          </div>

          {selected && selectedMeta ? (
            <section className="traffic-policy-editor">
              <div className="traffic-policy-editor-header">
                <div>
                  <h4>{selectedMeta.title}</h4>
                  <p>{selectedMeta.description}</p>
                </div>
                <div className="button-row">
                  {policyEnabled(props.policies, selected) ? (
                    <button className="button danger" type="button" onClick={() => setPolicy(selected, null)}>Disable</button>
                  ) : null}
                  <button className="icon-button" type="button" aria-label="Close policy editor" onClick={() => setSelected(null)}>
                    <X size={16} />
                  </button>
                </div>
              </div>
              <PolicyEditorBody
                policyKey={selected}
                customEditor={selectedMeta.customEditor}
                policyValue={props.policies?.[selected] ?? null}
                help={help}
                saving={false}
                schemaRoot={props.schemaRoot}
                onSave={(value) => setPolicy(selected, value)}
              />
            </section>
          ) : null}

          {props.policies ? (
            <details className="nested-details">
              <summary>Current policy YAML</summary>
              <YamlBlock value={props.policies} />
            </details>
          ) : null}
        </div>
      ) : null}
      {!catalog.length ? (
        <Tooltip content="No schema properties are available for this policy object">
          <span className="muted">No policy fields</span>
        </Tooltip>
      ) : null}
    </section>
  );
}
