import type { GatewayConfig } from "./types";

type PlaygroundEndpoint = {
  baseUrl: string;
  sameOrigin: boolean;
};

export function gatewayOrigin(port: number) {
  const protocol = window.location.protocol || "http:";
  const hostname = bracketIpv6(window.location.hostname || "localhost");
  return `${protocol}//${hostname}:${port}`;
}

export function gatewayEndpoint(port: number, path = "") {
  return `${gatewayOrigin(port)}${path}`;
}

export function llmPlaygroundEndpoint(
  config: GatewayConfig | null | undefined,
): PlaygroundEndpoint {
  return playgroundEndpoint(config, config?.llm?.gateways, 4000, "", "");
}

export function mcpPlaygroundEndpoint(
  config: GatewayConfig | null | undefined,
): PlaygroundEndpoint {
  return playgroundEndpoint(
    config,
    config?.mcp?.gateways,
    3000,
    "/mcp",
    "/mcp",
  );
}

function playgroundEndpoint(
  config: GatewayConfig | null | undefined,
  targetGateways: string | string[] | undefined,
  defaultPort: number,
  sameOriginBaseUrl: string,
  path: string,
): PlaygroundEndpoint {
  const uiGateways = gatewayRefs(config?.ui?.gateways);
  const targets = gatewayRefs(targetGateways);
  if (
    config &&
    uiGateways.length > 0 &&
    targets.length > 0 &&
    gatewayRefsOverlap(config, uiGateways, targets)
  ) {
    return { baseUrl: sameOriginBaseUrl, sameOrigin: true };
  }

  const gatewayPort = firstGatewayPort(config, targets);
  if (gatewayPort !== undefined) {
    return { baseUrl: gatewayEndpoint(gatewayPort, path), sameOrigin: false };
  }
  return { baseUrl: gatewayEndpoint(defaultPort, path), sameOrigin: false };
}

function gatewayRefs(refs: string | string[] | undefined) {
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

function firstGatewayPort(
  config: GatewayConfig | null | undefined,
  refs: string[] | undefined,
) {
  return refs
    ?.flatMap((ref) => expandGatewayRef(config, ref))
    .find((item) => item.port !== undefined)?.port;
}

function expandGatewayRefs(config: GatewayConfig, refs: string[]) {
  return refs.flatMap((ref) => expandGatewayRef(config, ref));
}

function expandGatewayRef(
  config: GatewayConfig | null | undefined,
  ref: string,
): Array<{ ref: string; port?: number }> {
  const [gatewayName, listenerName] = ref.split("/", 2);
  const gateway = config?.gateways?.[gatewayName];
  if (!gateway) return [{ ref }];

  if (!gateway.listeners?.length) {
    return [{ ref: gatewayName, port: gateway.port ?? undefined }];
  }

  if (listenerName) {
    return [
      {
        ref,
        port: gateway.port ?? undefined,
      },
    ];
  }

  return gateway.listeners.map((listener, index) => ({
    ref: `${gatewayName}/${listener.name ?? `listener${index}`}`,
    port: gateway.port ?? undefined,
  }));
}

function bracketIpv6(hostname: string) {
  return hostname.includes(":") && !hostname.startsWith("[")
    ? `[${hostname}]`
    : hostname;
}
