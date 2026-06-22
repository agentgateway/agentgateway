import yaml from "js-yaml";
import type { CorsPolicy, LlmPolicy } from "../types";
import type { LocalRateLimitConfig, SchemaNode } from "./types";

export function policyEnabled(
  policies: (LlmPolicy | Record<string, unknown>) | null | undefined,
  key: string,
) {
  const value = (policies as Record<string, unknown> | null | undefined)?.[key];
  if (Array.isArray(value)) return value.length > 0;
  return value !== undefined && value !== null;
}

export function policySummary(
  policies: (LlmPolicy | Record<string, unknown>) | null | undefined,
  key: string,
) {
  const value = (policies as Record<string, unknown> | null | undefined)?.[key];
  if (!policyEnabled(policies, key)) return "";
  if (key === "cors") {
    const cors = value as CorsPolicy;
    return `${cors.allowOrigins?.length ?? 0} origins, ${(cors.allowMethods ?? []).join(", ") || "no methods"}`;
  }
  if (key === "jwtAuth") {
    const jwt = value as { mode?: string; issuer?: string };
    return `${jwt.mode ?? "strict"}${jwt.issuer ? `, ${jwt.issuer}` : ""}`;
  }
  if (key === "oidc") {
    const oidc = value as { issuer?: string; clientId?: string };
    return (
      [oidc.clientId, oidc.issuer].filter(Boolean).join(", ") ||
      "Browser login configured"
    );
  }
  if (key === "apiKey") {
    const apiKey = value as { keys?: unknown[]; mode?: string };
    return `${apiKey.keys?.length ?? 0} keys, ${apiKey.mode ?? "strict"}`;
  }
  if (key === "localRateLimit") {
    const limits = value as LocalRateLimitConfig;
    if (!Array.isArray(limits)) return "Conditional limits";
    const first = limits[0];
    return first
      ? `${first.type ?? "requests"} every ${first.fillInterval}`
      : "Configured";
  }
  if (key === "authorization") {
    const authorization = value as {
      rules?:
        | Array<unknown>
        | { allow?: unknown[]; deny?: unknown[]; require?: unknown[] };
    };
    const grouped =
      authorization.rules && !Array.isArray(authorization.rules)
        ? authorization.rules
        : {};
    const ordered = Array.isArray(authorization.rules)
      ? authorization.rules
      : [];
    const allow =
      (grouped.allow?.length ?? 0) +
      ordered.filter(
        (rule) =>
          typeof rule === "string" ||
          Boolean(rule && typeof rule === "object" && "allow" in rule),
      ).length;
    const deny =
      (grouped.deny?.length ?? 0) +
      ordered.filter((rule) =>
        Boolean(rule && typeof rule === "object" && "deny" in rule),
      ).length;
    const require =
      (grouped.require?.length ?? 0) +
      ordered.filter((rule) =>
        Boolean(rule && typeof rule === "object" && "require" in rule),
      ).length;
    return `${allow} allow, ${deny} deny, ${require} require`;
  }
  return "Configured";
}

export function schemaType(schema: SchemaNode | undefined) {
  if (!schema) return undefined;
  if (schema.const !== undefined) return typeof schema.const;
  if (schema.enum?.length) return typeof schema.enum[0];
  if (Array.isArray(schema.type))
    return schema.type.find((type) => type !== "null");
  if (schema.type) return schema.type;
  if (schema.properties) return "object";
  if (schema.items) return "array";
  return undefined;
}

export function enumOptionDetails(
  schema: SchemaNode | undefined,
): Array<{ value: string; label: string; description?: string }> {
  if (!schema) return [];
  if (schema.enum)
    return schema.enum.map((value) => ({
      value: String(value),
      label: String(value),
    }));
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants?.length)
    return variants.flatMap((item) => {
      if (item.const !== undefined) {
        return [
          {
            value: String(item.const),
            label: String(item.const),
            description: item.description,
          },
        ];
      }
      const properties = item.properties ? Object.keys(item.properties) : [];
      if (properties.length === 1) {
        return [
          {
            value: properties[0],
            label: properties[0],
            description: item.description,
          },
        ];
      }
      return [];
    });
  return [];
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function cleanEmpty(value: unknown): unknown {
  if (Array.isArray(value)) {
    const next = value.map(cleanEmpty).filter((item) => item !== undefined);
    return next.length > 0 ? next : undefined;
  }
  if (!isRecord(value))
    return value === "" || value === null ? undefined : value;
  const next: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    const cleaned = cleanEmpty(item);
    if (cleaned !== undefined) next[key] = cleaned;
  }
  return Object.keys(next).length > 0 ? next : undefined;
}

export function lines(values: string[] | undefined) {
  return values?.join("\n") ?? "";
}

export function toText(value: unknown) {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

export function toYamlText(value: unknown) {
  return yaml.dump(value, { noRefs: true, lineWidth: 100 });
}

export function toYamlMappingText(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return "";
  return Object.keys(value).length ? toYamlText(value) : "";
}

export function parseYamlText(value: string) {
  return yaml.load(value) as unknown;
}

export function appendUnique(values: string[], value: string) {
  return values.some((item) => item.toLowerCase() === value.toLowerCase())
    ? values
    : [...values, value];
}

export function toggleStringSet(values: Set<string>, value: string) {
  const next = new Set(values);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  return next;
}

export function titleFromKey(key: string) {
  const title = key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/^./, (first) => first.toUpperCase());
  return title.replace(
    /\b(Api|Jwt|Oidc|Cors|Csrf|A2a|Ai|Mcp|Http|Tcp|Tls|Url)\b/g,
    (word) => {
      const acronyms: Record<string, string> = {
        A2a: "A2A",
        Ai: "AI",
        Api: "API",
        Cors: "CORS",
        Csrf: "CSRF",
        Http: "HTTP",
        Jwt: "JWT",
        Mcp: "MCP",
        Oidc: "OIDC",
        Tcp: "TCP",
        Tls: "TLS",
        Url: "URL",
      };
      return acronyms[word] ?? word;
    },
  );
}
