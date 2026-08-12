// Shared helpers for read-only, XDS-config-dump-sourced policy views.
// Duplicated (not imported) from DumpPolicies.tsx deliberately: that page is
// already working in production and is left untouched to avoid regression
// risk. These are pure, side-effect-free formatting helpers only.

export type TargetedPolicy = {
  key: string;
  name?: { kind?: string; namespace?: string; name?: string } | null;
  target?: unknown;
  policy?: unknown;
  inheritance?: unknown;
  [key: string]: unknown;
};

export function isTargetedPolicy(value: unknown): value is TargetedPolicy {
  return Boolean(
    value &&
      typeof value === "object" &&
      typeof (value as { key?: unknown }).key === "string",
  );
}

export function policyName(policy: TargetedPolicy) {
  return policy.name
    ? `${policy.name.namespace}/${policy.name.name}`
    : policy.key;
}

export function policyInheritanceLabel(value: unknown) {
  return typeof value === "string" && value ? value : "default";
}

const policyMetadataKeys = new Set(["phase", "inheritance"]);

function firstPolicyKey(record: Record<string, unknown>) {
  return Object.keys(record).find((key) => !policyMetadataKeys.has(key));
}

export function policyTypeLabel(policy: unknown) {
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

export function policyTargetLabel(target: unknown) {
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

/**
 * Best-effort classification of a policy's "family" for the purposes of the
 * Guardrails / Virtual API Keys filtered views. Matches on the type label
 * agentgateway itself reports in the dump (via policyTypeLabel), using
 * case-insensitive substring matching since the exact runtime key name isn't
 * guaranteed to match the CRD's Go json tag 1:1 (the dump is produced by the
 * Rust data plane, not the Go controller).
 */
export function policyMatchesKeywords(policy: TargetedPolicy, keywords: string[]) {
  const label = (policyTypeLabel(policy.policy) ?? "policy").toLowerCase();
  const key = policy.key.toLowerCase();
  return keywords.some((kw) => label.includes(kw) || key.includes(kw));
}
