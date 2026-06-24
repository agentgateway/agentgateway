import type { CorsPolicy, GatewayConfig } from "./types";

export type CorsTarget = "llm" | "mcp";

export function currentOrigin() {
  return window.location.origin;
}

export function corsNeedsUpdate(
  cors: CorsPolicy | null | undefined,
  target: CorsTarget,
  origin = currentOrigin(),
) {
  if (!cors) return true;
  return (
    !hasValue(cors.allowOrigins, origin) ||
    !hasValue(cors.allowMethods, "GET") ||
    !hasValue(cors.allowMethods, "POST") ||
    !hasValue(cors.allowHeaders, "*") ||
    (target === "mcp" && !hasValue(cors.exposeHeaders, "Mcp-Session-Id"))
  );
}

export function applyPlaygroundCors(
  config: GatewayConfig,
  target: CorsTarget,
  origin = currentOrigin(),
) {
  if (target === "llm") {
    config.llm ??= { models: [] };
    config.llm.policies ??= {};
    config.llm.policies.cors = withPlaygroundCors(
      config.llm.policies.cors,
      target,
      origin,
    );
    return;
  }

  config.mcp ??= { targets: [] };
  config.mcp.policies ??= {};
  config.mcp.policies.cors = withPlaygroundCors(
    config.mcp.policies.cors,
    target,
    origin,
  );
}

function withPlaygroundCors(
  cors: CorsPolicy | null | undefined,
  target: CorsTarget,
  origin: string,
): CorsPolicy {
  return {
    ...(cors ?? {}),
    allowOrigins: appendUnique(cors?.allowOrigins, origin),
    allowHeaders: appendUnique(cors?.allowHeaders, "*"),
    allowMethods: appendUnique(appendUnique(cors?.allowMethods, "GET"), "POST"),
    exposeHeaders:
      target === "mcp"
        ? appendUnique(cors?.exposeHeaders, "Mcp-Session-Id")
        : cors?.exposeHeaders,
  };
}

function appendUnique(values: string[] | undefined, value: string) {
  const next = values ? [...values] : [];
  if (!hasValue(next, value)) next.push(value);
  return next;
}

function hasValue(values: string[] | undefined, value: string) {
  return Boolean(
    values?.some((item) => item.toLowerCase() === value.toLowerCase()),
  );
}
