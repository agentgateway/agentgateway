import { Eye, ShieldCheck } from "lucide-react";
import {
  Drawer,
  EmptyState,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { useStickyQueryParam } from "../drawerRouteState";
import { useConfigDumpMode } from "../hooks";
import { ReadonlyModeBanner } from "./traffic/TrafficConfigDumpPanel";

type TargetedPolicy = {
  key: string;
  name?: { kind?: string; namespace?: string; name?: string } | null;
  target?: unknown;
  policy?: unknown;
  inheritance?: unknown;
  [key: string]: unknown;
};

export function DumpPoliciesPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("policy");
  const dumpMode = mode.data?.mode === "dump";
  const policies = (mode.data?.dump?.policies ?? []).filter(isTargetedPolicy);
  const selectedPolicy = policies.find((policy) => policy.key === selectedKey);

  return (
    <div className="page-stack">
      <PageHeader
        title="Policies"
        description="Read-only top-level policies from the active gateway dump."
      />
      <ReadonlyModeBanner />

      <Panel>
        {mode.isLoading ? (
          <StatusBanner state="loading" title="Loading runtime policies" />
        ) : mode.error ? (
          <StatusBanner state="bad" title="Config dump unavailable">
            {mode.error.message}
          </StatusBanner>
        ) : !dumpMode ? (
          <StatusBanner state="warn" title="Readonly policies unavailable">
            Top-level runtime policies are only available when the gateway is
            running from XDS config.
          </StatusBanner>
        ) : !policies.length ? (
          <EmptyState
            title="No top-level policies"
            description="No top-level policies are present in the active gateway dump."
          />
        ) : (
          <div className="table-wrap">
            <table className="dump-policies-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Target</th>
                  <th>Type</th>
                  <th>Inheritance</th>
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {policies.map((policy) => (
                  <tr key={policy.key}>
                    <td>
                      <div className="resource-name-cell">
                        <strong>{policyName(policy)}</strong>
                        <small>{policy.name?.kind ?? "Policy"}</small>
                      </div>
                    </td>
                    <td>{policyTargetLabel(policy.target)}</td>
                    <td>
                      <span className="badge">
                        {policyTypeLabel(policy.policy)}
                      </span>
                    </td>
                    <td>{policyInheritanceLabel(policy.inheritance)}</td>
                    <td className="row-actions">
                      <Tooltip content="View policy">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label={`View ${policyName(policy)}`}
                          onClick={() => setSelectedKey(policy.key)}
                        >
                          <Eye size={16} />
                        </button>
                      </Tooltip>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      {selectedPolicy ? (
        <Drawer
          title={policyName(selectedPolicy)}
          headerActions={
            <span className="badge">
              <ShieldCheck size={14} /> {policyTypeLabel(selectedPolicy.policy)}
            </span>
          }
          onClose={() => setSelectedKey(null)}
        >
          <div className="drawer-summary-list">
            <div>
              <span>Target</span>
              <strong>{policyTargetLabel(selectedPolicy.target)}</strong>
            </div>
            <div>
              <span>Type</span>
              <strong>{policyTypeLabel(selectedPolicy.policy)}</strong>
            </div>
          </div>
          <FieldLabel>Policy YAML</FieldLabel>
          <YamlBlock value={selectedPolicy} />
        </Drawer>
      ) : null}
    </div>
  );
}

function FieldLabel(props: { children: string }) {
  return <label className="field-label">{props.children}</label>;
}

function policyName(policy: TargetedPolicy) {
  return policy.name
    ? `${policy.name.namespace}/${policy.name.name}`
    : policy.key;
}

function policyInheritanceLabel(value: unknown) {
  return typeof value === "string" && value ? value : "default";
}

function policyTypeLabel(policy: unknown) {
  if (!policy || typeof policy !== "object") return "policy";
  const record = policy as Record<string, unknown>;
  const outer = firstPolicyKey(record);
  const inner = outer ? record[outer] : null;
  if (inner && typeof inner === "object") {
    const child = firstPolicyKey(inner as Record<string, unknown>);
    return child ?? outer;
  }
  return outer ?? "policy";
}

function firstPolicyKey(record: Record<string, unknown>) {
  return Object.keys(record).find((key) => !policyMetadataKeys.has(key));
}

const policyMetadataKeys = new Set(["phase", "inheritance"]);

function isTargetedPolicy(value: unknown): value is TargetedPolicy {
  return Boolean(
    value &&
    typeof value === "object" &&
    typeof (value as { key?: unknown }).key === "string",
  );
}

function policyTargetLabel(target: unknown) {
  if (!target || typeof target !== "object") return "unknown target";
  const record = target as Record<string, unknown>;
  if ("gateway" in record) return gatewayTargetLabel(record.gateway);
  if ("route" in record) return routeTargetLabel(record.route);
  if ("backend" in record) return backendTargetLabel(record.backend);
  if ("listenerSet" in record)
    return listenerSetTargetLabel(record.listenerSet);
  return "target";
}

function gatewayTargetLabel(value: unknown) {
  const gateway = value as {
    gatewayName?: string;
    gatewayNamespace?: string;
    listenerName?: string | null;
  } | null;
  if (!gateway) return "Gateway";
  const listener = gateway.listenerName ? ` · ${gateway.listenerName}` : "";
  return `Gateway ${gateway.gatewayNamespace ?? "default"}/${gateway.gatewayName ?? "gateway"}${listener}`;
}

function routeTargetLabel(value: unknown) {
  const route = value as {
    namespace?: string;
    name?: string;
    ruleName?: string | null;
    kind?: string | null;
  } | null;
  if (!route) return "Route";
  const kind = route.kind ? `${route.kind} ` : "Route ";
  const rule = route.ruleName ? ` · ${route.ruleName}` : "";
  return `${kind}${route.namespace ?? "default"}/${route.name ?? "route"}${rule}`;
}

function backendTargetLabel(value: unknown) {
  if (typeof value === "string") return `Backend ${value}`;
  if (!value || typeof value !== "object") return "Backend";
  const backend = value as Record<string, unknown>;
  if ("backend" in backend) {
    const named = backend.backend as {
      namespace?: string;
      name?: string;
      section?: string | null;
    } | null;
    const section = named?.section ? ` · ${named.section}` : "";
    return `Backend ${named?.namespace ?? "default"}/${named?.name ?? "backend"}${section}`;
  }
  if ("service" in backend) {
    const service = backend.service as {
      namespace?: string;
      hostname?: string;
      port?: number | null;
    } | null;
    return `Service ${service?.namespace ?? "default"}/${service?.hostname ?? "service"}${service?.port ? `:${service.port}` : ""}`;
  }
  return "Backend";
}

function listenerSetTargetLabel(value: unknown) {
  const listenerSet = value as {
    namespace?: string;
    name?: string;
    section?: string | null;
  } | null;
  if (!listenerSet) return "ListenerSet";
  const section = listenerSet.section ? ` · ${listenerSet.section}` : "";
  return `ListenerSet ${listenerSet.namespace ?? "default"}/${listenerSet.name ?? "listener-set"}${section}`;
}
