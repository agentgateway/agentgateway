import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import type { ComponentType } from "react";
import { Shield } from "lucide-react";
import { ensureLlm, ensureMcp } from "../config";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import { useSchemaHelp } from "../schemaHelp";
import {
  PageHeader,
  Panel,
  StatusBanner,
  YamlBlock,
} from "../components/Primitives";
import { PolicyDrawer } from "../policies/PolicyDrawer";
import { policyUi } from "../policies/registry";
import type { PolicyKey } from "../policies/types";
import {
  policyEnabled,
  policySummary,
  titleFromKey,
} from "../policies/policyUtils";
import type { GatewayConfig } from "../types";

const llmPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
  {
    title: "Access",
    keys: [
      "cors",
      "apiKey",
      "basicAuth",
      "jwtAuth",
      "oidc",
      "authorization",
      "extAuthz",
    ] as PolicyKey[],
  },
  { title: "Safety", keys: ["guardrails"] as PolicyKey[] },
  {
    title: "Traffic Shaping",
    keys: ["localRateLimit", "remoteRateLimit"] as PolicyKey[],
  },
  { title: "Mutation", keys: ["transformations", "extProc"] as PolicyKey[] },
];

const mcpPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
  {
    title: "MCP",
    keys: [
      "mcpAuthentication",
      "mcpAuthorization",
      "mcpGuardrails",
    ] as PolicyKey[],
  },
  {
    title: "Access",
    keys: ["authorization", "cors", "extAuthz", "jwtAuth"] as PolicyKey[],
  },
  {
    title: "Traffic Shaping",
    keys: ["localRateLimit", "remoteRateLimit"] as PolicyKey[],
  },
  { title: "Mutation", keys: ["transformations", "extProc"] as PolicyKey[] },
];

const mcpPolicyKeys = mcpPolicySections.flatMap((section) => section.keys);

export function PoliciesPage() {
  return (
    <PolicyCatalogPage
      title="LLM Policies"
      description="Configure top-level behavior that applies before model-specific routing."
      schemaRoot="LocalLLMPolicy"
      sections={llmPolicySections}
      yamlDescription="Read-only view of llm.policies."
      policies={(config) =>
        config.data?.llm?.policies as Record<string, unknown> | null | undefined
      }
      managedLinks={{
        apiKey: { to: "/llm/keys", summary: "Managed on Virtual API Keys" },
        guardrails: { to: "/llm/guardrails", summary: "Managed on Guardrails" },
      }}
      onSavePolicy={(next, key, value) => {
        const llm = ensureLlm(next);
        llm.policies ??= {};
        (llm.policies as Record<string, unknown>)[key] = value;
      }}
      onDisablePolicy={(next, key) => {
        const llm = ensureLlm(next);
        if (llm.policies)
          (llm.policies as Record<string, unknown>)[key] =
            key === "localRateLimit" ? undefined : null;
      }}
    />
  );
}

export function McpPoliciesPage() {
  return (
    <PolicyCatalogPage
      title="MCP Policies"
      description="Configure top-level behavior for MCP gateway traffic."
      schemaRoot="FilterOrPolicy"
      sections={mcpPolicySections}
      policyKeys={mcpPolicyKeys}
      yamlDescription="Read-only view of mcp.policies."
      policies={(config) =>
        config.data?.mcp?.policies as Record<string, unknown> | null | undefined
      }
      onSavePolicy={(next, key, value) => {
        const mcp = ensureMcp(next);
        mcp.policies ??= {};
        (mcp.policies as Record<string, unknown>)[key] = value;
      }}
      onDisablePolicy={(next, key) => {
        const mcp = ensureMcp(next);
        if (mcp.policies) delete (mcp.policies as Record<string, unknown>)[key];
      }}
    />
  );
}

function PolicyCatalogPage(props: {
  title: string;
  description: string;
  schemaRoot: string;
  sections: Array<{ title: string; keys: PolicyKey[] }>;
  policyKeys?: PolicyKey[];
  yamlDescription: string;
  policies: (
    config: ReturnType<typeof useGatewayConfig>,
  ) => Record<string, unknown> | null | undefined;
  managedLinks?: Partial<Record<PolicyKey, { to: string; summary: string }>>;
  onSavePolicy: (config: GatewayConfig, key: PolicyKey, value: unknown) => void;
  onDisablePolicy: (config: GatewayConfig, key: PolicyKey) => void;
}) {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const policies = props.policies(config);
  const [selected, setSelected] = useState<PolicyKey | null>(() =>
    policyKeyFromHash(),
  );
  const pendingScrollRestore = useRef<{ x: number; y: number } | null>(null);
  const help = useSchemaHelp();
  const policyCatalog = useMemo(() => {
    const schemaKeys = help.objectProperties([
      "$defs",
      props.schemaRoot,
    ]) as PolicyKey[];
    const keys =
      props.policyKeys ??
      (schemaKeys.length > 0
        ? schemaKeys
        : (Object.keys(policyUi) as PolicyKey[]));
    return keys.map((key) => {
      const policyKey = key as PolicyKey;
      const ui = policyUi[policyKey];
      return {
        key: policyKey,
        title: ui?.title ?? titleFromKey(policyKey),
        description:
          help.propertyDescription(
            props.schemaRoot,
            [policyKey],
            "Configured from schema.",
          ) ?? "Configured from schema.",
        icon: ui?.icon ?? Shield,
        customEditor: ui?.customEditor,
      };
    });
  }, [help, props.policyKeys, props.schemaRoot]);
  const selectedMeta = policyCatalog.find((policy) => policy.key === selected);

  const policyItems = useMemo(() => {
    return policyCatalog.map((meta) => ({
      ...meta,
      enabled: policyEnabled(policies, meta.key),
      summary: policySummary(policies, meta.key),
    }));
  }, [policies, policyCatalog]);
  const groupedPolicyItems = useMemo(() => {
    const byKey = new Map(policyItems.map((policy) => [policy.key, policy]));
    const known = new Set(props.sections.flatMap((section) => section.keys));
    const sections = props.sections
      .map((section) => ({
        ...section,
        policies: section.keys
          .map((key) => byKey.get(key))
          .filter((policy): policy is NonNullable<typeof policy> =>
            Boolean(policy),
          )
          .sort(
            (a, b) =>
              Number(b.enabled) - Number(a.enabled) ||
              section.keys.indexOf(a.key) - section.keys.indexOf(b.key),
          ),
      }))
      .filter((section) => section.policies.length > 0);
    const otherPolicies = policyItems
      .filter((policy) => !known.has(policy.key))
      .sort(
        (a, b) =>
          Number(b.enabled) - Number(a.enabled) ||
          a.title.localeCompare(b.title),
      );
    return otherPolicies.length
      ? [...sections, { title: "Other", keys: [], policies: otherPolicies }]
      : sections;
  }, [policyItems, props.sections]);

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

  useLayoutEffect(() => {
    const scroll = pendingScrollRestore.current;
    if (!scroll) return;
    pendingScrollRestore.current = null;
    window.scrollTo(scroll.x, scroll.y);
  }, [selected]);

  function openPolicy(policyKey: PolicyKey) {
    update.reset();
    setSelected(policyKey);
    setPolicyHash(policyKey, "push");
  }

  function closePolicy() {
    update.reset();
    pendingScrollRestore.current = { x: window.scrollX, y: window.scrollY };
    setSelected(null);
    setPolicyHash(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader title={props.title} description={props.description} />
      {config.isError ? (
        <StatusBanner state="bad" title="Configuration API unavailable">
          {config.error.message}
        </StatusBanner>
      ) : null}
      {update.isError && !selected ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}

      <div className="policy-section-list">
        {groupedPolicyItems.map((section) => (
          <section className="policy-page-section" key={section.title}>
            <h3>{section.title}</h3>
            <div className="policy-page-grid">
              {section.policies.map((policy) => (
                <PolicyTile
                  key={policy.key}
                  policy={policy}
                  managedLink={props.managedLinks?.[policy.key]}
                  onOpen={openPolicy}
                />
              ))}
            </div>
          </section>
        ))}
      </div>

      <details className="schema-details policy-yaml-details">
        <summary>Current top-level policy YAML</summary>
        <Panel>
          <div className="section-heading">
            <p>{props.yamlDescription}</p>
          </div>
          <YamlBlock value={policies ?? {}} />
        </Panel>
      </details>

      {selected && selectedMeta ? (
        <PolicyDrawer
          key={selected}
          policyKey={selected}
          title={selectedMeta.title}
          customEditor={selectedMeta.customEditor}
          policyValue={policies?.[selected] ?? null}
          policies={policies as Record<string, unknown> | null | undefined}
          help={help}
          schemaRoot={props.schemaRoot}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onClose={closePolicy}
          onSave={(value) =>
            update.mutate(
              (next) => {
                props.onSavePolicy(next, selected, value);
              },
              { onSuccess: closePolicy },
            )
          }
          onDisable={() =>
            update.mutate(
              (next) => {
                props.onDisablePolicy(next, selected);
              },
              { onSuccess: closePolicy },
            )
          }
        />
      ) : null}
    </div>
  );
}

function PolicyTile(props: {
  policy: {
    key: PolicyKey;
    title: string;
    enabled: boolean;
    summary: string;
    description: string;
    icon: ComponentType<{ size?: number }>;
  };
  managedLink?: { to: string; summary: string };
  onOpen: (policyKey: PolicyKey) => void;
}) {
  const className = props.policy.enabled
    ? "policy-tile enabled"
    : "policy-tile";
  if (props.managedLink) {
    return (
      <Link className={className} to={props.managedLink.to}>
        <PolicyTileContent
          policy={props.policy}
          summary={props.managedLink.summary}
        />
      </Link>
    );
  }
  return (
    <button
      className={className}
      type="button"
      onClick={() => props.onOpen(props.policy.key)}
    >
      <PolicyTileContent policy={props.policy} />
    </button>
  );
}

function policyKeyFromHash(): PolicyKey | null {
  const raw = decodeURIComponent(window.location.hash.replace(/^#/, ""));
  if (!raw) return null;
  const policy = raw.startsWith("policy=") ? raw.slice("policy=".length) : raw;
  return policy ? (policy as PolicyKey) : null;
}

function setPolicyHash(policyKey: PolicyKey | null, mode: "push" | "replace") {
  const nextUrl = `${window.location.pathname}${window.location.search}${policyKey ? `#${encodeURIComponent(policyKey)}` : ""}`;
  if (
    nextUrl ===
    `${window.location.pathname}${window.location.search}${window.location.hash}`
  )
    return;
  const scrollX = window.scrollX;
  const scrollY = window.scrollY;
  if (mode === "push") {
    History.prototype.pushState.call(window.history, null, "", nextUrl);
  } else {
    History.prototype.replaceState.call(window.history, null, "", nextUrl);
  }
  window.scrollTo(scrollX, scrollY);
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
        <span className="policy-icon">
          <Icon size={18} />
        </span>
        <span className={props.policy.enabled ? "badge ok" : "badge"}>
          {props.policy.enabled ? "enabled" : "disabled"}
        </span>
      </div>
      <strong>{props.policy.title}</strong>
      {summary ? <span>{summary}</span> : null}
      <small>{props.policy.description}</small>
    </>
  );
}
