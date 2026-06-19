import type * as Monaco from "monaco-editor";
import {
  configureMonacoYaml,
  type JSONSchema,
  type MonacoYaml,
  type SchemasSettings,
} from "monaco-yaml";
import configSchema from "../../schema/config.json";
import { configureConfigMonacoWorkers } from "./monacoWorkers";

let yaml: MonacoYaml | null = null;
let completionsRegistered = false;
const registeredSchemas = new Map<string, SchemasSettings>();

const knownJsonSchemaFormats = new Set([
  "date",
  "date-time",
  "duration",
  "email",
  "hostname",
  "ipv4",
  "ipv6",
  "regex",
  "time",
  "uri",
  "uri-reference",
  "uuid",
]);

export const rawConfigModelPath = "agentgateway-config.yaml";
const rawConfigSchemaFileMatch = [
  rawConfigModelPath,
  `/${rawConfigModelPath}`,
  `**/${rawConfigModelPath}`,
  `file:///${rawConfigModelPath}`,
];
const monacoConfigSchema = toMonacoYamlSchema(configSchema) as JSONSchema;
const baseSchemas: SchemasSettings[] = [
  {
    uri: "file:///agentgateway-config-schema.json",
    fileMatch: rawConfigSchemaFileMatch,
    schema: monacoConfigSchema,
  },
];

export function configureConfigYamlMonaco(monaco: typeof Monaco) {
  configureConfigMonacoWorkers();
  if (yaml) return;

  yaml = configureMonacoYaml(monaco, {
    completion: false,
    hover: false,
    validate: false,
    schemas: [...baseSchemas, ...registeredSchemas.values()],
  });
  registerConfigYamlCompletions(monaco);
}

export { configureConfigMonacoWorkers };

export function registerConfigYamlSchema(
  monaco: typeof Monaco,
  path: string,
  schema: unknown,
) {
  configureConfigYamlMonaco(monaco);
  const uri = `file:///agentgateway-${path}-schema.json`;
  registeredSchemas.set(path, {
    uri,
    fileMatch: [path, `/${path}`, `**/${path}`, `file:///${path}`],
    schema: configSubschema(schema),
  });
  void yaml?.update({
    schemas: [...baseSchemas, ...registeredSchemas.values()],
  });
}

export function installYamlKeySuggest(
  editor: Monaco.editor.IStandaloneCodeEditor,
) {
  let promptedEmpty = false;

  editor.onDidFocusEditorText(() => {
    const model = editor.getModel();
    if (!model || promptedEmpty || model.getValue().trim()) return;
    promptedEmpty = true;
    window.setTimeout(() => {
      editor.trigger("yaml-empty-focus", "editor.action.triggerSuggest", {});
    }, 50);
  });

  editor.onDidChangeModelContent((event) => {
    if (!event.isFlush && editor.getModel()?.getValue().trim()) return;
    promptedEmpty = false;
  });
}

export function configSubschema(schema: unknown): JSONSchema {
  const next = toMonacoYamlSchema(schema) as JSONSchema;
  return normalizeEditorSchema({
    ...next,
    definitions: {
      ...((monacoConfigSchema.definitions ?? {}) as Record<string, JSONSchema>),
      ...((next.definitions ?? {}) as Record<string, JSONSchema>),
    },
  });
}

export function toMonacoYamlSchema(schema: unknown): unknown {
  if (Array.isArray(schema)) return schema.map(toMonacoYamlSchema);
  if (!schema || typeof schema !== "object") return schema;

  const source = schema as Record<string, unknown>;
  const next: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(source)) {
    if (key === "$schema") {
      next.$schema = "http://json-schema.org/draft-07/schema#";
      continue;
    }
    if (key === "$defs") {
      next.definitions = toMonacoYamlSchema(value);
      continue;
    }
    if (key === "$ref" && typeof value === "string") {
      next.$ref = value.replace("#/$defs/", "#/definitions/");
      continue;
    }
    if (
      key === "format" &&
      typeof value === "string" &&
      !knownJsonSchemaFormats.has(value)
    ) {
      continue;
    }
    next[key] = toMonacoYamlSchema(value);
  }
  return next;
}

function registerConfigYamlCompletions(monaco: typeof Monaco) {
  if (completionsRegistered) return;
  completionsRegistered = true;

  monaco.languages.registerCompletionItemProvider("yaml", {
    triggerCharacters: [" ", "\n"],
    provideCompletionItems(model, position) {
      const schemaRoot = schemaForModel(model);
      if (!schemaRoot) return { suggestions: [] };
      const line = model.getLineContent(position.lineNumber);
      const prefix = line.slice(0, position.column - 1);
      if (prefix.includes(":")) return { suggestions: [] };

      const path = yamlPathAt(model, position.lineNumber);
      const schema = objectSchemaAtPath(schemaRoot, path);
      const properties =
        schema &&
        typeof schema.properties === "object" &&
        !Array.isArray(schema.properties)
          ? (schema.properties as Record<string, Record<string, unknown>>)
          : undefined;
      if (!properties) return { suggestions: [] };

      const existing = siblingKeysAt(model, position.lineNumber);
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      return {
        suggestions: Object.entries(properties)
          .filter(([name]) => !existing.has(name))
          .map(([name, property]) => {
            const resolved = resolveSchema(property);
            const structured = isStructuredSchema(resolved);
            return {
              label: name,
              kind: monaco.languages.CompletionItemKind.Property,
              detail: schemaTypeLabel(resolved),
              documentation:
                typeof resolved.description === "string"
                  ? resolved.description
                  : undefined,
              insertText: structured ? `${name}:\n  ` : `${name}: `,
              range,
            };
          }),
      };
    },
  });
}

function schemaForModel(model: Monaco.editor.ITextModel) {
  const uri = model.uri.toString();
  if (uri === `file:///${rawConfigModelPath}`)
    return monacoConfigSchema as Record<string, unknown>;
  const path = uri.startsWith("file:///")
    ? uri.slice("file:///".length)
    : (uri.split("/").pop() ?? "");
  const schema = registeredSchemas.get(path)?.schema;
  return schema && typeof schema === "object" && !Array.isArray(schema)
    ? (schema as Record<string, unknown>)
    : undefined;
}

function yamlPathAt(model: Monaco.editor.ITextModel, lineNumber: number) {
  const currentIndent = leadingSpaces(model.getLineContent(lineNumber));
  const stack: Array<{ indent: number; key: string }> = [];
  for (let line = 1; line < lineNumber; line += 1) {
    const content = model.getLineContent(line);
    const match = content.match(/^(\s*)([A-Za-z0-9_-]+)\s*:\s*(?:#.*)?$/);
    if (!match) continue;
    const indent = match[1].length;
    while (stack.length && stack[stack.length - 1].indent >= indent)
      stack.pop();
    stack.push({ indent, key: match[2] });
  }
  return stack
    .filter((entry) => entry.indent < currentIndent)
    .map((entry) => entry.key);
}

function siblingKeysAt(model: Monaco.editor.ITextModel, lineNumber: number) {
  const currentIndent = leadingSpaces(model.getLineContent(lineNumber));
  const keys = new Set<string>();
  for (let line = 1; line <= model.getLineCount(); line += 1) {
    if (line === lineNumber) continue;
    const content = model.getLineContent(line);
    if (leadingSpaces(content) !== currentIndent) continue;
    const match = content.match(/^\s*([A-Za-z0-9_-]+)\s*:/);
    if (match) keys.add(match[1]);
  }
  return keys;
}

function objectSchemaAtPath(
  schema: Record<string, unknown>,
  path: string[],
): Record<string, unknown> | undefined {
  let current: Record<string, unknown> | undefined = resolveSchema(schema);
  for (const segment of path) {
    if (!current) return undefined;
    if (
      current.type === "array" &&
      current.items &&
      typeof current.items === "object" &&
      !Array.isArray(current.items)
    ) {
      current = resolveSchema(current.items as Record<string, unknown>);
    }
    const properties = current.properties;
    if (
      !properties ||
      typeof properties !== "object" ||
      Array.isArray(properties)
    )
      return undefined;
    current = resolveSchema(
      (properties as Record<string, Record<string, unknown>>)[segment],
    );
  }
  return current;
}

function resolveSchema(
  schema: Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (!schema) return {};
  if (typeof schema.$ref === "string") {
    const resolved = schema.$ref.startsWith("#/definitions/")
      ? getByPath(
          monacoConfigSchema as Record<string, unknown>,
          schema.$ref.slice(2).split("/"),
        )
      : undefined;
    return resolveSchema(
      resolved && typeof resolved === "object" && !Array.isArray(resolved)
        ? (resolved as Record<string, unknown>)
        : {},
    );
  }
  const anyOf = Array.isArray(schema.anyOf)
    ? schema.anyOf
    : Array.isArray(schema.oneOf)
      ? schema.oneOf
      : undefined;
  if (anyOf) {
    const branch = selectEditableBranch(anyOf);
    return resolveSchema(branch as Record<string, unknown> | undefined);
  }
  if (Array.isArray(schema.allOf)) {
    return schema.allOf.reduce<Record<string, unknown>>((merged, candidate) => {
      const resolved = resolveSchema(candidate as Record<string, unknown>);
      return {
        ...merged,
        ...resolved,
        properties: {
          ...(typeof merged.properties === "object" &&
          !Array.isArray(merged.properties)
            ? merged.properties
            : {}),
          ...(typeof resolved.properties === "object" &&
          !Array.isArray(resolved.properties)
            ? resolved.properties
            : {}),
        },
      };
    }, {});
  }
  return schema;
}

function normalizeEditorSchema(schema: JSONSchema): JSONSchema {
  if (!schema || typeof schema !== "object" || Array.isArray(schema))
    return schema;
  const normalized = normalizeEditorSchemaNode(
    schema as Record<string, unknown>,
    schema as Record<string, unknown>,
  );
  return normalized as JSONSchema;
}

function normalizeEditorSchemaNode(
  schema: Record<string, unknown>,
  root: Record<string, unknown>,
): Record<string, unknown> {
  if (typeof schema.$ref === "string") {
    const resolved = resolveSchemaReference(root, schema.$ref);
    if (resolved) {
      return {
        ...normalizeEditorSchemaNode(resolved, root),
        ...copySchemaAnnotations(schema),
      };
    }
  }

  const branches = Array.isArray(schema.anyOf)
    ? schema.anyOf
    : Array.isArray(schema.oneOf)
      ? schema.oneOf
      : undefined;
  if (branches) {
    if (shouldCollapseBranches(branches, root)) {
      const branch = selectEditableBranch(branches, root);
      if (branch && typeof branch === "object" && !Array.isArray(branch)) {
        return {
          ...normalizeEditorSchemaNode(branch as Record<string, unknown>, root),
          ...copySchemaAnnotations(schema),
        };
      }
    }
  }

  const next: Record<string, unknown> = { ...schema };
  if (
    next.properties &&
    typeof next.properties === "object" &&
    !Array.isArray(next.properties)
  ) {
    next.properties = Object.fromEntries(
      Object.entries(next.properties).map(([key, value]) => [
        key,
        value && typeof value === "object" && !Array.isArray(value)
          ? normalizeEditorSchemaNode(value as Record<string, unknown>, root)
          : value,
      ]),
    );
  }
  if (
    next.items &&
    typeof next.items === "object" &&
    !Array.isArray(next.items)
  ) {
    next.items = normalizeEditorSchemaNode(
      next.items as Record<string, unknown>,
      root,
    );
  }
  if (
    next.additionalProperties &&
    typeof next.additionalProperties === "object" &&
    !Array.isArray(next.additionalProperties)
  ) {
    next.additionalProperties = normalizeEditorSchemaNode(
      next.additionalProperties as Record<string, unknown>,
      root,
    );
  }
  if (Array.isArray(next.anyOf)) {
    next.anyOf = next.anyOf.map((branch) =>
      branch && typeof branch === "object" && !Array.isArray(branch)
        ? normalizeEditorSchemaNode(branch as Record<string, unknown>, root)
        : branch,
    );
  }
  if (Array.isArray(next.oneOf)) {
    next.oneOf = next.oneOf.map((branch) =>
      branch && typeof branch === "object" && !Array.isArray(branch)
        ? normalizeEditorSchemaNode(branch as Record<string, unknown>, root)
        : branch,
    );
  }
  return next;
}

function selectEditableBranch(
  branches: unknown[],
  root: Record<string, unknown> = monacoConfigSchema as Record<string, unknown>,
) {
  const nonNull = branches.filter((candidate) => {
    const resolved = resolveBranchCandidate(candidate, root);
    return resolved?.type !== "null";
  });
  return (
    nonNull.find((candidate) => !isConditionalPolicyBranch(candidate, root)) ??
    nonNull[0]
  );
}

function shouldCollapseBranches(
  branches: unknown[],
  root: Record<string, unknown>,
) {
  const nonNull = branches.filter(
    (candidate) => resolveBranchCandidate(candidate, root)?.type !== "null",
  );
  return (
    nonNull.length === 1 ||
    nonNull.some((candidate) => isConditionalPolicyBranch(candidate, root))
  );
}

function isConditionalPolicyBranch(
  candidate: unknown,
  root: Record<string, unknown>,
) {
  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate))
    return false;
  const schema = candidate as Record<string, unknown>;
  if (
    typeof schema.$ref === "string" &&
    /#\/definitions\/LocalConditionalPolicies\d*$/.test(schema.$ref)
  )
    return true;
  const resolved = resolveBranchCandidate(candidate, root);
  const properties = resolved?.properties;
  if (
    !properties ||
    typeof properties !== "object" ||
    Array.isArray(properties)
  )
    return false;
  const keys = Object.keys(properties);
  return keys.length === 1 && keys[0] === "conditional";
}

function resolveBranchCandidate(
  candidate: unknown,
  root: Record<string, unknown>,
) {
  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate))
    return undefined;
  const schema = candidate as Record<string, unknown>;
  if (typeof schema.$ref === "string")
    return resolveSchemaReference(root, schema.$ref);
  return schema;
}

function resolveSchemaReference(root: Record<string, unknown>, ref: string) {
  if (!ref.startsWith("#/definitions/")) return undefined;
  const resolved = getByPath(root, ref.slice(2).split("/"));
  return resolved && typeof resolved === "object" && !Array.isArray(resolved)
    ? (resolved as Record<string, unknown>)
    : undefined;
}

function copySchemaAnnotations(schema: Record<string, unknown>) {
  const annotations: Record<string, unknown> = {};
  for (const key of ["description", "title", "default", "examples"]) {
    if (schema[key] !== undefined) annotations[key] = schema[key];
  }
  return annotations;
}

function getByPath(source: Record<string, unknown>, path: string[]) {
  return path.reduce<unknown>((current, segment) => {
    if (!current || typeof current !== "object" || Array.isArray(current))
      return undefined;
    return (current as Record<string, unknown>)[segment];
  }, source);
}

function isStructuredSchema(schema: Record<string, unknown>) {
  if (schema.type === "object" || schema.type === "array") return true;
  return Boolean(schema.properties || schema.items);
}

function schemaTypeLabel(schema: Record<string, unknown>) {
  if (typeof schema.type === "string") return schema.type;
  if (Array.isArray(schema.enum)) return "enum";
  if (schema.properties) return "object";
  if (schema.items) return "array";
  return undefined;
}

function leadingSpaces(value: string) {
  return value.match(/^\s*/)?.[0].length ?? 0;
}
