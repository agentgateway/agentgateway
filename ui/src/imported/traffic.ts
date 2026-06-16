import type {
  GatewayConfig,
  TrafficBind,
  TrafficListener,
  TrafficRoute,
  TrafficRouteBackend,
  TrafficTcpRoute,
  TrafficTcpRouteBackend,
} from "./types";

export type RouteKind = "http" | "tcp";

export interface ListenerContext {
  bind: TrafficBind;
  bindIndex: number;
  listener: TrafficListener;
  listenerIndex: number;
}

export interface RouteContext extends ListenerContext {
  kind: RouteKind;
  route: TrafficRoute | TrafficTcpRoute;
  routeIndex: number;
}

export interface BackendContext extends RouteContext {
  backend: TrafficRouteBackend | TrafficTcpRouteBackend;
  backendIndex: number;
}

export function ensureBinds(config: GatewayConfig) {
  if (!Array.isArray(config.binds)) config.binds = [];
  return config.binds;
}

export function trafficStats(config: GatewayConfig | undefined) {
  const binds = config?.binds ?? [];
  let listeners = 0;
  let httpRoutes = 0;
  let tcpRoutes = 0;
  let backends = 0;
  let invalidListeners = 0;
  for (const bind of binds) {
    for (const listener of bind.listeners ?? []) {
      listeners += 1;
      const routes = listener.routes ?? [];
      const tcp = listener.tcpRoutes ?? [];
      if (routes.length && tcp.length) invalidListeners += 1;
      httpRoutes += routes.length;
      tcpRoutes += tcp.length;
      for (const route of routes) backends += route.backends?.length ?? 0;
      for (const route of tcp) backends += route.backends?.length ?? 0;
    }
  }
  return { binds: binds.length, listeners, httpRoutes, tcpRoutes, backends, invalidListeners };
}

export function listenerContexts(config: GatewayConfig | undefined): ListenerContext[] {
  return (config?.binds ?? []).flatMap((bind, bindIndex) =>
    (bind.listeners ?? []).map((listener, listenerIndex) => ({
      bind,
      bindIndex,
      listener,
      listenerIndex,
    })),
  );
}

export function routeContexts(config: GatewayConfig | undefined): RouteContext[] {
  return listenerContexts(config).flatMap((context) => {
    const http = (context.listener.routes ?? []).map((route, routeIndex) => ({
      ...context,
      kind: "http" as const,
      route,
      routeIndex,
    }));
    const tcp = (context.listener.tcpRoutes ?? []).map((route, routeIndex) => ({
      ...context,
      kind: "tcp" as const,
      route,
      routeIndex,
    }));
    return [...http, ...tcp];
  });
}

export function backendContexts(config: GatewayConfig | undefined): BackendContext[] {
  return routeContexts(config).flatMap((context) =>
    ((context.route.backends ?? []) as Array<TrafficRouteBackend | TrafficTcpRouteBackend>).map((backend, backendIndex) => ({
      ...context,
      backend,
      backendIndex,
    })),
  );
}

export function getListener(config: GatewayConfig, bindIndex: number, listenerIndex: number) {
  return config.binds?.[bindIndex]?.listeners?.[listenerIndex];
}

export function getRoute(config: GatewayConfig, bindIndex: number, listenerIndex: number, kind: RouteKind, routeIndex: number) {
  const listener = getListener(config, bindIndex, listenerIndex);
  return kind === "http" ? listener?.routes?.[routeIndex] : listener?.tcpRoutes?.[routeIndex];
}

export function routeArray(listener: TrafficListener, kind: RouteKind) {
  if (kind === "http") {
    if (!Array.isArray(listener.routes)) listener.routes = [];
    return listener.routes;
  }
  if (!Array.isArray(listener.tcpRoutes)) listener.tcpRoutes = [];
  return listener.tcpRoutes;
}

export function bindKey(bindIndex: number) {
  return String(bindIndex);
}

export function listenerKey(bindIndex: number, listenerIndex: number) {
  return `${bindIndex}:${listenerIndex}`;
}

export function routeKey(bindIndex: number, listenerIndex: number, kind: RouteKind, routeIndex: number) {
  return `${bindIndex}:${listenerIndex}:${kind}:${routeIndex}`;
}

export function backendKey(bindIndex: number, listenerIndex: number, kind: RouteKind, routeIndex: number, backendIndex: number) {
  return `${routeKey(bindIndex, listenerIndex, kind, routeIndex)}:${backendIndex}`;
}

export function parseRouteKey(value: string | null) {
  if (!value) return null;
  const [bind, listener, kind, route] = value.split(":");
  if (kind !== "http" && kind !== "tcp") return null;
  return {
    bindIndex: Number(bind),
    listenerIndex: Number(listener),
    kind,
    routeIndex: Number(route),
  };
}

export function parseBackendKey(value: string | null) {
  if (!value) return null;
  const [bind, listener, kind, route, backend] = value.split(":");
  if (kind !== "http" && kind !== "tcp") return null;
  return {
    bindIndex: Number(bind),
    listenerIndex: Number(listener),
    kind,
    routeIndex: Number(route),
    backendIndex: Number(backend),
  };
}

export function backendType(backend: TrafficRouteBackend | TrafficTcpRouteBackend) {
  if (typeof backend === "string") return "unsupported";
  if ("host" in backend) return "host";
  if ("service" in backend) return "service";
  if ("dynamic" in backend) return "dynamic";
  return "unsupported";
}

export function backendSummary(backend: TrafficRouteBackend | TrafficTcpRouteBackend) {
  if (typeof backend === "string") return backend;
  if ("host" in backend) return String(backend.host);
  if ("service" in backend) {
    const service = backend.service as { name: { namespace: string; hostname: string }; port: number };
    return `${service.name.namespace}/${service.name.hostname}:${service.port}`;
  }
  if ("dynamic" in backend) return "dynamic";
  if ("backend" in backend) return `backend ref: ${backend.backend}`;
  if ("mcp" in backend) return "legacy MCP backend";
  if ("ai" in backend) return "legacy LLM backend";
  if ("aws" in backend) return "AWS backend";
  if ("routeGroup" in backend) return `route group: ${backend.routeGroup}`;
  return "unsupported backend";
}

export function routeDisplayName(route: TrafficRoute | TrafficTcpRoute, index: number) {
  return route.name?.trim() || route.ruleName?.trim() || `Route ${index + 1}`;
}

export function listenerDisplayName(listener: TrafficListener, index: number) {
  return listener.name?.trim() || `Listener ${index + 1}`;
}

export function pathSummary(route: TrafficRoute | TrafficTcpRoute) {
  if (!("matches" in route)) return "TCP";
  const first = route.matches?.[0]?.path;
  if (!first || first === "invalid") return "/";
  if ("pathPrefix" in first) return `${first.pathPrefix}*`;
  if ("exact" in first) return `= ${first.exact}`;
  if ("regex" in first) return `~ ${first.regex}`;
  return "/";
}
