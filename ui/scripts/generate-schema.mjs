#!/usr/bin/env node
/**
 * Schema generation script — run as part of the build/dev setup.
 *
 * What this script does (in order):
 *   1. Converts JSON schemas → TypeScript .d.ts files used at build-time for type checking.
 *   2. Copies the raw JSON schemas to public/ so the Monaco YAML editor can load them at runtime
 *      to provide inline validation and autocomplete in the config editor.
 *   3. Copies the generated CEL type declarations to public/ and then patches the file with
 *      additional `declare const` statements so Monaco's JS language service knows about the
 *      global variables available inside CEL expressions (e.g. `request`, `jwt`).
 *
 * Run via: yarn --cwd=./ui generate-schema
 */
import { execSync } from "child_process";
import { copyFileSync, readFileSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

// Resolve absolute paths for the ui/ directory and the repo root.
// import.meta.url points to this script file, so we walk up one level to get ui/.
const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = join(uiDir, "..");

// ─── Step 1: Generate TypeScript type declarations from JSON schemas ──────────
//
// json2ts reads a JSON Schema and writes out a .d.ts file with equivalent TypeScript types.
// Both conversions run in parallel (shell background jobs via `&`) to save time.
//
//   schema/cel.json    → src/cel.d.ts    (types for CEL expression variables)
//   schema/config.json → src/config.d.ts (types for the gateway config structure)
//
// These .d.ts files are imported by the TypeScript source and checked at build time —
// they are NOT shipped to the browser directly.
execSync(
  "yarn json2ts ../schema/cel.json > src/cel.d.ts & " +
    "yarn json2ts ../schema/config.json > src/config.d.ts & wait",
  { cwd: uiDir, shell: true, stdio: "inherit" },
);

// ─── Step 2: Copy JSON schemas to public/ for Monaco runtime validation ───────
//
// The Monaco YAML editor loads these schema files at runtime (via fetch) to power
// inline validation and autocomplete in the config editor. They must live in public/
// so Vite serves them as static assets.
//
//   schema/config.json → public/config-schema.json
//   schema/cel.json    → public/cel-schema.json
copyFileSync(
  join(rootDir, "schema/config.json"),
  join(uiDir, "public/config-schema.json"),
);
copyFileSync(
  join(rootDir, "schema/cel.json"),
  join(uiDir, "public/cel-schema.json"),
);

// ─── Step 3: Patch cel.d.ts with global variable declarations for Monaco ─────
//
// Monaco's JS language service needs to know about the variables that are implicitly
// available inside a CEL expression (e.g. `request`, `jwt`). These variables come from
// the ExecutorSerde type, but CEL expressions reference them as bare globals — no object
// prefix — so we must tell Monaco they exist as top-level `const` declarations.
//
// Process:
//   a) Copy the freshly generated src/cel.d.ts to public/ (it will be injected into Monaco).
//   b) Read the CEL JSON schema to discover every property of ExecutorSerde.
//   c) For each property, generate a `declare const <name>: ExecutorSerde['<name>'];` line
//      (optionally preceded by a JSDoc comment from the schema's `description` field).
//   d) Append those declarations to public/cel.d.ts so Monaco sees them as globals.

// (a) Copy the base type declarations to public/ so they can be served to the browser.
copyFileSync(join(uiDir, "src/cel.d.ts"), join(uiDir, "public/cel.d.ts"));

// (b) Parse the CEL JSON schema to get the list of ExecutorSerde properties.
const celSchema = JSON.parse(
  readFileSync(join(rootDir, "schema/cel.json"), "utf8"),
);

// (c) Build one `declare const` line per property.
//     If the schema entry has a `description`, emit it as a JSDoc comment so Monaco
//     can show it in hover tooltips and autocomplete documentation.
const decls = Object.entries(celSchema.properties)
  .map(([k, v]) => {
    const doc = v.description ? `/** ${v.description} */\n` : "";
    return `${doc}declare const ${k}: ExecutorSerde['${k}'];`;
  })
  .join("\n");

// (d) Append the generated declarations to public/cel.d.ts.
//     The header comment explains why the extra declarations are there,
//     so anyone reading the generated file understands it was intentionally patched.
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
