import {
  ArrowLeft,
  Bot,
  Braces,
  Cable,
  ChevronRight,
  Clock3,
  FileKey2,
  Fingerprint,
  GitBranch,
  KeyRound,
  LockKeyhole,
  Network,
  RefreshCw,
  Route,
  Save,
  Send,
  Server,
  Shield,
  ShieldCheck,
  SlidersHorizontal,
  Timer,
  Trash2,
  Workflow,
} from "lucide-react";
import type { ComponentType } from "react";
import { useMemo, useState } from "react";
import {
  Drawer,
  EmptyState,
  Tooltip,
  YamlBlock,
} from "../../components/Primitives";
import {
  PolicyEditorBody,
  type PolicyEditorKind,
} from "../../policies/PolicyDrawer";
import { policyUi } from "../../policies/registry";
import {
  policyEnabled,
  policySummary,
  titleFromKey,
} from "../../policies/policyUtils";
import { useSchemaHelp } from "../../schemaHelp";

type TrafficPolicySchemaRoot =
  | "LocalGatewayPolicy"
  | "FilterOrPolicy"
  | "TCPFilterOrPolicy"
  | "LocalBackendPolicies"
  | "LocalTCPBackendPolicies";

type TrafficPolicyMeta = {
  key: string;
  title: string;
  description: string;
  icon: ComponentType<{ size?: number }>;
  customEditor?: PolicyEditorKind;
};

type TrafficPolicyItem = TrafficPolicyMeta & {
  enabled: boolean;
  summary: string;
};

const trafficPolicySections: Array<{ title: string; keys: string[] }> = [
  { title: "Backend", keys: ["backendTLS", "backendTunnel", "backendAuth"] },
  {
    title: "Security",
    keys: [
      "cors",
      "apiKey",
      "basicAuth",
      "jwtAuth",
      "oidc",
      "authorization",
      "extAuthz",
      "csrf",
    ],
  },
  {
    title: "Mutation",
    keys: [
      "transformations",
      "extProc",
      "requestHeaderModifier",
      "responseHeaderModifier",
      "directResponse",
      "urlRewrite",
      "requestRedirect",
    ],
  },
  {
    title: "Shaping",
    keys: [
      "localRateLimit",
      "remoteRateLimit",
      "requestMirror",
      "retry",
      "timeout",
      "buffer",
    ],
  },
  {
    title: "AI",
    keys: [
      "mcpAuthorization",
      "mcpGuardrails",
      "mcpAuthentication",
      "a2a",
      "ai",
    ],
  },
  { title: "Routing", keys: ["health", "inferenceRouting", "http", "tcp"] },
];

const trafficPolicyTitles: Record<string, string> = {
  a2a: "A2A",
  ai: "AI",
  backendAuth: "Backend auth",
  backendTLS: "Backend TLS",
  backendTunnel: "Backend tunnel",
  csrf: "CSRF",
  directResponse: "Direct response",
  inferenceRouting: "Inference routing",
  mcpAuthentication: "MCP authentication",
  mcpAuthorization: "MCP authorization",
  mcpGuardrails: "MCP guardrails",
  requestHeaderModifier: "Request headers",
  requestMirror: "Mirror",
  requestRedirect: "Redirect",
  responseHeaderModifier: "Response headers",
  urlRewrite: "URL rewrite",
};

const trafficPolicyIcons: Record<string, ComponentType<{ size?: number }>> = {
  a2a: Bot,
  ai: Bot,
  apiKey: KeyRound,
  authorization: ShieldCheck,
  backendAuth: LockKeyhole,
  backendTLS: FileKey2,
  backendTunnel: Cable,
  basicAuth: LockKeyhole,
  buffer: Server,
  cors: Workflow,
  csrf: Shield,
  directResponse: Send,
  extAuthz: ShieldCheck,
  extProc: SlidersHorizontal,
  health: RefreshCw,
  inferenceRouting: GitBranch,
  jwtAuth: FileKey2,
  localRateLimit: Timer,
  mcpAuthentication: KeyRound,
  mcpAuthorization: ShieldCheck,
  mcpGuardrails: Shield,
  oidc: Fingerprint,
  remoteRateLimit: Braces,
  requestHeaderModifier: SlidersHorizontal,
  requestMirror: Network,
  requestRedirect: Route,
  responseHeaderModifier: SlidersHorizontal,
  retry: RefreshCw,
  timeout: Clock3,
  transformations: Braces,
  urlRewrite: Route,
};

export function TrafficPolicySection(props: {
  title: string;
  schemaRoot: TrafficPolicySchemaRoot;
  policies?: Record<string, unknown> | null;
  onChange: (policies: Record<string, unknown> | null) => void;
}) {
  const help = useSchemaHelp();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const catalog = useMemo(() => {
    return help.objectProperties(["$defs", props.schemaRoot]).map((key) => {
      const ui = (
        policyUi as Record<
          string,
          | {
              title: string;
              icon: ComponentType<{ size?: number }>;
              customEditor?: PolicyEditorKind;
            }
          | undefined
        >
      )[key];
      return {
        key,
        title: trafficPolicyTitles[key] ?? ui?.title ?? titleFromKey(key),
        description:
          help.propertyDescription(
            props.schemaRoot,
            [key],
            "Configured from schema.",
          ) ?? "Configured from schema.",
        icon: trafficPolicyIcons[key] ?? ui?.icon ?? Shield,
        customEditor: ui?.customEditor,
      };
    });
  }, [help, props.schemaRoot]);
  const items = useMemo(() => {
    return catalog.map((policy) => ({
      ...policy,
      enabled: policyEnabled(props.policies, policy.key),
      summary: policySummary(props.policies, policy.key),
    }));
  }, [catalog, props.policies]);
  const selectedMeta = items.find((item) => item.key === selected);
  const selectedFormId = selectedMeta
    ? `traffic-policy-editor-${sanitizePolicyFormId(props.schemaRoot)}-${sanitizePolicyFormId(selectedMeta.key)}`
    : undefined;
  const enabled = items.filter((item) => item.enabled);
  const grouped = useMemo(() => groupTrafficPolicies(items), [items]);

  function setPolicy(key: string, value: unknown) {
    const next = { ...(props.policies ?? {}) };
    if (value === undefined || value === null) delete next[key];
    else next[key] = value;
    props.onChange(Object.keys(next).length ? next : null);
    setSelected(null);
  }

  function closeDrawer() {
    setDrawerOpen(false);
    setSelected(null);
  }

  return (
    <section className="traffic-policy-section">
      <button
        className="traffic-policy-summary"
        type="button"
        onClick={() => setDrawerOpen(true)}
      >
        <span className="policy-form-section-icon">
          <Shield size={17} />
        </span>
        <span>
          <strong>{props.title}</strong>
          <small>
            {enabled.length
              ? enabled
                  .map(
                    (policy) =>
                      `${policy.title}${policy.summary ? `: ${policy.summary}` : ""}`,
                  )
                  .join(" · ")
              : "No policies configured"}
          </small>
        </span>
        <ChevronRight size={17} />
      </button>

      {drawerOpen ? (
        <Drawer
          title={selectedMeta ? selectedMeta.title : props.title}
          variant="nested"
          onClose={closeDrawer}
          headerActions={
            selectedMeta ? (
              <Tooltip
                content={
                  policyEnabled(props.policies, selectedMeta.key)
                    ? "Delete policy"
                    : "Policy is not enabled"
                }
              >
                <button
                  className="icon-button danger"
                  type="button"
                  aria-label="Delete policy"
                  disabled={!policyEnabled(props.policies, selectedMeta.key)}
                  onClick={() => setPolicy(selectedMeta.key, null)}
                >
                  <Trash2 size={17} />
                </button>
              </Tooltip>
            ) : undefined
          }
          footer={
            selectedMeta && selectedFormId ? (
              <button
                className="button primary"
                type="submit"
                form={selectedFormId}
              >
                <Save size={16} />
                Save
              </button>
            ) : undefined
          }
        >
          {selectedMeta ? (
            <TrafficPolicyEditor
              formId={selectedFormId}
              policy={selectedMeta}
              schemaRoot={props.schemaRoot}
              policies={props.policies}
              onBack={() => setSelected(null)}
              onSave={(value) => setPolicy(selectedMeta.key, value)}
            />
          ) : (
            <TrafficPolicyCatalog
              title={props.title}
              grouped={grouped}
              policies={props.policies}
              onOpen={setSelected}
            />
          )}
        </Drawer>
      ) : null}
    </section>
  );
}

function TrafficPolicyCatalog(props: {
  title: string;
  grouped: Array<{ title: string; policies: TrafficPolicyItem[] }>;
  policies?: Record<string, unknown> | null;
  onOpen: (key: string) => void;
}) {
  return (
    <div className="traffic-policy-drawer-stack">
      {props.grouped.length ? (
        props.grouped.map((section) => (
          <section
            className="policy-page-section traffic-policy-catalog-section"
            key={section.title}
          >
            <h3>{section.title}</h3>
            <div className="traffic-policy-catalog-grid">
              {section.policies.map((policy) => (
                <TrafficPolicyTile
                  key={policy.key}
                  policy={policy}
                  onOpen={props.onOpen}
                />
              ))}
            </div>
          </section>
        ))
      ) : (
        <EmptyState
          title="No policy fields"
          description="No schema properties are available for this policy object."
        />
      )}
      {props.policies ? (
        <details className="nested-details">
          <summary>Current policy YAML</summary>
          <YamlBlock value={props.policies} />
        </details>
      ) : null}
    </div>
  );
}

function TrafficPolicyTile(props: {
  policy: TrafficPolicyItem;
  onOpen: (key: string) => void;
}) {
  const Icon = props.policy.icon;
  return (
    <button
      className={
        props.policy.enabled
          ? "traffic-policy-list-item enabled"
          : "traffic-policy-list-item"
      }
      type="button"
      onClick={() => props.onOpen(props.policy.key)}
    >
      <span className="policy-icon">
        <Icon size={17} />
      </span>
      <span>
        <strong>{props.policy.title}</strong>
        <small>{props.policy.summary || props.policy.description}</small>
      </span>
      <span className={props.policy.enabled ? "badge ok" : "badge"}>
        {props.policy.enabled ? "enabled" : "disabled"}
      </span>
    </button>
  );
}

function TrafficPolicyEditor(props: {
  formId?: string;
  policy: TrafficPolicyItem;
  schemaRoot: TrafficPolicySchemaRoot;
  policies?: Record<string, unknown> | null;
  onBack: () => void;
  onSave: (value: unknown) => void;
}) {
  const help = useSchemaHelp();
  return (
    <div className="traffic-policy-drawer-stack">
      <div className="traffic-policy-editor-topbar">
        <button className="button" type="button" onClick={props.onBack}>
          <ArrowLeft size={16} />
          Policies
        </button>
      </div>
      <div className="section-heading">
        <p>{props.policy.description}</p>
      </div>
      <PolicyEditorBody
        formId={props.formId}
        policyKey={props.policy.key}
        customEditor={props.policy.customEditor}
        policyValue={props.policies?.[props.policy.key] ?? null}
        help={help}
        saving={false}
        schemaRoot={props.schemaRoot}
        showGenericSchemaDescription={false}
        onSave={props.onSave}
      />
    </div>
  );
}

function sanitizePolicyFormId(value: string) {
  return value.replace(/[^A-Za-z0-9_-]+/g, "-");
}

function groupTrafficPolicies(items: TrafficPolicyItem[]) {
  const byKey = new Map(items.map((policy) => [policy.key, policy]));
  const known = new Set(
    trafficPolicySections.flatMap((section) => section.keys),
  );
  const sections = trafficPolicySections
    .map((section) => ({
      title: section.title,
      policies: section.keys
        .map((key) => byKey.get(key))
        .filter((policy): policy is TrafficPolicyItem => Boolean(policy))
        .sort(
          (a, b) =>
            Number(b.enabled) - Number(a.enabled) ||
            section.keys.indexOf(a.key) - section.keys.indexOf(b.key),
        ),
    }))
    .filter((section) => section.policies.length > 0);
  const other = items
    .filter((policy) => !known.has(policy.key))
    .sort(
      (a, b) =>
        Number(b.enabled) - Number(a.enabled) || a.title.localeCompare(b.title),
    );
  return other.length
    ? [...sections, { title: "Other", policies: other }]
    : sections;
}
