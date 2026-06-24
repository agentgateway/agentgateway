import { useEffect, useMemo, useState } from "react";
import { publicAssetPath } from "./basePath";

type JsonObject = { [key: string]: unknown };
type Primitive =
  | string
  | number
  | boolean
  | bigint
  | symbol
  | null
  | undefined
  | ((...args: never[]) => unknown);
type DotPrefix<T extends string> = T extends "" ? "" : `.${T}`;
type KnownStringKeys<T> = {
  [K in keyof T]: K extends string ? (string extends K ? never : K) : never;
}[keyof T];
type StringKeys<T> = [KnownStringKeys<T>] extends [never]
  ? string
  : KnownStringKeys<T>;
type PropertyValue<T, K extends string> = K extends keyof T ? T[K] : unknown;
export type DeepPath<T> = T extends unknown
  ? DeepPathOne<NonNullable<T>>
  : never;
type DeepPathOne<T> = T extends Primitive
  ? never
  : T extends readonly (infer Item)[]
    ? `[]${DotPrefix<DeepPath<Item>>}`
    : T extends object
      ? {
          [K in StringKeys<T>]: NonNullable<
            PropertyValue<T, K>
          > extends Primitive
            ? K
            : K | `${K}.${DeepPath<NonNullable<PropertyValue<T, K>>>}`;
        }[StringKeys<T>]
      : never;

export type SchemaHelp = {
  node(path: Array<string | number>): unknown;
  resolve(node: unknown): unknown;
  description(
    path: Array<string | number>,
    fallback?: string,
  ): string | undefined;
  definition(defName: string, fallback?: string): string | undefined;
  fieldNode<T>(defName: string, propertyPath: DeepPath<T>): unknown;
  field<T>(
    defName: string,
    propertyPath: DeepPath<T>,
    fallback?: string,
  ): string | undefined;
  propertyNode(defName: string, propertyPath: string[]): unknown;
  propertyDescription(
    defName: string,
    propertyPath: string[],
    fallback?: string,
  ): string | undefined;
  objectProperties(path: Array<string | number>): string[];
};

const helpOverrides: Record<string, string> = {
  "$defs.CorsSerde.properties.allowOrigins":
    "Browser origins that may call this listener. Use exact origins such as http://localhost:19000.",
  "$defs.CorsSerde.properties.allowHeaders":
    "Request headers allowed by browser preflight checks. Use * while debugging, then narrow it for production.",
  "$defs.CorsSerde.properties.allowMethods":
    "HTTP methods allowed by browser preflight checks. Playgrounds typically need GET and POST.",
  "$defs.CorsSerde.properties.exposeHeaders":
    "Response headers browser JavaScript can read. MCP playgrounds need Mcp-Session-Id.",
  "$defs.LocalJwtConfig.oneOf.1.properties.mode":
    "strict requires a valid JWT, optional validates only when present, and permissive never rejects requests.",
  "$defs.LocalJwtConfig.oneOf.1.properties.issuer":
    "Expected issuer claim for accepted JWTs.",
  "$defs.LocalJwtConfig.oneOf.1.properties.audiences":
    "Accepted audience claims. Leave empty only when the gateway should not enforce audience matching.",
  "$defs.LocalJwtConfig.oneOf.1.properties.jwks":
    "JWKS used to validate JWT signatures. This may be inline JSON, a file reference, or a remote URL object.",
  "$defs.RateLimitSpec.properties.type":
    "Whether this limit counts requests immediately or tokens after an LLM response completes.",
  "$defs.RateLimitSpec.properties.fillInterval":
    "How often tokens are replenished, such as 1s, 60s, or 1m.",
  "$defs.RateLimitSpec.properties.maxTokens":
    "Maximum burst size for this local rate limit bucket.",
  "$defs.RateLimitSpec.properties.tokensPerFill":
    "Number of tokens added back to the bucket every fill interval.",
};

export function useSchemaHelp(): SchemaHelp {
  const [schema, setSchema] = useState<JsonObject | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch(publicAssetPath("config-schema.json"))
      .then((response) =>
        response.ok ? (response.json() as Promise<JsonObject>) : null,
      )
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

  return useMemo(
    () => ({
      node(path: Array<string | number>) {
        return readPath(schema, path);
      },
      resolve(node: unknown) {
        return resolveSchemaNode(schema, node);
      },
      description(path: Array<string | number>, fallback?: string) {
        const key = path.join(".");
        return (
          helpOverrides[key] ?? schemaDescription(schema, path) ?? fallback
        );
      },
      definition(defName: string, fallback?: string) {
        return schemaDescription(schema, ["$defs", defName]) ?? fallback;
      },
      fieldNode<T>(defName: string, propertyPath: DeepPath<T>) {
        return propertyNode(schema, defName, splitPropertyPath(propertyPath));
      },
      field<T>(defName: string, propertyPath: DeepPath<T>, fallback?: string) {
        return (
          propertyDescription(
            schema,
            defName,
            splitPropertyPath(propertyPath),
          ) ?? fallback
        );
      },
      propertyNode(defName: string, propertyPath: string[]) {
        return propertyNode(schema, defName, propertyPath);
      },
      propertyDescription(
        defName: string,
        propertyPath: string[],
        fallback?: string,
      ) {
        return propertyDescription(schema, defName, propertyPath) ?? fallback;
      },
      objectProperties(path: Array<string | number>) {
        const value = readPath(schema, path);
        if (!value || typeof value !== "object") return [];
        const properties = (value as { properties?: unknown }).properties;
        if (!properties || typeof properties !== "object") return [];
        return Object.keys(properties);
      },
    }),
    [schema],
  );
}

function schemaDescription(
  schema: JsonObject | null,
  path: Array<string | number>,
) {
  const value = readPath(schema, path);
  if (!value || typeof value !== "object") return undefined;
  const description = (value as { description?: unknown }).description;
  return typeof description === "string" && description.trim()
    ? description.trim()
    : undefined;
}

function propertyDescription(
  schema: JsonObject | null,
  defName: string,
  propertyPath: string[],
) {
  const node = propertyNode(schema, defName, propertyPath);
  if (!node || typeof node !== "object") return undefined;
  const description = (node as { description?: unknown }).description;
  return typeof description === "string" && description.trim()
    ? description.trim()
    : undefined;
}

function propertyNode(
  schema: JsonObject | null,
  defName: string,
  propertyPath: string[],
) {
  if (!propertyPath.length) return undefined;
  const root = resolveSchemaNode(schema, readPath(schema, ["$defs", defName]));
  return findPropertyNode(schema, root, propertyPath, new Set());
}

function splitPropertyPath(path: string) {
  return path
    .replaceAll("[]", ".[]")
    .split(".")
    .filter((part) => part && part !== "[]");
}

function findPropertyNode(
  schema: JsonObject | null,
  node: unknown,
  path: string[],
  seen: Set<unknown>,
): unknown {
  const resolved = resolveSchemaNode(schema, node);
  if (!resolved || typeof resolved !== "object" || seen.has(resolved))
    return undefined;
  seen.add(resolved);
  const record = resolved as Record<string, unknown>;
  const properties = record.properties;
  if (
    properties &&
    typeof properties === "object" &&
    !Array.isArray(properties)
  ) {
    const child = (properties as Record<string, unknown>)[path[0]];
    if (child !== undefined) {
      if (path.length === 1) return resolveSchemaNode(schema, child);
      const nested = findPropertyNode(schema, child, path.slice(1), seen);
      if (nested) return nested;
    }
  }
  const branches = [
    ...(Array.isArray(record.oneOf) ? record.oneOf : []),
    ...(Array.isArray(record.anyOf) ? record.anyOf : []),
  ];
  for (const branch of branches) {
    const found = findPropertyNode(schema, branch, path, seen);
    if (found) return found;
  }
  return undefined;
}

function readPath(value: unknown, path: Array<string | number>) {
  let current = value;
  for (const segment of path) {
    if (!current || typeof current !== "object") return undefined;
    current = (current as Record<string | number, unknown>)[segment];
  }
  return current;
}

function resolveSchemaNode(
  schema: JsonObject | null,
  node: unknown,
  seen = new Set<string>(),
): unknown {
  if (!node || typeof node !== "object") return node;
  const record = node as Record<string, unknown>;
  const ref = typeof record.$ref === "string" ? record.$ref : "";
  if (ref.startsWith("#/") && !seen.has(ref)) {
    seen.add(ref);
    const target = readPath(
      schema,
      ref
        .slice(2)
        .split("/")
        .map((part) => part.replaceAll("~1", "/").replaceAll("~0", "~")),
    );
    const resolved = resolveSchemaNode(schema, target, seen);
    if (resolved && typeof resolved === "object") {
      return {
        ...(resolved as Record<string, unknown>),
        ...withoutRef(record),
      };
    }
    return resolved;
  }
  return node;
}

function withoutRef(value: Record<string, unknown>) {
  const next = { ...value };
  delete next.$ref;
  return next;
}
