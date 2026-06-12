import type {
    FilterOrPolicy,
    FullLocalBackend,
    LocalAIBackend,
    LocalBackendPolicies,
    LocalBind,
    LocalConfig,
    LocalGatewayPolicy,
    LocalListener,
    LocalListenerProtocol,
    LocalMcpBackend,
    LocalMcpTarget,
    LocalNamedAIProvider,
    LocalPolicy,
    LocalRoute,
    LocalRouteBackend,
    LocalTLSServerConfig,
    RouteMatch,
  } from "../config";
  
  import {
    isKnownGatewayPolicyKey,
    isKnownPolicyKey,
  } from "../components/TrafficHierarchy/policyTypes";
  
  type Wire = Record<string, any>;

  type ResolvedBackend = 
    | { ai: LocalAIBackend }
    | { mcp: LocalMcpBackend }
    | { host: string };
  
  export function configDumpToLocalConfig(dump: Wire): LocalConfig {
    if (!dump || typeof dump !== "object") {
      return { binds: [], backends: [] };
    }
  
    const { listenerPolicies, routePolicies, orphans } = buildPolicyMaps(
      dump.policies ?? [],
    );

    const backendByRef = buildBackendRefMap(dump.backends ?? []);

    const binds: LocalBind[] = (dump.binds ?? [])
      .map((b: Wire) => mapBind(b, listenerPolicies, routePolicies, backendByRef))
      .filter(Boolean) as LocalBind[];
  
    const backends: FullLocalBackend[] = (dump.backends ?? [])
      .map(mapBackend)
      .filter(Boolean) as FullLocalBackend[];
  
    const result: LocalConfig = {
      binds,
      backends,
    };
  
    if (dump.workloads !== undefined) result.workloads = dump.workloads;
    if (dump.services !== undefined) result.services = dump.services;
    if (orphans.length > 0) result.policies = orphans;
  
    return result;
  }
  
  function parsePort(address: string | undefined): number {
    if (!address) return 0;
    const idx = address.lastIndexOf(":");
    if (idx === -1) return 0;
    const port = parseInt(address.slice(idx + 1), 10);
    return Number.isNaN(port) ? 0 : port;
  }
  
  function mapBind(
    wire: Wire,
    listenerPolicies: Map<string, LocalGatewayPolicy>,
    routePolicies: Map<string, FilterOrPolicy>,
    backendByRef: Map<string, ResolvedBackend>,
  ): LocalBind | undefined {
    if (!wire || typeof wire !== "object") return undefined;

    const listeners: LocalListener[] = Object.values(wire.listeners ?? {})
      .map((l: any) =>
        mapListener(l, listenerPolicies, routePolicies, backendByRef),
      )
      .filter(Boolean) as LocalListener[];
  
    const bind: LocalBind = {
      port: parsePort(wire.address),
      listeners,
    };
    if (wire.tunnelProtocol) bind.tunnelProtocol = wire.tunnelProtocol;
    return bind;
  }
  
  function mapListener(
    wire: Wire,
    listenerPolicies: Map<string, LocalGatewayPolicy>,
    routePolicies: Map<string, FilterOrPolicy>,
    backendByRef: Map<string, ResolvedBackend>,
  ): LocalListener | undefined {
    if (!wire || typeof wire !== "object") return undefined;
  
    const listener: LocalListener = {};
  
    if (wire.listenerName) listener.name = wire.listenerName;
    if (wire.gatewayNamespace) listener.namespace = wire.gatewayNamespace;
    if (wire.hostname !== undefined && wire.hostname !== "")
      listener.hostname = wire.hostname;
    if (wire.protocol) listener.protocol = wire.protocol as LocalListenerProtocol;
  
    if (wire.tls) {
      const tls = mapTls(wire.tls);
      if (tls) listener.tls = tls;
    }
  
    if (wire.routes) {
      listener.routes = Object.values(wire.routes)
        .map((r: any) => mapRoute(r, routePolicies, backendByRef))
        .filter(Boolean) as LocalRoute[];
    }
  
    if (wire.tcpRoutes) {
        // eslint-disable-next-line no-console
      console.warn(
        "configMapper: tcpRoutes present but mapping not yet implemented",
        wire.tcpRoutes,
      );
    }

    if (wire.listenerName) {
      const gatewayPolicy = listenerPolicies.get(wire.listenerName);
      if (gatewayPolicy) listener.policies = gatewayPolicy;
    }
  
    return listener;
  }
  
  function mapRoute(
    wire: Wire,
    routePolicies: Map<string, FilterOrPolicy>,
    backendByRef: Map<string, ResolvedBackend>,
  ): LocalRoute | undefined {
    if (!wire || typeof wire !== "object") return undefined;

    const route: LocalRoute = {};

    if (wire.name) route.name = wire.name;
    if (wire.namespace) route.namespace = wire.namespace;
    if (Array.isArray(wire.matches)) {
      route.matches = wire.matches.map(mapMatch).filter(Boolean) as RouteMatch[];
    }

    if (Array.isArray(wire.backends)) {
      route.backends = wire.backends
        .map((b: Wire) => mapRouteBackend(b, backendByRef))
        .filter(Boolean) as LocalRouteBackend[];
    }

    const inline = mergeInlinePolicies<FilterOrPolicy>(wire.inlinePolicies);
    const attached =
      wire.namespace && wire.name
        ? routePolicies.get(`${wire.namespace}/${wire.name}`)
        : undefined;
    if (inline || attached) {
      route.policies = { ...(attached ?? {}), ...(inline ?? {}) };
    }

    return route;
  }
  
  function mapMatch(wire: Wire): RouteMatch | undefined {
    if (!wire || typeof wire !== "object") return undefined;
    const match: RouteMatch = {};
  
    if (wire.path) match.path = wire.path; // already in dest shape
    if (Array.isArray(wire.headers)) match.headers = wire.headers;
    if (Array.isArray(wire.query)) match.query = wire.query;
    if (typeof wire.method === "string") match.method = wire.method;
  
    return match;
  }
  
  function mapRouteBackend(
    wire: Wire,
    backendByRef: Map<string, ResolvedBackend>,
  ): LocalRouteBackend | undefined {
    if (!wire || typeof wire !== "object") return undefined;

    const result: any = {};
    if (typeof wire.weight === "number") result.weight = wire.weight;

    if (typeof wire.backend === "string") {
      const resolved = backendByRef.get(wire.backend);
      if (resolved) {
        Object.assign(result, resolved);
      } else {
        result.backend = wire.backend;
      }
    }

    return result as LocalRouteBackend;
  }

  function mapBackend(wire: Wire): FullLocalBackend | undefined {
    if (!wire?.backend || typeof wire.backend !== "object") return undefined;
    const inner = wire.backend;
  
    let kindObj: Record<string, unknown> | undefined;
    let name = "";
  
    if (inner.host) {
      name = inner.host.name ?? "";
      kindObj = { host: inner.host.target ?? "" };
    } else if (inner.mcp) {
      name = inner.mcp.name ?? "";
      const mcp = mapMcpBackend(inner.mcp);
      if (mcp) kindObj = { mcp };
    } else if (inner.ai) {
      name = inner.ai.name ?? "";
      const ai = mapAiBackend(inner.ai);
      if (ai) kindObj = { ai };
    } else {
      // eslint-disable-next-line no-console
      console.warn("configMapper: unknown backend kind", Object.keys(inner));
      return undefined;
    }
  
    if (!kindObj) return undefined;
  
    const policies = mergeInlinePolicies<LocalBackendPolicies>(
      wire.inlinePolicies,
    );
    const result: any = { name, ...kindObj };
    if (policies) result.policies = policies;
    return result as FullLocalBackend;
  }

  function buildBackendRefMap(
    wireBackends: Wire[],
  ): Map<string, ResolvedBackend> {
    const map = new Map<string, ResolvedBackend>();
    for (const wb of wireBackends) {
      const inner = wb?.backend;
      if (!inner) continue;

      const meta = inner.host ?? inner.mcp ?? inner.ai;
      if (!meta?.name) continue;

      const refKey = `${meta.namespace ?? ""}/${meta.name}`;

      if (map.has(refKey) && inner.host) continue;

      if (inner.mcp) {
        const mcp = mapMcpBackend(inner.mcp);
        if (mcp) map.set(refKey, { mcp });
      } else if (inner.ai) {
        const ai = mapAiBackend(inner.ai);
        if (ai) map.set(refKey, { ai });
      } else if (inner.host) {
        map.set(refKey, { host: inner.host.target ?? "" });
      }
    }
    return map;
  }

  function mapMcpBackend(wire: Wire): LocalMcpBackend | undefined {
    const target = wire?.target;
    if (!target) return undefined;
  
    const result: LocalMcpBackend = {
      targets: Array.isArray(target.targets)
        ? (target.targets.map(mapMcpTarget).filter(Boolean) as LocalMcpTarget[])
        : [],
    };
  
    if (typeof target.stateful === "boolean") {
      result.statefulMode = target.stateful ? "stateful" : "stateless";
    }
    if (typeof target.alwaysUsePrefix === "boolean") {
      result.prefixMode = target.alwaysUsePrefix ? "always" : "conditional";
    }
    if (
      target.failureMode === "failClosed" ||
      target.failureMode === "failOpen"
    ) {
      result.failureMode = target.failureMode;
    }
  
    return result;
  }
  
  function mapMcpTarget(wire: Wire): LocalMcpTarget | undefined {
    if (!wire?.name) return undefined;
    const base: any = { name: wire.name };
  
    if (wire.stdio) {
      base.stdio = {
        cmd: wire.stdio.cmd,
        ...(wire.stdio.args !== undefined && { args: wire.stdio.args }),
        ...(wire.stdio.env !== undefined && { env: wire.stdio.env }),
        ...(wire.stdio.clear_env !== undefined && {
          clear_env: wire.stdio.clear_env,
        }),
      };
    } else if (wire.sse) {
      base.sse = unwrapTransportTarget(wire.sse);
    } else if (wire.mcp) {
      base.mcp = unwrapTransportTarget(wire.mcp);
    } else if (wire.openapi) {
      base.openapi = {
        ...unwrapTransportTarget(wire.openapi),
        schema: wire.openapi.schema,
      };
    } else {
      // eslint-disable-next-line no-console
      console.warn("configMapper: unknown MCP target kind", wire);
      return undefined;
    }
  
    return base as LocalMcpTarget;
  }
  
  function unwrapTransportTarget(wire: Wire) {
    const out: Record<string, unknown> = {};
    if (wire.host !== undefined) out.host = wire.host;
    if (wire.port !== undefined) out.port = wire.port;
    if (wire.path !== undefined) out.path = wire.path;
    if (wire.backend !== undefined) {
      out.backend =
        typeof wire.backend === "object" && wire.backend !== null
          ? wire.backend.backend
          : wire.backend;
    }
    return out;
  }
  
  function mapAiBackend(wire: Wire): LocalAIBackend | undefined {
    const wireProviders = wire?.target?.providers;
    if (!Array.isArray(wireProviders)) return undefined;

    const fallbackName = typeof wire?.name === "string" ? wire.name : "";
    const providers: LocalNamedAIProvider[] = wireProviders
      .map((p: Wire) => mapAiProvider(p, fallbackName))
      .filter(Boolean) as LocalNamedAIProvider[];

    if (providers.length === 0) return undefined;

    if (providers.length === 1) {
      return providers[0];
    }
    return {
      groups: [{ providers }],
    };
  }

  function mapAiProvider(
    wire: Wire,
    fallbackName: string,
  ): LocalNamedAIProvider | undefined {
    const active = wire?.active;
    if (!active || typeof active !== "object") return undefined;

    const providerKeys = Object.keys(active);
    if (providerKeys.length === 0) return undefined;
    const inner = active[providerKeys[0]];
    const endpoint = inner?.endpoint;
    if (!endpoint) return undefined;

    const rawName = endpoint.name ?? "";
    const name = rawName === "backend" && fallbackName ? fallbackName : rawName;

    const result: LocalNamedAIProvider = {
      name,
      provider: endpoint.provider,
    };
  
    if (endpoint.hostOverride != null)
      result.hostOverride = endpoint.hostOverride;
    if (endpoint.pathOverride != null)
      result.pathOverride = endpoint.pathOverride;
    if (endpoint.pathPrefix != null) result.pathPrefix = endpoint.pathPrefix;
    if (typeof endpoint.tokenize === "boolean")
      result.tokenize = endpoint.tokenize;
  
    return result;
  }
  
  function mergeInlinePolicies<T>(wire: unknown): T | undefined {
    if (!Array.isArray(wire) || wire.length === 0) return undefined;
  
    const merged: Record<string, unknown> = {};
    for (const entry of wire) {
      if (!entry || typeof entry !== "object") continue;
      const keys = Object.keys(entry);
      if (keys.length === 0) continue;
      for (const key of keys) {
        const value = (entry as Record<string, unknown>)[key];
        if (value === null || value === undefined) continue;
  
        const destKey = key === "transformation" ? "transformations" : key;
  
        if (!isKnownPolicyKey(destKey)) {
          // eslint-disable-next-line no-console
          console.warn(`configMapper: unknown inline policy key '${key}'`, value);
        }
        merged[destKey] = value;
      }
    }
  
    return Object.keys(merged).length > 0 ? (merged as T) : undefined;
  }
  
  interface PolicyMapsResult {
    listenerPolicies: Map<string, LocalGatewayPolicy>;
    routePolicies: Map<string, FilterOrPolicy>;
    orphans: LocalPolicy[];
  }

  function buildPolicyMaps(wirePolicies: unknown): PolicyMapsResult {
    const listenerPolicies = new Map<string, LocalGatewayPolicy>();
    const routePolicies = new Map<string, FilterOrPolicy>();
    const orphans: LocalPolicy[] = [];

    if (!Array.isArray(wirePolicies)) {
      return { listenerPolicies, routePolicies, orphans };
    }

    for (const wp of wirePolicies) {
      if (!wp || typeof wp !== "object") continue;

      const listenerName: string | undefined = wp.target?.gateway?.listenerName;
      const routeTarget = wp.target?.route;
      const traffic = wp.policy?.traffic;

      if (!traffic || typeof traffic !== "object") {
        orphans.push(wp as LocalPolicy);
        continue;
      }

      let policySlot: Record<string, unknown> | undefined;
      let storeBack: (() => void) | undefined;

      if (listenerName) {
        const slot = listenerPolicies.get(listenerName) ?? {};
        policySlot = slot as Record<string, unknown>;
        storeBack = () => listenerPolicies.set(listenerName, slot);
      } else if (
        routeTarget &&
        typeof routeTarget.name === "string" &&
        typeof routeTarget.namespace === "string"
      ) {
        const routeKey = `${routeTarget.namespace}/${routeTarget.name}`;
        const slot = routePolicies.get(routeKey) ?? {};
        policySlot = slot as Record<string, unknown>;
        storeBack = () => routePolicies.set(routeKey, slot);
      } else {
        orphans.push(wp as LocalPolicy);
        continue;
      }

      for (const key of Object.keys(traffic)) {
        if (key === "phase") continue;
        const value = (traffic as Record<string, unknown>)[key];
        if (value === null || value === undefined) continue;

        const destKey = key === "transformation" ? "transformations" : key;

        const known = listenerName
          ? isKnownGatewayPolicyKey(destKey)
          : isKnownPolicyKey(destKey);
        if (!known) {
          // eslint-disable-next-line no-console
          console.warn(
            `configMapper: unknown ${listenerName ? "gateway" : "route"} policy key '${key}'`,
            value,
          );
        }
        policySlot[destKey] = value;
      }
      storeBack!();
    }

    return { listenerPolicies, routePolicies, orphans };
  }
  
  function mapTls(wire: Wire): LocalTLSServerConfig | undefined {
    if (!wire || typeof wire !== "object") return undefined;
    if (typeof wire.cert !== "string" || typeof wire.key !== "string")
      return undefined;
  
    const result: LocalTLSServerConfig = { cert: wire.cert, key: wire.key };
    if (wire.root != null) result.root = wire.root;
    if (Array.isArray(wire.cipherSuites)) result.cipherSuites = wire.cipherSuites;
    if (wire.minTLSVersion) result.minTLSVersion = wire.minTLSVersion;
    if (wire.maxTLSVersion) result.maxTLSVersion = wire.maxTLSVersion;
    return result;
  }
  