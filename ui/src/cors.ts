import type { CorsSerde, FilterOrPolicy, LocalConfig } from "./config";

export function currentOrigin() {
  return window.location.origin;
}

export function corsNeedsUpdate(cors: CorsSerde | null | undefined, origin = currentOrigin()): boolean {
  if (!cors) return true;
  return (
    !hasValue(cors.allowOrigins, origin) ||
    !hasValue(cors.allowMethods, "GET") ||
    !hasValue(cors.allowMethods, "POST") ||
    !hasValue(cors.allowHeaders, "*")
  );
}

export function mcpCorsNeedsUpdate(cors: CorsSerde | null | undefined, origin = currentOrigin()): boolean {
  if (!cors) return true;
  return (
    !hasValue(cors.allowOrigins, origin) ||
    !hasValue(cors.allowMethods, "GET") ||
    !hasValue(cors.allowMethods, "POST") ||
    !hasValue(cors.allowHeaders, "*") ||
    !hasValue(cors.exposeHeaders, "Mcp-Session-Id")
  );
}

export function applyMcpPlaygroundCors(config: LocalConfig, origin = currentOrigin()): LocalConfig {
  const next = { ...config };
  const existingCors = (next.mcp?.policies as FilterOrPolicy | null | undefined)?.cors;
  next.mcp = {
    ...(next.mcp ?? { targets: [] }),
    policies: {
      ...(next.mcp?.policies ?? {}),
      cors: withMcpPlaygroundCors(existingCors, origin),
    },
  };
  return next;
}

function withMcpPlaygroundCors(cors: CorsSerde | null | undefined, origin: string): CorsSerde {
  return {
    ...(cors ?? {}),
    allowOrigins: appendUnique(cors?.allowOrigins, origin),
    allowHeaders: appendUnique(cors?.allowHeaders, "*"),
    allowMethods: appendUnique(appendUnique(cors?.allowMethods, "GET"), "POST"),
    exposeHeaders: appendUnique(cors?.exposeHeaders, "Mcp-Session-Id"),
  };
}

export function applyPlaygroundCors(config: LocalConfig, origin = currentOrigin()): LocalConfig {
  const next = { ...config };
  next.llm = {
    ...(next.llm ?? { models: [] }),
    policies: {
      ...(next.llm?.policies ?? {}),
      cors: withPlaygroundCors(next.llm?.policies?.cors, origin),
    },
  };
  return next;
}

function withPlaygroundCors(cors: CorsSerde | null | undefined, origin: string): CorsSerde {
  return {
    ...(cors ?? {}),
    allowOrigins: appendUnique(cors?.allowOrigins, origin),
    allowHeaders: appendUnique(cors?.allowHeaders, "*"),
    allowMethods: appendUnique(appendUnique(cors?.allowMethods, "GET"), "POST"),
  };
}

function appendUnique(values: string[] | undefined, value: string): string[] {
  const next = values ? [...values] : [];
  if (!hasValue(next, value)) next.push(value);
  return next;
}

function hasValue(values: string[] | undefined, value: string): boolean {
  return Boolean(values?.some((item) => item.toLowerCase() === value.toLowerCase()));
}
