import { useEffect, useMemo, useState } from "react";

type JsonObject = { [key: string]: unknown };

export type SchemaHelp = {
  node(path: Array<string | number>): unknown;
  description(path: Array<string | number>, fallback?: string): string | undefined;
  objectProperties(path: Array<string | number>): string[];
};

const helpOverrides: Record<string, string> = {
  "$defs.CorsSerde.properties.allowOrigins": "Browser origins that may call this listener. Use exact origins such as http://localhost:19000.",
  "$defs.CorsSerde.properties.allowHeaders": "Request headers allowed by browser preflight checks. Use * while debugging, then narrow it for production.",
  "$defs.CorsSerde.properties.allowMethods": "HTTP methods allowed by browser preflight checks. Playgrounds typically need GET and POST.",
  "$defs.CorsSerde.properties.exposeHeaders": "Response headers browser JavaScript can read. MCP playgrounds need Mcp-Session-Id.",
  "$defs.LocalJwtConfig.oneOf.1.properties.mode": "strict requires a valid JWT, optional validates only when present, and permissive never rejects requests.",
  "$defs.LocalJwtConfig.oneOf.1.properties.issuer": "Expected issuer claim for accepted JWTs.",
  "$defs.LocalJwtConfig.oneOf.1.properties.audiences": "Accepted audience claims. Leave empty only when the gateway should not enforce audience matching.",
  "$defs.LocalJwtConfig.oneOf.1.properties.jwks": "JWKS used to validate JWT signatures. This may be inline JSON, a file reference, or a remote URL object.",
  "$defs.RateLimitSpec.properties.type": "Whether this limit counts requests immediately or tokens after an LLM response completes.",
  "$defs.RateLimitSpec.properties.fillInterval": "How often tokens are replenished, such as 1s, 60s, or 1m.",
  "$defs.RateLimitSpec.properties.maxTokens": "Maximum burst size for this local rate limit bucket.",
  "$defs.RateLimitSpec.properties.tokensPerFill": "Number of tokens added back to the bucket every fill interval.",
};

export function useSchemaHelp(): SchemaHelp {
  const [schema, setSchema] = useState<JsonObject | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/config-schema.json")
      .then((response) => response.ok ? response.json() as Promise<JsonObject> : null)
      .then((value) => {
        if (!cancelled) setSchema(value);
      })
      .catch(() => {
        if (!cancelled) setSchema(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return useMemo(() => ({
    node(path: Array<string | number>) {
      return readPath(schema, path);
    },
    description(path: Array<string | number>, fallback?: string) {
      const key = path.join(".");
      return helpOverrides[key] ?? schemaDescription(schema, path) ?? fallback;
    },
    objectProperties(path: Array<string | number>) {
      const value = readPath(schema, path);
      if (!value || typeof value !== "object") return [];
      const properties = (value as { properties?: unknown }).properties;
      if (!properties || typeof properties !== "object") return [];
      return Object.keys(properties);
    },
  }), [schema]);
}

function schemaDescription(schema: JsonObject | null, path: Array<string | number>) {
  const value = readPath(schema, path);
  if (!value || typeof value !== "object") return undefined;
  const description = (value as { description?: unknown }).description;
  return typeof description === "string" && description.trim() ? description.trim() : undefined;
}

function readPath(value: unknown, path: Array<string | number>) {
  let current = value;
  for (const segment of path) {
    if (!current || typeof current !== "object") return undefined;
    current = (current as Record<string | number, unknown>)[segment];
  }
  return current;
}
