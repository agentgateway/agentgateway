import { YamlTextBlock } from "./Primitives";
import type { SchemaHelp } from "../schemaHelp";

export function SchemaHelpPanel(props: {
  schema: unknown;
  help: SchemaHelp;
  summary?: string;
  showDescription?: boolean;
}) {
  const description = schemaTopLevelDescription(props.schema, props.help);
  const yaml = schemaHelpYaml(props.schema, props.help);
  return (
    <>
      {props.showDescription !== false && description ? (
        <p className="policy-schema-description">{description}</p>
      ) : null}
      {yaml ? (
        <details className="schema-details policy-schema-help">
          <summary>{props.summary ?? "Help"}</summary>
          <YamlTextBlock className="policy-schema-shape" value={yaml} />
        </details>
      ) : null}
    </>
  );
}

type SchemaNode = {
  $ref?: string;
  anyOf?: SchemaNode[];
  oneOf?: SchemaNode[];
  allOf?: SchemaNode[];
  const?: unknown;
  default?: unknown;
  description?: string;
  enum?: unknown[];
  items?: SchemaNode;
  properties?: Record<string, SchemaNode>;
  required?: string[];
  type?: string | string[];
};

export function schemaHelpYaml(schema: unknown, help: SchemaHelp) {
  const raw = asSchemaNode(schema);
  if (!raw) return "";
  const resolved = resolveDisplayNode(raw, help);
  return schemaNodeLines(resolved, help, 0, { maxDepth: 4 }).join("\n").trim();
}

export function schemaTopLevelDescription(schema: unknown, help: SchemaHelp) {
  const raw = asSchemaNode(schema);
  if (!raw) return "";
  const resolved = resolveDisplayNode(raw, help);
  return cleanDescription(raw.description ?? resolved.description);
}

function schemaNodeLines(
  schema: SchemaNode,
  help: SchemaHelp,
  indent: number,
  options: { maxDepth: number },
): string[] {
  const resolved = resolveDisplayNode(schema, help);
  const variantNodes = displayVariants(resolved, help);
  if (variantNodes.length === 1 && !resolved.properties) {
    return schemaNodeLines(variantNodes[0], help, indent, options);
  }
  if (variantNodes.length > 1 && !resolved.properties) {
    const lines: string[] = [];
    pushComment(lines, "One of:", indent);
    for (const variant of variantNodes.slice(0, 4)) {
      const label = schemaTypeLabel(variant);
      pushComment(
        lines,
        `- ${label}${variant.description ? `: ${cleanDescription(variant.description)}` : ""}`,
        indent,
      );
    }
    const firstObject = variantNodes.find(
      (variant) => resolveDisplayNode(variant, help).properties,
    );
    if (firstObject)
      lines.push(
        ...schemaNodeLines(firstObject, help, indent, {
          maxDepth: options.maxDepth - 1,
        }),
      );
    return lines;
  }

  if (!resolved.properties || options.maxDepth <= 0) {
    return [`${spaces(indent)}${placeholderForSchema(resolved)}`];
  }

  const required = new Set(resolved.required ?? []);
  const entries = Object.entries(resolved.properties);
  const lines: string[] = [];
  for (const [key, child] of entries) {
    const childResolved = resolveDisplayNode(child, help);
    const childDescription = cleanDescription(
      child.description ?? childResolved.description,
    );
    if (childDescription) pushComment(lines, childDescription, indent);
    const meta = [
      schemaTypeLabel(childResolved),
      required.has(key) ? "required" : "",
    ]
      .filter(Boolean)
      .join(", ");
    if (meta) pushComment(lines, meta, indent);

    const childVariants = displayVariants(childResolved, help);
    const objectVariant = childVariants.find(
      (variant) => resolveDisplayNode(variant, help).properties,
    );
    const objectNode = objectVariant
      ? resolveDisplayNode(objectVariant, help)
      : childResolved;
    if (objectNode.properties && options.maxDepth > 1) {
      lines.push(`${spaces(indent)}${key}:`);
      lines.push(
        ...schemaNodeLines(objectNode, help, indent + 2, {
          maxDepth: options.maxDepth - 1,
        }),
      );
    } else {
      lines.push(
        `${spaces(indent)}${key}: ${placeholderForSchema(childResolved)}`,
      );
    }
    lines.push("");
  }
  while (lines.at(-1) === "") lines.pop();
  return lines;
}

function resolveDisplayNode(node: SchemaNode, help: SchemaHelp): SchemaNode {
  return asSchemaNode(help.resolve(node)) ?? node;
}

function displayVariants(node: SchemaNode, help: SchemaHelp) {
  const variants = node.oneOf ?? node.anyOf ?? [];
  return variants
    .map((variant) => resolveDisplayNode(variant, help))
    .filter((variant) => !schemaTypeValues(variant).includes("null"));
}

function schemaTypeLabel(node: SchemaNode) {
  if (node.const !== undefined) return JSON.stringify(node.const);
  if (node.enum?.length)
    return node.enum.map((value) => JSON.stringify(value)).join(" | ");
  const variants = displayVariantLabels(node);
  if (variants.length) return variants.join(" | ");
  const types = schemaTypeValues(node).filter((type) => type !== "null");
  if (types.length) return types.join(" | ");
  if (node.properties) return "object";
  if (node.items) return "array";
  return "value";
}

function displayVariantLabels(node: SchemaNode) {
  const variants = node.oneOf ?? node.anyOf ?? [];
  return variants
    .filter((variant) => !schemaTypeValues(variant).includes("null"))
    .map((variant) =>
      variant.const !== undefined
        ? JSON.stringify(variant.const)
        : schemaTypeValues(variant)
            .filter((type) => type !== "null")
            .join(" | "),
    )
    .filter(Boolean);
}

function schemaTypeValues(node: SchemaNode) {
  if (Array.isArray(node.type)) return node.type;
  return node.type ? [node.type] : [];
}

function placeholderForSchema(node: SchemaNode) {
  if (node.const !== undefined) return JSON.stringify(node.const);
  if (node.enum?.length) return "";
  const types = schemaTypeValues(node);
  if (types.includes("object") || node.properties) return "{}";
  if (types.includes("array")) return "[]";
  return "";
}

function pushComment(lines: string[], text: string, indent: number) {
  for (const line of text.split("\n")) {
    lines.push(`${spaces(indent)}# ${line.trim()}`);
  }
}

function cleanDescription(value: unknown) {
  return typeof value === "string" ? value.trim() : "";
}

function spaces(count: number) {
  return " ".repeat(count);
}

function asSchemaNode(value: unknown): SchemaNode | null {
  return value && typeof value === "object" ? (value as SchemaNode) : null;
}
