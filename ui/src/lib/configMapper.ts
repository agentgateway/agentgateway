// Mapping utilities to attempt to convert the /config_dump response into the LocalConfig TypeScript object
// Support for full conversion is not yet implemented.
import {
  Bind,
  Backend,
  Listener,
  LocalConfig,
  Route,
  ListenerProtocol,
  Match,
  TlsConfig,
  HostBackend,
  McpBackend,
  McpTarget,
  McpStatefulMode,
  ServiceBackend,
  AiBackend,
  TargetFilter,
  StdioTarget,
  StreamHttpTarget,
  OpenApiTarget,
} from "./types";

export function configDumpToLocalConfig(configDump: any): LocalConfig {
  const localConfig: LocalConfig = {
    binds: [],
    workloads: configDump.workloads || [],
    services: configDump.services || [],
    appliedPolicies: configDump.policies || [],
  };

  const backends = (configDump.backends || [])
    .map((b: any) => (b.backend ? mapToBackend(b.backend) : null))
    .filter(Boolean);

  localConfig.binds = (configDump.binds || []).map((bind: any) =>
    mapToBind(bind, backends as Backend[])
  );

  return localConfig;
}

function mapToBind(bindData: any, backends: Backend[]): Bind {
  return {
    port: parseInt(bindData.address.split(":")[1]),
    listeners: Object.values(bindData.listeners || {}).map((listenerData: any) =>
      mapToListener(listenerData, backends)
    ),
  };
}

function mapToListener(listenerData: any, backends: Backend[]): Listener {
  return {
    name: listenerData.name,
    hostname: listenerData.hostname,
    protocol: listenerData.protocol as ListenerProtocol,
    tls: mapToTlsConfig(listenerData.tls),
    routes: Object.values(listenerData.routes || {}).map((routeData: any) =>
      mapToRoute(routeData, backends)
    ),
  };
}

function mapToRoute(routeData: any, backends: Backend[]): Route {
  return {
    name: routeData.name,
    ruleName: routeData.ruleName || "",
    hostnames: routeData.hostnames || [],
    matches: mapToMatches(routeData.matches),
    backends: (routeData.backends || []).map((rb: any) => mapToRouteBackend(rb, backends)),
  };
}

function mapToMatches(matchesData: any): Match[] {
  if (!matchesData) return [];
  return Object.values(matchesData).map((matchData: any) => {
    const match: Match = { path: {} } as Match;

    if (matchData.headers) {
      match.headers = Object.entries(matchData.headers).map(([name, value]) => ({
        name,
        value: { exact: value as string },
      }));
    }

    if (matchData.path) {
      if (matchData.path.exact) {
        match.path.exact = matchData.path.exact;
      } else if (matchData.path.prefix) {
        match.path.pathPrefix = matchData.path.prefix;
      } else if (matchData.path.regex) {
        match.path.regex = [matchData.path.regex, 0];
      }
    }

    if (matchData.method) match.method = { method: matchData.method };

    if (matchData.query) {
      match.query = Object.entries(matchData.query).map(([name, value]) => ({
        name,
        value: { exact: value as string },
      }));
    }

    return match;
  });
}

function mapToBackend(backendData: any): Backend | undefined {
  if (!backendData || typeof backendData !== "object") return undefined;
  const backend: Backend = {} as Backend;
  if (typeof backendData.weight === "number") backend.weight = backendData.weight;
  if (backendData.service) backend.service = mapToServiceBackend(backendData.service);
  else if (backendData.host) backend.host = mapToHostBackend(backendData.host);
  else if (backendData.mcp) backend.mcp = mapToMcpBackend(backendData.mcp);
  else if (backendData.ai) backend.ai = mapToAiBackend(backendData.ai);
  return backend;
}

function mapToRouteBackend(rb: any, backends: Backend[]): Backend | undefined {
  // Route backend reference is a string in "namespace/name" format
  if (typeof rb.backend === "string") {
    const found = backends.find((b) => getBackendName(b) === rb.backend);
    if (found) return found;
  }

  // Fallback: instantiate a backend in-place based on the route backend data
  return mapToBackend(rb);
}

function getBackendName(backend: Backend): string {
  if (backend.service)
    return `${backend.service.name.namespace}/${backend.service.name.hostname}:${backend.service.port}`;
  if (backend.host) return backend.host.name ?? "";
  // name already includes namespace prefix (e.g., "namespace/name")
  if (backend.mcp) return backend.mcp.name || "";
  if (backend.ai) return backend.ai.name;
  return "";
}

function mapToServiceBackend(data: any): ServiceBackend | undefined {
  if (!data || typeof data.port !== "number") return undefined;

  let namespace = "";
  let hostname = "";

  if (typeof data.name === "string") {
    // Handle formats like "namespace/hostname" or "namespace/fqdn"
    if (data.name.includes("/")) {
      const parts = data.name.split("/");
      namespace = parts[0];
      hostname = parts.slice(1).join("/"); // Keep original (might be FQDN)
    } else {
      hostname = data.name;
      // Extract namespace from FQDN if it looks like K8s format: hostname.namespace.svc.cluster.local
      if (hostname.includes(".")) {
        const parts = hostname.split(".");
        if (parts.length >= 2) {
          namespace = parts[1]; // Second segment is namespace
        }
      }
    }
  } else if (typeof data.name === "object" && data.name !== null) {
    hostname = data.name.hostname ?? "";
    namespace = data.name.namespace ?? "";
    // If namespace is empty but hostname is FQDN, extract namespace
    if (!namespace && hostname.includes(".")) {
      const parts = hostname.split(".");
      if (parts.length >= 2) {
        namespace = parts[1];
      }
    }
  }

  return {
    name: { namespace, hostname },
    port: data.port,
  } as ServiceBackend;
}

function mapToHostBackend(data: any): HostBackend | undefined {
  if (!data) return undefined;
  // Include namespace in name to match route backend reference format "namespace/name"
  const fullName = data.namespace && data.name ? `${data.namespace}/${data.name}` : data.name;
  if (typeof data.target === "string") {
    const [host, portStr] = data.target.split(":");
    const port = Number(portStr);
    if (!isNaN(port)) {
      return {
        Hostname: [host, port],
        name: fullName,
      } as HostBackend;
    }
  }

  return undefined;
}

// Transform /config_dump nested format to UI-friendly flat format
// Input:  { name, namespace, target: { targets: [...], stateful, alwaysUsePrefix } }
// Output: { name, targets: [...], statefulMode }
function mapToMcpBackend(data: any): McpBackend | undefined {
  if (typeof data?.name !== "string" || !Array.isArray(data?.target?.targets)) return undefined;
  const targets = data.target.targets.map(mapToMcpTarget).filter(Boolean) as McpTarget[];
  // Include namespace in name to match route backend reference format "namespace/name"
  const fullName = data.namespace ? `${data.namespace}/${data.name}` : data.name;
  return {
    name: fullName,
    targets, // Flat structure for UI and write path
    statefulMode: data.target?.stateful ? McpStatefulMode.STATEFUL : McpStatefulMode.STATELESS,
  };
}

function mapToMcpTarget(data: any): McpTarget | undefined {
  if (!data || typeof data.name !== "string") return undefined;
  const target: McpTarget = { name: data.name } as McpTarget;
  if (Array.isArray(data.filters))
    target.filters = data.filters.map(mapToTargetFilter).filter(Boolean);
  if (data.stdio) target.stdio = mapToStdioTarget(data.stdio);
  else if (data.sse) target.sse = mapToStreamHttpTarget(data.sse);
  else if (data.openapi) target.openapi = mapToOpenApiTarget(data.openapi);
  else if (data.mcp) target.mcp = mapToStreamHttpTarget(data.mcp);
  return target;
}

function mapToTargetFilter(data: any): TargetFilter | undefined {
  if (!data || !data.matcher) return undefined;
  return { matcher: data.matcher, resource_type: data.resource_type };
}

function mapToStdioTarget(data: any): StdioTarget | undefined {
  if (!data || typeof data.cmd !== "string") return undefined;
  return { cmd: data.cmd, args: data.args, env: data.env } as StdioTarget;
}

// Transform /config_dump nested format { backend: {...}, path: "..." } to flat { host, port, path }
function mapToStreamHttpTarget(data: any): StreamHttpTarget | undefined {
  if (!data) return undefined;

  // Handle new nested format from /config_dump: { backend: {...}, path: "..." }
  if (data.backend) {
    const { host, port } = parseBackendReference(data.backend);
    if (host) {
      return { host, port, path: data.path };
    }
  }

  // Handle legacy/config file format: { host, port, path } or { host: "url" }
  if (typeof data.host === "string") {
    return { host: data.host, port: data.port, path: data.path };
  }

  return undefined;
}

// Transform /config_dump nested format { backend: {...}, schema: {...} } to flat { host, port, schema }
function mapToOpenApiTarget(data: any): OpenApiTarget | undefined {
  if (!data) return undefined;

  // Handle new nested format from /config_dump: { backend: {...}, schema: {...} }
  if (data.backend) {
    const { host, port } = parseBackendReference(data.backend);
    if (host) {
      return { host, port, schema: data.schema };
    }
  }

  // Handle legacy/config file format: { host, port, schema }
  if (typeof data.host === "string") {
    return { host: data.host, port: data.port, schema: data.schema };
  }

  return undefined;
}

// Parse SimpleBackendReference from /config_dump into host:port
function parseBackendReference(backend: any): { host: string; port?: number } {
  if (!backend) return { host: "" };

  // Handle inlineBackend: "hostname:port" string
  if (typeof backend.inlineBackend === "string") {
    const parts = backend.inlineBackend.split(":");
    if (parts.length >= 2) {
      const port = parseInt(parts[parts.length - 1], 10);
      const host = parts.slice(0, -1).join(":");
      return { host, port: isNaN(port) ? undefined : port };
    }
    return { host: backend.inlineBackend };
  }

  // Handle host: "hostname:port" string (alternate serialization)
  if (typeof backend.host === "string") {
    const parts = backend.host.split(":");
    if (parts.length >= 2) {
      const port = parseInt(parts[parts.length - 1], 10);
      const host = parts.slice(0, -1).join(":");
      return { host, port: isNaN(port) ? undefined : port };
    }
    return { host: backend.host };
  }

  // Handle service reference: { service: { name: {...}, port: number } }
  if (backend.service) {
    const name = backend.service.name;
    const port = backend.service.port;
    const hostname = typeof name === "string" ? name : name?.hostname || name?.name || "unknown";
    return { host: hostname, port };
  }

  // Handle backend reference string: { backend: "namespace/name" }
  if (typeof backend.backend === "string") {
    return { host: backend.backend };
  }

  return { host: "" };
}

// Transform /config_dump nested AI backend format to UI-friendly flat format
// Input:  { name, namespace, target: { providers: [{ active: { key: { endpoint: {...} } } }] } }
// Output: { name, provider, hostOverride?, pathOverride? }
function mapToAiBackend(data: any): AiBackend | undefined {
  if (!data?.name) return undefined;

  // Include namespace in name to match route backend reference format
  const fullName = data.namespace ? `${data.namespace}/${data.name}` : data.name;

  // Extract the first provider from nested EndpointSet structure
  const providers = data.target?.providers;
  if (!Array.isArray(providers) || !providers[0]?.active) {
    return { name: fullName, provider: {} };
  }

  const firstKey = Object.keys(providers[0].active)[0];
  const endpointWithInfo = providers[0].active[firstKey];
  const namedProvider = endpointWithInfo?.endpoint;

  if (!namedProvider) {
    return { name: fullName, provider: {} };
  }

  return {
    name: fullName,
    provider: namedProvider.provider || {},
    hostOverride: namedProvider.hostOverride,
    pathOverride: namedProvider.pathOverride,
  };
}

function mapToTlsConfig(data: any): TlsConfig | undefined {
  if (!data) return undefined;
  return {
    cert: data.cert,
    key: data.key,
    root: data.root,
    cipherSuites: data.cipherSuites,
    minTLSVersion: data.minTLSVersion,
    maxTLSVersion: data.maxTLSVersion,
  };
}
