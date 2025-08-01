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
  ServiceBackend,
  TargetFilter,
  StdioTarget,
  SseTarget,
  OpenApiTarget,
  McpConnectionTarget,
  AiBackend,
} from "./types";

export function configDumpToLocalConfig(configDump: any): LocalConfig {
  const localConfig: LocalConfig = {
    binds: [],
    workloads: configDump.workloads || [],
    services: configDump.services || [],
  };

  const backends = (configDump.backends || []).map((b: any) => mapToBackend(b)).filter(Boolean);

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
    gatewayName: listenerData.gatewayName,
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
    name: routeData.routeName,
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
  if (rb.backend) {
    const found = backends.find((b) => getBackendName(b) === rb.backend);
    if (found) return found;
  }

  // Fallback: instantiate a backend in-place based on the route backend data
  // This covers cases where service/host backends are defined directly inside the route
  return mapToBackend(rb);
}

function getBackendName(backend: Backend): string {
  if (backend.service) return `${backend.service.name.namespace}/${backend.service.name.hostname}:${backend.service.port}`;
  if (backend.host) return backend.host.name ?? "";
  if (backend.mcp) return backend.mcp.name;
  if (backend.ai) return backend.ai.name;
  return "";
}

function mapToServiceBackend(data: any): ServiceBackend | undefined {
  if (!data || typeof data.port !== "number") return undefined;

  let namespace = "";
  let hostname = "";

  if (typeof data.name === "string") {
    // Handle formats like "default/httpbin" or fully qualified hostnames
    if (data.name.includes("/")) {
      const parts = data.name.split("/");
      namespace = parts[0];
      hostname = parts.slice(1).join("/");
    } else if (data.name.includes(".")) {
      // Possibly a FQDN like "httpbin.default.svc.cluster.local" – treat first segment as hostname
      hostname = data.name.split(".")[0];
    } else {
      hostname = data.name;
    }
  } else if (typeof data.name === "object" && data.name !== null) {
    namespace = data.name.namespace ?? "";
    hostname = data.name.hostname ?? "";
  }

  return {
    name: { namespace, hostname },
    port: data.port,
  } as ServiceBackend;
}

function mapToHostBackend(data: any): HostBackend | undefined {
  if (!data) return undefined;
  if (typeof data.target === "string") {
    const [host, portStr] = data.target.split(":");
    const port = Number(portStr);
    if (!isNaN(port)) {
      return {
        Hostname: [host, port],
        name: data.name,
      } as HostBackend;
    }
  }

  return undefined;
}

function mapToMcpBackend(data: any): McpBackend | undefined {
  if (typeof data?.name !== "string" || !Array.isArray(data?.target?.targets)) return undefined;
  const targets = data.target.targets.map(mapToMcpTarget).filter(Boolean) as McpTarget[];
  return { name: data.name, targets } as McpBackend;
}

function mapToMcpTarget(data: any): McpTarget | undefined {
  if (!data || typeof data.name !== "string") return undefined;
  const target: McpTarget = { name: data.name } as McpTarget;
  if (Array.isArray(data.filters))
    target.filters = data.filters.map(mapToTargetFilter).filter(Boolean);
  if (data.stdio) target.stdio = mapToStdioTarget(data.stdio);
  else if (data.sse) target.sse = mapToSseTarget(data.sse);
  else if (data.openapi) target.openapi = mapToOpenApiTarget(data.openapi);
  else if (data.mcp) target.mcp = mapToMcpConnectionTarget(data.mcp);
  return target;
}

function mapToTargetFilter(data: any): TargetFilter | undefined {
  if (!data || typeof data.matcher !== "string") return undefined;
  return { matcher: data.matcher, resource_type: data.resource_type };
}

function mapToStdioTarget(data: any): StdioTarget | undefined {
  if (!data || typeof data.cmd !== "string") return undefined;
  return { cmd: data.cmd, args: data.args, env: data.env } as StdioTarget;
}

function mapToSseTarget(data: any): SseTarget | undefined {
  if (!data || typeof data.host !== "string" || typeof data.port !== "number") return undefined;
  return { host: data.host, port: data.port, path: data.path } as SseTarget;
}

function mapToOpenApiTarget(data: any): OpenApiTarget | undefined {
  if (!data || typeof data.host !== "string" || typeof data.port !== "number") return undefined;
  return { host: data.host, port: data.port, schema: data.schema } as OpenApiTarget;
}

function mapToMcpConnectionTarget(data: any): McpConnectionTarget | undefined {
  if (!data || typeof data.host !== "string" || typeof data.port !== "number") return undefined;
  return { host: data.host, port: data.port, path: data.path } as McpConnectionTarget;
}

function mapToAiBackend(data: any): AiBackend | undefined {
  if (!data?.name) return undefined;
  const providerData = data.target?.provider;
  const hostOverrideRaw = data.target?.hostOverride;
  if (!providerData) return undefined;
  return {
    name: data.name,
    provider: providerData,
    hostOverride: hostOverrideRaw ? mapToHostBackend(hostOverrideRaw) : undefined,
  } as AiBackend;
}

function mapToTlsConfig(data: any): TlsConfig | undefined {
  if (!data) return undefined;
  return { cert: data.cert, key: data.key } as TlsConfig;
}
