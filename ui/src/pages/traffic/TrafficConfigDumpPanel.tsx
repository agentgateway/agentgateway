import { tr } from "../../i18n";
import { Eye } from "lucide-react";
import { Link } from "@tanstack/react-router";
import {
  Drawer,
  EmptyState,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../../components/Primitives";
import { useStickyQueryParam } from "../../drawerRouteState";
import type {
  DumpBind,
  DumpListener,
  Route,
  TCPRoute,
} from "../../gateway-admin";
import type { AdminConfigDump } from "../../types";

type BackendWithPolicies = {
  backend?: unknown;
  inlinePolicies?: unknown[];
  [key: string]: unknown;
};

type Service = {
  namespace?: string;
  hostname?: string;
  [key: string]: unknown;
};

type TargetedPolicy = {
  key: string;
  name?: { namespace?: string; name?: string } | null;
  target?: unknown;
  policy?: unknown;
  inheritance?: unknown;
  [key: string]: unknown;
};

export type DumpListenerRow = {
  bind: DumpBind;
  listener: DumpListener;
};

export type DumpRouteRow = {
  type: "HTTP" | "TCP";
  source: "listener" | "mesh" | "route group";
  bind?: DumpBind;
  listener?: DumpListener;
  route: Route | TCPRoute;
};

export function ReadonlyModeBanner() {
  return (
    <div className="readonly-mode-banner">
      <strong>{tr("copy.readonlyMode")}</strong>
      <span>
        {tr(
          "copy.configurationIsManagedByXdsThisViewReflectsTheActiveRuntimeDumpEditingIsDisabled",
        )}
      </span>
    </div>
  );
}

export function TrafficDumpOverview(props: {
  dump?: AdminConfigDump | null;
  isLoading?: boolean;
  error?: Error | null;
}) {
  const inventory = props.dump ? buildTrafficInventory(props.dump) : null;
  return (
    <Panel className="traffic-dump-panel">
      <div className="traffic-dump-header">
        <div>
          <h3>{tr("copy.runtimeTraffic")}</h3>
          <p>{tr("copy.activeRuntimeResourcesFromTheGatewayDump")}</p>
        </div>
      </div>
      {props.isLoading ? (
        <StatusBanner
          state="loading"
          title={tr("copy.loadingRuntimeTrafficConfiguration")}
        />
      ) : props.error ? (
        <StatusBanner state="bad" title={tr("copy.configDumpUnavailable")}>
          {props.error.message}
        </StatusBanner>
      ) : inventory && hasTrafficInventory(inventory) ? (
        <div className="traffic-dump-link-list">
          <Link to="/traffic/listeners">
            <span>{tr("copy.listeners")}</span>
            <strong>{inventory.listeners.length}</strong>
          </Link>
          <Link to="/traffic/routes">
            <span>{tr("copy.routes_14u6307")}</span>
            <strong>{inventory.routes.length}</strong>
          </Link>
          <Link to="/traffic/policies">
            <span>{tr("copy.policies_raqot3")}</span>
            <strong>{inventory.policies.length}</strong>
          </Link>
        </div>
      ) : (
        <StatusBanner
          state="warn"
          title={tr("copy.noRuntimeTrafficConfiguration")}
        />
      )}
    </Panel>
  );
}

export function TrafficDumpListenersView(props: {
  dump?: AdminConfigDump | null;
  isLoading?: boolean;
  error?: Error | null;
}) {
  const inventory = props.dump ? buildTrafficInventory(props.dump) : null;
  const [selectedListenerKey, setSelectedListenerKey] =
    useStickyQueryParam("listener");
  const selectedListener = inventory?.listeners.find(
    ({ listener }) => listener.key === selectedListenerKey,
  );
  return (
    <Panel>
      {props.isLoading ? (
        <StatusBanner
          state="loading"
          title={tr("copy.loadingTrafficListeners")}
        />
      ) : props.error ? (
        <StatusBanner state="bad" title={tr("copy.configDumpUnavailable")}>
          {props.error.message}
        </StatusBanner>
      ) : !inventory?.listeners.length ? (
        <EmptyState
          title={tr("copy.noRuntimeListeners")}
          description={tr("copy.noListenersArePresentInTheActiveGatewayDump")}
        />
      ) : (
        <div className="traffic-bind-list">
          {props.dump?.binds.map((bind) => {
            const listeners = Object.values(bind.listeners ?? {});
            return (
              <section className="traffic-bind readonly" key={bind.key}>
                <div className="traffic-bind-header">
                  <div>
                    <h3>{bindDisplayName(bind.address)}</h3>
                    <p>
                      {listeners.length}
                      {tr("copy.listeners_1fzojr3")}
                      {listenerRouteCount(bind)} {tr("copy.routes")}
                    </p>
                  </div>
                  <div className="button-row">
                    <span className="badge">
                      {bind.tunnelProtocol ?? "direct"}
                    </span>
                    <span className="badge">
                      {bindProtocolLabel(bind.protocol)}
                    </span>
                  </div>
                </div>
                {listeners.length ? (
                  <RuntimeListenerTable
                    rows={listeners.map((listener) => ({ bind, listener }))}
                    onSelect={(listener) =>
                      setSelectedListenerKey(listener.key)
                    }
                  />
                ) : (
                  <EmptyState
                    title={tr("copy.noListenersOnThisBind")}
                    description={tr(
                      "copy.noListenersAreAttachedToThisBindInTheRuntimeDump",
                    )}
                  />
                )}
              </section>
            );
          })}
        </div>
      )}
      {selectedListener ? (
        <ListenerDumpDrawer
          row={selectedListener}
          onClose={() => setSelectedListenerKey(null)}
        />
      ) : null}
    </Panel>
  );
}

export function TrafficDumpRoutesView(props: {
  dump?: AdminConfigDump | null;
  isLoading?: boolean;
  error?: Error | null;
}) {
  const inventory = props.dump ? buildTrafficInventory(props.dump) : null;
  const [selectedRouteKey, setSelectedRouteKey] = useStickyQueryParam("route");
  const selectedRoute = inventory?.routes.find(
    (row) => row.route.key === selectedRouteKey,
  );
  return (
    <Panel>
      {props.isLoading ? (
        <StatusBanner state="loading" title={tr("copy.loadingTrafficRoutes")} />
      ) : props.error ? (
        <StatusBanner state="bad" title={tr("copy.configDumpUnavailable")}>
          {props.error.message}
        </StatusBanner>
      ) : !inventory?.routes.length ? (
        <EmptyState
          title={tr("copy.noRuntimeRoutes")}
          description={tr("copy.noRoutesArePresentInTheActiveGatewayDump")}
        />
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{tr("copy.name")}</th>
                <th>{tr("copy.type")}</th>
                <th>{tr("copy.listener")}</th>
                <th>{tr("copy.match")}</th>
                <th>{tr("copy.backends_i9thuc")}</th>
                <th>{tr("copy.policies_raqot3")}</th>
                <th aria-label={tr("copy.actions")} />
              </tr>
            </thead>
            <tbody>
              {inventory.routes.map((row) => {
                const backends = row.route.backends ?? [];
                const policies = routeInlinePolicies(row.route);
                const backendPolicies = backends.flatMap(
                  (backend) => backend.inlinePolicies ?? [],
                );
                return (
                  <tr key={`${row.source}-${row.type}-${row.route.key}`}>
                    <td className="strong">{routeDisplayName(row.route)}</td>
                    <td>
                      <span className="badge">{row.type}</span>
                    </td>
                    <td>{routeListenerCell(row)}</td>
                    <td>{routeMatchSummary(row)}</td>
                    <td>{backendListSummary(backends)}</td>
                    <td>{policies.length + backendPolicies.length}</td>
                    <td className="row-actions">
                      <Tooltip content="View route">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label={tr("copy.viewValue")}
                          onClick={() => setSelectedRouteKey(row.route.key)}
                        >
                          <Eye size={16} />
                        </button>
                      </Tooltip>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {selectedRoute ? (
        <RouteDumpDrawer
          row={selectedRoute}
          backends={inventory?.backends ?? []}
          services={(props.dump?.services ?? []).filter(isService)}
          onClose={() => setSelectedRouteKey(null)}
        />
      ) : null}
    </Panel>
  );
}

function RuntimeListenerTable(props: {
  rows: DumpListenerRow[];
  onSelect: (listener: DumpListener) => void;
}) {
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>{tr("copy.name")}</th>
            <th>{tr("copy.hostname")}</th>
            <th>{tr("copy.protocol")}</th>
            <th>{tr("copy.routes_14u6307")}</th>
            <th>{tr("copy.backends_i9thuc")}</th>
            <th aria-label={tr("copy.actions")} />
          </tr>
        </thead>
        <tbody>
          {props.rows.map(({ listener }) => (
            <tr key={listener.key}>
              <td className="strong">
                {listener.listenerName || listener.key}
              </td>
              <td>{listener.hostname || "*"}</td>
              <td>
                <span className="badge">
                  {listenerProtocolLabel(listener.protocol)}
                </span>
              </td>
              <td>{listenerRouteObjectCount(listener)}</td>
              <td>{listenerBackendCount(listener)}</td>
              <td className="row-actions">
                <Tooltip content="View listener">
                  <button
                    className="icon-button"
                    type="button"
                    aria-label={tr("copy.viewValue")}
                    onClick={() => props.onSelect(listener)}
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
  );
}

function ListenerDumpDrawer(props: {
  row: DumpListenerRow;
  onClose: () => void;
}) {
  const listener = props.row.listener;
  const routeCount = listenerRouteObjectCount(listener);
  const backendCount = listenerBackendCount(listener);
  return (
    <Drawer
      title={listener.listenerName || listener.key}
      onClose={props.onClose}
    >
      <div className="drawer-summary-list">
        <div>
          <span>{tr("copy.bind")}</span>
          <strong>{bindDisplayName(props.row.bind.address)}</strong>
        </div>
        <div>
          <span>{tr("copy.hostname")}</span>
          <strong>{listener.hostname || "*"}</strong>
        </div>
        <div>
          <span>{tr("copy.protocol")}</span>
          <strong>{listenerProtocolLabel(listener.protocol)}</strong>
        </div>
        <div>
          <span>{tr("copy.routes_14u6307")}</span>
          <strong>{routeCount}</strong>
        </div>
        <div>
          <span>{tr("copy.backends_i9thuc")}</span>
          <strong>{backendCount}</strong>
        </div>
      </div>
      <div className="drawer-yaml-section">
        <label className="field-label">{tr("copy.listenerYaml")}</label>
        <YamlBlock value={listenerDumpForDisplay(listener)} />
      </div>
    </Drawer>
  );
}

function RouteDumpDrawer(props: {
  row: DumpRouteRow;
  backends: BackendWithPolicies[];
  services: Service[];
  onClose: () => void;
}) {
  const backends = props.row.route.backends ?? [];
  const policies = routeInlinePolicies(props.row.route);
  const backendPolicies = backends.flatMap(
    (backend) => backend.inlinePolicies ?? [],
  );
  const resolvedBackends = resolveRouteBackends(
    backends,
    props.backends,
    props.services,
  );
  return (
    <Drawer title={routeDisplayName(props.row.route)} onClose={props.onClose}>
      <div className="drawer-summary-list">
        <div>
          <span>{tr("copy.type")}</span>
          <strong>{props.row.type}</strong>
        </div>
        <div>
          <span>{tr("copy.listener")}</span>
          <strong>
            {props.row.listener
              ? listenerDisplayName(props.row.listener)
              : props.row.source}
          </strong>
        </div>
        <div>
          <span>{tr("copy.match")}</span>
          <strong>{routeMatchSummary(props.row)}</strong>
        </div>
        <div>
          <span>{tr("copy.backends_i9thuc")}</span>
          <strong>{backends.length}</strong>
        </div>
        <div>
          <span>{tr("copy.policies_raqot3")}</span>
          <strong>{policies.length + backendPolicies.length}</strong>
        </div>
      </div>
      <div className="drawer-yaml-section">
        <label className="field-label">{tr("copy.routeYaml")}</label>
        <YamlBlock value={props.row.route} />
      </div>
      {resolvedBackends.length ? (
        <div className="drawer-yaml-section">
          <label className="field-label">{tr("copy.backendYaml")}</label>
          <YamlBlock
            value={
              resolvedBackends.length === 1
                ? resolvedBackends[0]
                : resolvedBackends
            }
          />
        </div>
      ) : null}
    </Drawer>
  );
}

export function buildTrafficInventory(dump: AdminConfigDump) {
  const listeners = (dump.binds ?? []).flatMap((bind) =>
    Object.values(bind.listeners ?? {}).map((listener) => ({ bind, listener })),
  );
  const routeKeys = new Set<string>();
  const routes: DumpRouteRow[] = [];
  for (const { bind, listener } of listeners) {
    for (const route of Object.values(listener.routes ?? {})) {
      routeKeys.add(route.key);
      routes.push({ type: "HTTP", source: "listener", bind, listener, route });
    }
    for (const route of Object.values(listener.tcpRoutes ?? {})) {
      routeKeys.add(route.key);
      routes.push({ type: "TCP", source: "listener", bind, listener, route });
    }
  }
  addRouteDump(routes, routeKeys, dump.routes?.httpMesh, "HTTP", "mesh");
  addRouteDump(routes, routeKeys, dump.routes?.tcpMesh, "TCP", "mesh");
  addRouteDump(
    routes,
    routeKeys,
    dump.routes?.routeGroups,
    "HTTP",
    "route group",
  );
  return {
    listeners,
    routes,
    policies: (dump.policies ?? []).filter(isTargetedPolicy),
    backends: (dump.backends ?? []).filter(isBackendWithPolicies),
  };
}

function isTargetedPolicy(value: unknown): value is TargetedPolicy {
  return Boolean(
    value &&
    typeof value === "object" &&
    typeof (value as { key?: unknown }).key === "string",
  );
}

function isBackendWithPolicies(value: unknown): value is BackendWithPolicies {
  return Boolean(value && typeof value === "object" && "backend" in value);
}

function isService(value: unknown): value is Service {
  return Boolean(
    value &&
    typeof value === "object" &&
    typeof (value as { hostname?: unknown }).hostname === "string",
  );
}

function addRouteDump(
  rows: DumpRouteRow[],
  seen: Set<string>,
  dump: Record<string, Record<string, Route | TCPRoute>> | undefined,
  type: "HTTP" | "TCP",
  source: DumpRouteRow["source"],
) {
  for (const group of Object.values(dump ?? {})) {
    for (const route of Object.values(group ?? {})) {
      if (seen.has(route.key)) continue;
      seen.add(route.key);
      rows.push({ type, source, route });
    }
  }
}

export function hasTrafficInventory(
  inventory: ReturnType<typeof buildTrafficInventory>,
) {
  return (
    inventory.listeners.length > 0 ||
    inventory.routes.length > 0 ||
    inventory.policies.length > 0 ||
    inventory.backends.length > 0
  );
}

function listenerProtocolLabel(protocol: DumpListener["protocol"]) {
  if (typeof protocol === "string") return protocol;
  return Object.keys(protocol)[0] ?? "unknown";
}

function bindProtocolLabel(protocol: DumpBind["protocol"]) {
  if (typeof protocol === "string") return protocol;
  return Object.keys(protocol)[0] ?? "unknown";
}

function bindDisplayName(address: string) {
  const port = bindPort(address);
  return port ? `Port ${port}` : address;
}

function bindPort(address: string) {
  const bracketPort = address.match(/^\[[^\]]+\]:(\d+)$/)?.[1];
  if (bracketPort) return bracketPort;
  const suffixPort = address.match(/:(\d+)$/)?.[1];
  if (suffixPort) return suffixPort;
  return null;
}

function routeDisplayName(route: Route | TCPRoute) {
  return route.ruleName || route.name || route.key;
}

function listenerDisplayName(listener: DumpListener) {
  return listener.listenerName || listener.key;
}

function routeListenerCell(row: DumpRouteRow) {
  if (!row.listener) return row.source;
  return (
    <Link
      className="table-link"
      to="/traffic/listeners"
      search={{ listener: row.listener.key }}
    >
      {listenerDisplayName(row.listener)}
    </Link>
  );
}

function routeInlinePolicies(route: Route | TCPRoute): unknown[] {
  const policies = (route as { inlinePolicies?: unknown[] }).inlinePolicies;
  return Array.isArray(policies) ? policies : [];
}

function routeMatchSummary(row: DumpRouteRow) {
  if (row.type === "TCP") return hostnamesSummary(row.route.hostnames);
  const route = row.route as Route;
  const first = route.matches?.[0];
  const path = first?.path;
  if (!path || path === "invalid") return hostnamesSummary(route.hostnames);
  if ("exact" in path) return path.exact;
  if ("regex" in path) return path.regex;
  return path.pathPrefix;
}

function listenerRouteCount(bind: DumpBind) {
  return Object.values(bind.listeners ?? {}).reduce(
    (total, listener) =>
      total +
      Object.keys(listener.routes ?? {}).length +
      Object.keys(listener.tcpRoutes ?? {}).length,
    0,
  );
}

function listenerBackendCount(listener: DumpListener) {
  const http = Object.values(listener.routes ?? {}).reduce(
    (total, route) => total + (route.backends?.length ?? 0),
    0,
  );
  const tcp = Object.values(listener.tcpRoutes ?? {}).reduce(
    (total, route) => total + (route.backends?.length ?? 0),
    0,
  );
  return http + tcp;
}

function listenerRouteObjectCount(listener: DumpListener) {
  return (
    Object.keys(listener.routes ?? {}).length +
    Object.keys(listener.tcpRoutes ?? {}).length
  );
}

function listenerDumpForDisplay(listener: DumpListener) {
  const next: Record<string, unknown> = { ...listener };
  delete next.routes;
  delete next.tcpRoutes;
  return next;
}

function resolveRouteBackends(
  routeBackends: unknown[],
  availableBackends: BackendWithPolicies[],
  availableServices: Service[],
) {
  const byName = new Map<string, BackendWithPolicies>();
  for (const backend of availableBackends) {
    const name = backendReferenceName(backend.backend);
    if (name) byName.set(name, backend);
  }
  const servicesByName = new Map<string, Service>();
  for (const service of availableServices) {
    servicesByName.set(
      normalizeBackendName(`${service.namespace}/${service.hostname}`),
      service,
    );
  }
  return routeBackends
    .map((backend) => {
      const serviceName = routeServiceReferenceName(backend);
      if (serviceName) return servicesByName.get(serviceName);
      const backendName = routeBackendReferenceName(backend);
      return backendName ? byName.get(backendName) : undefined;
    })
    .filter((backend): backend is BackendWithPolicies | Service =>
      Boolean(backend),
    );
}

function routeBackendReferenceName(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const backend = (value as Record<string, unknown>).backend;
  return typeof backend === "string" ? normalizeBackendName(backend) : null;
}

function routeServiceReferenceName(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const service = (value as Record<string, unknown>).service;
  if (!service || typeof service !== "object") return null;
  const name = (service as Record<string, unknown>).name;
  if (typeof name === "string") return normalizeBackendName(name);
  if (!name || typeof name !== "object") return null;
  const record = name as Record<string, unknown>;
  const namespace =
    typeof record.namespace === "string" ? record.namespace : "default";
  const hostname = typeof record.hostname === "string" ? record.hostname : null;
  return hostname ? normalizeBackendName(`${namespace}/${hostname}`) : null;
}

function backendReferenceName(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const backendRecord = value as Record<string, unknown>;
  const kind = Object.keys(backendRecord)[0];
  if (!kind) return null;
  const payload = backendRecord[kind];
  if (Array.isArray(payload)) {
    for (const part of payload) {
      const name = resourceName(part);
      if (name) return name;
    }
    return null;
  }
  return resourceName(payload);
}

function resourceName(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const namespace =
    typeof record.namespace === "string" ? record.namespace : "default";
  const name =
    typeof record.name === "string"
      ? record.name
      : typeof record.hostname === "string"
        ? record.hostname
        : null;
  return name ? normalizeBackendName(`${namespace}/${name}`) : null;
}

function normalizeBackendName(value: string) {
  return value.includes("/") ? value : `default/${value}`;
}

function backendListSummary(backends: unknown[]) {
  if (!backends.length) return "none";
  const labels = backends.map(routeBackendLabel);
  return (
    labels.slice(0, 2).join(", ") +
    (labels.length > 2 ? ` +${labels.length - 2}` : "")
  );
}

function routeBackendLabel(value: unknown) {
  if (!value || typeof value !== "object") return "unknown";
  const backend = value as Record<string, unknown>;
  if ("backend" in backend) return backendKind(backend.backend);
  if ("service" in backend) {
    const service = backend.service as {
      name?: { namespace?: string; hostname?: string };
      port?: number;
    } | null;
    const namespace = service?.name?.namespace;
    const hostname = service?.name?.hostname;
    const port = service?.port;
    return `${namespace ? `${namespace}/` : ""}${hostname ?? "service"}${port ? `:${port}` : ""}`;
  }
  if ("routeGroup" in backend)
    return `route group: ${String(backend.routeGroup)}`;
  if ("invalid" in backend) return "invalid";
  return backendKind(value);
}

function hostnamesSummary(hostnames: string[] | undefined) {
  if (!hostnames?.length) return "*";
  return (
    hostnames.slice(0, 2).join(", ") +
    (hostnames.length > 2 ? ` +${hostnames.length - 2}` : "")
  );
}

function backendKind(value: unknown) {
  if (typeof value === "string") return value;
  if (!value || typeof value !== "object") return "unknown";
  const key = Object.keys(value as Record<string, unknown>)[0];
  return key ?? "unknown";
}
