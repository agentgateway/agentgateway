#!/usr/bin/env node
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { compileFromFile } from "json-schema-to-typescript";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = join(uiDir, "..");

mkdirSync(join(uiDir, "src"), { recursive: true });
mkdirSync(join(uiDir, "public"), { recursive: true });

const sanitizedConfigSchemaPath = join(uiDir, ".config-schema-for-types.json");
const configSchema = JSON.parse(
  readFileSync(join(rootDir, "schema/config.json"), "utf8"),
);
writeFileSync(
  sanitizedConfigSchemaPath,
  JSON.stringify(sanitizeSchemaForTypes(configSchema)),
);
const sanitizedAdminSchemaPath = join(uiDir, ".admin-schema-for-types.json");
const adminSchema = JSON.parse(
  readFileSync(join(rootDir, "schema/admin.json"), "utf8"),
);
writeFileSync(
  sanitizedAdminSchemaPath,
  JSON.stringify(sanitizeSchemaForTypes(adminSchema)),
);

try {
  await Promise.all([
    writeTypes(join(rootDir, "schema/cel.json"), join(uiDir, "src/cel.d.ts")),
    writeTypes(
      sanitizedConfigSchemaPath,
      join(uiDir, "src/gateway-config.d.ts"),
    ),
    writeTypes(sanitizedAdminSchemaPath, join(uiDir, "src/gateway-admin.d.ts")),
  ]);
} finally {
  rmSync(sanitizedConfigSchemaPath, { force: true });
  rmSync(sanitizedAdminSchemaPath, { force: true });
}

copyFileSync(
  join(rootDir, "schema/config.json"),
  join(uiDir, "public/config-schema.json"),
);
copyFileSync(
  join(rootDir, "schema/admin.json"),
  join(uiDir, "public/admin-schema.json"),
);
copyFileSync(
  join(rootDir, "schema/cel.json"),
  join(uiDir, "public/cel-schema.json"),
);

function sanitizeSchemaForTypes(value) {
  if (Array.isArray(value)) {
    return value
      .filter((item) => !isImpossibleSchema(item))
      .map(sanitizeSchemaForTypes);
  }
  if (!value || typeof value !== "object") return value;

  const next = {};
  for (const [key, child] of Object.entries(value)) {
    const sanitized = sanitizeSchemaForTypes(child);
    if (
      (key === "oneOf" || key === "anyOf" || key === "allOf") &&
      Array.isArray(sanitized)
    ) {
      next[key] = sanitized.filter((item) => !isImpossibleSchema(item));
      if (next[key].length === 0) next.not = {};
      continue;
    }
    next[key] = sanitized;
  }
  return next;
}

async function writeTypes(schemaPath, outputPath) {
  writeFileSync(outputPath, await compileFromFile(schemaPath));
}

function isImpossibleSchema(value) {
  return Boolean(
    value &&
    typeof value === "object" &&
    Array.isArray(value.enum) &&
    value.enum.length === 0,
  );
}
