import { useEffect, useMemo, useState } from "react";
import { Link } from "../_adapters/_router";
import type { ComponentType } from "react";
import { Shield } from "lucide-react";
import { ensureLlm } from "../config";
import { useGatewayConfig, useUpdateConfig } from "../_adapters/hooks";
import { useSchemaHelp } from "../schemaHelp";
import { PageHeader, Panel, StatusBanner, YamlBlock } from "../components/Primitives";
import { PolicyDrawer } from "../policies/PolicyDrawer";
import { policyUi } from "../policies/registry";
import type { PolicyKey } from "../policies/types";
import { policyEnabled, policySummary, titleFromKey } from "../policies/policyUtils";

export function PoliciesPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const policies = config.data?.llm?.policies;
  const [selected, setSelected] = useState<PolicyKey | null>(() => policyKeyFromHash());
  const help = useSchemaHelp();
  const policyCatalog = useMemo(() => {
    const schemaKeys = help.objectProperties(["$defs", "LocalLLMPolicy"]);
    const keys = schemaKeys.length > 0 ? schemaKeys : Object.keys(policyUi);
    return keys.map((key) => {
      const policyKey = key as PolicyKey;
      const ui = policyUi[policyKey];
      return {
        key: policyKey,
        title: ui?.title ?? titleFromKey(policyKey),
        description: help.description(["$defs", "LocalLLMPolicy", "properties", policyKey], "Configured from schema.") ?? "Configured from schema.",
        icon: ui?.icon ?? Shield,
        customEditor: ui?.customEditor,
      };
    });
  }, [help]);
  const selectedMeta = policyCatalog.find((policy) => policy.key === selected);

  const policyItems = useMemo(() => {
    return policyCatalog
      .map((meta) => ({
        ...meta,
        enabled: policyEnabled(policies, meta.key),
        summary: policySummary(policies, meta.key),
      }))
      .sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.title.localeCompare(b.title));
  }, [policies, policyCatalog]);

  useEffect(() => {
    function syncSelectedFromUrl() {
      update.reset();
      setSelected(policyKeyFromHash());
    }
    window.addEventListener("hashchange", syncSelectedFromUrl);
    window.addEventListener("popstate", syncSelectedFromUrl);
    return () => {
      window.removeEventListener("hashchange", syncSelectedFromUrl);
      window.removeEventListener("popstate", syncSelectedFromUrl);
    };
  }, [update]);

  function openPolicy(policyKey: PolicyKey) {
    update.reset();
    setSelected(policyKey);
    setPolicyHash(policyKey, "push");
  }

  function closePolicy() {
    update.reset();
    setSelected(null);
    setPolicyHash(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Policies"
        description="Configure top-level behavior that applies before model-specific routing."
      />
      {config.isError ? <StatusBanner state="bad" title="Configuration API unavailable">{config.error!.message}</StatusBanner> : null}
      {update.isError && !selected ? <StatusBanner state="bad" title="Save failed">{update.error!.message}</StatusBanner> : null}

      <div className="policy-page-grid">
        {policyItems.map((policy) => (
          policy.key === "apiKey" ? (
            <Link className={policy.enabled ? "policy-tile enabled" : "policy-tile"} key={policy.key} to="/llm/keys">
              <PolicyTileContent policy={policy} summary="Managed on Virtual API Keys" />
            </Link>
          ) : (
            <button className={policy.enabled ? "policy-tile enabled" : "policy-tile"} type="button" key={policy.key} onClick={() => {
              openPolicy(policy.key);
            }}>
              <PolicyTileContent policy={policy} />
            </button>
          )
        ))}
      </div>

      <Panel>
        <div className="section-heading">
          <h3>Current top-level policy YAML</h3>
          <p>Read-only view of llm.policies.</p>
        </div>
        <YamlBlock value={policies ?? {}} />
      </Panel>

      {selected && selectedMeta ? (
        <PolicyDrawer
          key={selected}
          policyKey={selected}
          title={selectedMeta.title}
          customEditor={selectedMeta.customEditor}
          policyValue={policies?.[selected] ?? null}
          policies={policies as Record<string, unknown> | null | undefined}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error!.message : null}
          onClose={closePolicy}
          onSave={(value) => update.mutate((next) => {
            const llm = ensureLlm(next);
            llm.policies ??= {};
            (llm.policies as Record<string, unknown>)[selected] = value;
          }, { onSuccess: closePolicy })}
          onDisable={() => update.mutate((next) => {
            const llm = ensureLlm(next);
            if (llm.policies) (llm.policies as Record<string, unknown>)[selected] = selected === "localRateLimit" ? undefined : null;
          }, { onSuccess: closePolicy })}
        />
      ) : null}
    </div>
  );
}

function policyKeyFromHash(): PolicyKey | null {
  const raw = decodeURIComponent(window.location.hash.replace(/^#/, ""));
  if (!raw) return null;
  const policy = raw.startsWith("policy=") ? raw.slice("policy=".length) : raw;
  return policy ? policy as PolicyKey : null;
}

function setPolicyHash(policyKey: PolicyKey | null, mode: "push" | "replace") {
  const nextUrl = `${window.location.pathname}${window.location.search}${policyKey ? `#${encodeURIComponent(policyKey)}` : ""}`;
  if (nextUrl === `${window.location.pathname}${window.location.search}${window.location.hash}`) return;
  if (mode === "push") {
    window.history.pushState(null, "", nextUrl);
  } else {
    window.history.replaceState(null, "", nextUrl);
  }
}

function PolicyTileContent(props: {
  policy: {
    title: string;
    enabled: boolean;
    summary: string;
    description: string;
    icon: ComponentType<{ size?: number }>;
  };
  summary?: string;
}) {
  const Icon = props.policy.icon;
  const summary = props.summary ?? props.policy.summary;
  return (
    <>
      <div className="policy-tile-header">
        <span className="policy-icon"><Icon size={18} /></span>
        <span className={props.policy.enabled ? "badge ok" : "badge"}>{props.policy.enabled ? "enabled" : "disabled"}</span>
      </div>
      <strong>{props.policy.title}</strong>
      {summary ? <span>{summary}</span> : null}
      <small>{props.policy.description}</small>
    </>
  );
}
