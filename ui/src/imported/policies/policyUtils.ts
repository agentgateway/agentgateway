import yaml from "js-yaml";
import type { SchemaHelp } from "../schemaHelp";
import type { CorsPolicy, LlmPolicy } from "../types";
import type { LocalRateLimitConfig, SchemaNode } from "./types";

export function policyEnabled(policies: (LlmPolicy | Record<string, unknown>) | null | undefined, key: string) {
  const value = (policies as Record<string, unknown> | null | undefined)?.[key];
  if (Array.isArray(value)) return value.length > 0;
  return value !== undefined && value !== null;
}

export function policySummary(policies: (LlmPolicy | Record<string, unknown>) | null | undefined, key: string) {
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
    return [oidc.clientId, oidc.issuer].filter(Boolean).join(", ") || "Browser login configured";
  }
  if (key === "apiKey") {
    const apiKey = value as { keys?: unknown[]; mode?: string };
    return `${apiKey.keys?.length ?? 0} keys, ${apiKey.mode ?? "strict"}`;
  }
  if (key === "localRateLimit") {
    const limits = value as LocalRateLimitConfig;
    if (!Array.isArray(limits)) return "Conditional limits";
    const first = limits[0];
    return first ? `${first.type ?? "requests"} every ${first.fillInterval}` : "Configured";
  }
  if (key === "authorization") {
    const authorization = value as {
      rules?: Array<unknown> | { allow?: unknown[]; deny?: unknown[]; require?: unknown[] };
    };
    const grouped = authorization.rules && !Array.isArray(authorization.rules) ? authorization.rules : {};
    const ordered = Array.isArray(authorization.rules) ? authorization.rules : [];
    const allow = (grouped.allow?.length ?? 0) + ordered.filter((rule) => typeof rule === "string" || Boolean(rule && typeof rule === "object" && "allow" in rule)).length;
    const deny = (grouped.deny?.length ?? 0) + ordered.filter((rule) => Boolean(rule && typeof rule === "object" && "deny" in rule)).length;
    const require = (grouped.require?.length ?? 0) + ordered.filter((rule) => Boolean(rule && typeof rule === "object" && "require" in rule)).length;
    return `${allow} allow, ${deny} deny, ${require} require`;
  }
  return "Configured";
}

export function resolveSchema(help: SchemaHelp, value: unknown): SchemaNode | undefined {
  if (!value || typeof value !== "object") return undefined;
  const schema = value as SchemaNode;
  if (schema.$ref) {
    const explicitRef = explicitBranchRef(schema.$ref);
    if (explicitRef) return resolveSchema(help, help.node(refToPath(explicitRef)));
    return resolveSchema(help, help.node(refToPath(schema.$ref)));
  }
  const variants = schema.anyOf ?? schema.oneOf;
  if (variants?.length) {
    if (variants.every((item) => item.const !== undefined)) return schema;
    const explicit = variants.find((item) => item.$ref && !item.$ref.includes("LocalConditionalPolicies"));
    const nonNull = variants.find((item) => schemaType(item) !== "null");
    return resolveSchema(help, explicit ?? nonNull ?? variants[0]);
  }
  return schema;
}

export function schemaObjectProperties(schema: SchemaNode | undefined) {
  if (!schema) return undefined;
  const resolved = schema.properties;
  return resolved && Object.keys(resolved).length > 0 ? resolved : undefined;
}

export function schemaType(schema: SchemaNode | undefined) {
  if (!schema) return undefined;
  if (schema.const !== undefined) return typeof schema.const;
  if (schema.enum?.length) return typeof schema.enum[0];
  if (Array.isArray(schema.type)) return schema.type.find((type) => type !== "null");
  if (schema.type) return schema.type;
  if (schema.properties) return "object";
  if (schema.items) return "array";
  return undefined;
}

export function enumOptionDetails(schema: SchemaNode | undefined): Array<{ value: string; label: string; description?: string }> {
  if (!schema) return [];
  if (schema.enum) return schema.enum.map((value) => ({ value: String(value), label: String(value) }));
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants?.every((item) => item.const !== undefined)) {
    return variants.map((item) => ({
      value: String(item.const),
      label: String(item.const),
      description: item.description,
    }));
  }
  return [];
}

export function placeholderForSchema(schema: SchemaNode | undefined) {
  if (!schema || schema.default === undefined || schema.default === null) return undefined;
  if (Array.isArray(schema.default)) return schema.default.join("\n");
  if (typeof schema.default === "object") return JSON.stringify(schema.default);
  return String(schema.default);
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function cleanEmpty(value: unknown): unknown {
  if (Array.isArray(value)) {
    const next = value.map(cleanEmpty).filter((item) => item !== undefined);
    return next.length > 0 ? next : undefined;
  }
  if (!isRecord(value)) return value === "" || value === null ? undefined : value;
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

export function splitList(value: string) {
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function toText(value: unknown) {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

export function toYamlText(value: unknown) {
  return yaml.dump(value, { noRefs: true, lineWidth: 100 });
}

export function parseYamlText(value: string) {
  return yaml.load(value) as unknown;
}

export function parseJsonOrString(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return trimmed;
  }
}

export function appendUnique(values: string[], value: string) {
  return values.some((item) => item.toLowerCase() === value.toLowerCase()) ? values : [...values, value];
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
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/^./, (first) => first.toUpperCase());
}

function explicitBranchRef(ref: string) {
  const explicitOrConditional = ref.match(/^#\/\$defs\/LocalExplicitOrConditional\d*$/);
  if (!explicitOrConditional) return undefined;
  const schemaName = ref.slice("#/$defs/".length);
  const explicitBySchema: Record<string, string> = {
    LocalExplicitOrConditional: "#/$defs/DirectResponse",
    LocalExplicitOrConditional2: "#/$defs/RemoteRateLimit",
    LocalExplicitOrConditional3: "#/$defs/ExtAuthz",
    LocalExplicitOrConditional4: "#/$defs/ExtProc",
    LocalExplicitOrConditional5: "#/$defs/LocalTransformationConfig",
  };
  return explicitBySchema[schemaName];
}

function refToPath(ref: string) {
  return ref.replace(/^#\//, "").split("/");
}
