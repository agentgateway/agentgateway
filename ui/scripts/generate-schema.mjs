#!/usr/bin/env node
// Generates TypeScript types from JSON schemas and prepares public assets for the UI.
// Run via: yarn generate-schema (from ui/) or: node ui/scripts/generate-schema.mjs (from repo root)
import { execSync } from "child_process";
import { copyFileSync, readFileSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = join(uiDir, "..");

// Generate TypeScript type declarations from JSON schemas (in parallel).
// src/cel.d.ts is used for build-time type checking.
// src/config.d.ts is used for build-time type checking.
execSync(
  "yarn json2ts ../schema/cel.json > src/cel.d.ts & " +
    "yarn json2ts ../schema/config.json > src/config.d.ts & wait",
  { cwd: uiDir, shell: true, stdio: "inherit" },
);

// Copy JSON schemas to public/ for runtime use by the Monaco YAML editor.
copyFileSync(
  join(rootDir, "schema/config.json"),
  join(uiDir, "public/config-schema.json"),
);
copyFileSync(
  join(rootDir, "schema/cel.json"),
  join(uiDir, "public/cel-schema.json"),
);

// Copy CEL type declarations to public/ for runtime injection into Monaco's JS language service.
// Then append a `declare const` for each ExecutorSerde property so Monaco can offer type hints
// for the bare variable names (e.g. `request`, `jwt`) used in CEL expressions.
copyFileSync(join(uiDir, "src/cel.d.ts"), join(uiDir, "public/cel.d.ts"));

const celSchema = JSON.parse(
  readFileSync(join(rootDir, "schema/cel.json"), "utf8"),
);
const decls = Object.entries(celSchema.properties)
  .map(([k, v]) => {
    const doc = v.description ? `/** ${v.description} */\n` : "";
    return `${doc}declare const ${k}: ExecutorSerde['${k}'];`;
  })
  .join("\n");

const celDts = readFileSync(join(uiDir, "public/cel.d.ts"), "utf8");
const header =
  "// The following declarations are appended post-generation.\n" +
  "// Each ExecutorSerde property is declared as a global const so that Monaco's\n" +
  "// JS language service can provide type hints for the bare variable names\n" +
  "// (e.g. `request`, `jwt`) used in CEL expressions.\n";
writeFileSync(
  join(uiDir, "public/cel.d.ts"),
  `${celDts}\n${header}\n${decls}\n`,
);
