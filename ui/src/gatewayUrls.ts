import type { GatewayConfig } from "./types";

type PlaygroundEndpoint = {
  baseUrl: string;
  sameOrigin: boolean;
};

type GatewayScheme = "http" | "https";

type ResolvedGatewayEndpoint = {
  ref: string;
  port?: number;
  scheme?: GatewayScheme;
};

export function gatewayOrigin(port: number, scheme = currentWindowScheme()) {
  const hostname = bracketIpv6(window.location.hostname || "localhost");
  const portSuffix = defaultPort(scheme) === port ? "" : `:${port}`;
  return `${scheme}://${hostname}${portSuffix}`;
}

export function gatewayEndpoint(
  port: number,
  path = "",
  scheme = currentWindowScheme(),
) {
  return `${gatewayOrigin(port, scheme)}${path}`;
}

export function llmPlaygroundEndpoint(
  config: GatewayConfig | null | undefined,
): PlaygroundEndpoint {
  return playgroundEndpoint(
    config,
    llmGatewayRefs(config),
    config?.llm?.port ?? 4000,
    config?.llm ? (config.llm.tls ? "https" : "http") : currentWindowScheme(),
    "",
    "",
  );
}

export function mcpPlaygroundEndpoint(
  config: GatewayConfig | null | undefined,
): PlaygroundEndpoint {
  return playgroundEndpoint(
    config,
    mcpGatewayRefs(config),
    config?.mcp?.port ?? 3000,
    "http",
    "/mcp",
    "/mcp",
  );
}

export function llmGatewayOrigin(config: GatewayConfig | null | undefined) {
  const gateway = firstGatewayEndpoint(config, llmGatewayRefs(config));
  if (gateway?.port !== undefined) {
    return gatewayOrigin(gateway.port, gateway.scheme ?? currentWindowScheme());
  }
  return gatewayOrigin(
    config?.llm?.port ?? 4000,
    config?.llm ? (config.llm.tls ? "https" : "http") : currentWindowScheme(),
  );
}

function playgroundEndpoint(
  config: GatewayConfig | null | undefined,
  targets: string[],
  defaultPort: number,
  defaultScheme: GatewayScheme,
  sameOriginBaseUrl: string,
  path: string,
): PlaygroundEndpoint {
  const uiGateways = uiGatewayRefs(config);
  if (
    config &&
    uiGateways.length > 0 &&
    targets.length > 0 &&
    gatewayRefsOverlap(config, uiGateways, targets)
  ) {
    return { baseUrl: sameOriginBaseUrl, sameOrigin: true };
  }

  const gateway = firstGatewayEndpoint(config, targets);
  if (gateway?.port !== undefined) {
    return {
      baseUrl: gatewayEndpoint(
        gateway.port,
        path,
        gateway.scheme ?? currentWindowScheme(),
      ),
      sameOrigin: false,
    };
  }
  return {
    baseUrl: gatewayEndpoint(defaultPort, path, defaultScheme),
    sameOrigin: false,
  };
}

function llmGatewayRefs(config: GatewayConfig | null | undefined) {
  const refs = gatewayRefs(config?.llm?.gateways);
  if (refs.length) return refs;
  if (
    config?.llm &&
    config.gateways?.default &&
    config.llm.port == null &&
    config.llm.tls == null
  ) {
    return ["default"];
  }
  return [];
}

function mcpGatewayRefs(config: GatewayConfig | null | undefined) {
  const refs = gatewayRefs(config?.mcp?.gateways);
  if (refs.length) return refs;
  if (config?.mcp && config.gateways?.default && config.mcp.port == null) {
    return ["default"];
  }
  return [];
}

function uiGatewayRefs(config: GatewayConfig | null | undefined) {
  const refs = gatewayRefs(config?.ui?.gateways);
  if (refs.length) return refs;
  if (config?.ui && config.gateways?.default) return ["default"];
  return [];
}

function gatewayRefs(refs: string | string[] | null | undefined) {
  if (!refs) return [];
  return Array.isArray(refs) ? refs : [refs];
}

function gatewayRefsOverlap(
  config: GatewayConfig,
  leftRefs: string[],
  rightRefs: string[],
) {
  const left = new Set(
    expandGatewayRefs(config, leftRefs).map((item) => item.ref),
  );
  return expandGatewayRefs(config, rightRefs).some((item) =>
    left.has(item.ref),
  );
}

function firstGatewayEndpoint(
  config: GatewayConfig | null | undefined,
  refs: string[] | undefined,
) {
  return refs
    ?.flatMap((ref) => expandGatewayRef(config, ref))
    .find((item) => item.port !== undefined);
}

function expandGatewayRefs(config: GatewayConfig, refs: string[]) {
  return refs.flatMap((ref) => expandGatewayRef(config, ref));
}

function expandGatewayRef(
  config: GatewayConfig | null | undefined,
  ref: string,
): ResolvedGatewayEndpoint[] {
  const [gatewayName, listenerName] = ref.split("/", 2);
  const gateway = config?.gateways?.[gatewayName];
  if (!gateway) return [{ ref }];

  if (!gateway.listeners?.length) {
    return [
      {
        ref: gatewayName,
        port: gateway.port ?? undefined,
        scheme: gatewayScheme(gateway),
      },
    ];
  }

  if (listenerName) {
    const listener = gateway.listeners.find((item, index) => {
      const name = item.name ?? `listener${index}`;
      return name === listenerName;
    });
    return [
      {
        ref,
        port: gateway.port ?? undefined,
        scheme: listener ? gatewayScheme(listener) : undefined,
      },
    ];
  }

  return gateway.listeners.map((listener, index) => ({
    ref: `${gatewayName}/${listener.name ?? `listener${index}`}`,
    port: gateway.port ?? undefined,
    scheme: gatewayScheme(listener),
  }));
}

function gatewayScheme(
  gateway:
    | NonNullable<GatewayConfig["gateways"]>[string]
    | {
        protocol?: "HTTP" | "HTTPS" | "TCP" | "TLS" | null;
        tls?: unknown;
      },
): GatewayScheme {
  const protocol = gateway.protocol ?? (gateway.tls ? "HTTPS" : "HTTP");
  return protocol === "HTTPS" || protocol === "TLS" ? "https" : "http";
}

function bracketIpv6(hostname: string) {
  return hostname.includes(":") && !hostname.startsWith("[")
    ? `[${hostname}]`
    : hostname;
}

function currentWindowScheme(): GatewayScheme {
  return window.location.protocol === "https:" ? "https" : "http";
}

function defaultPort(scheme: GatewayScheme) {
  return scheme === "https" ? 443 : 80;
}
