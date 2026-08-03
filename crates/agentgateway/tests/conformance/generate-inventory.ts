import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

// Emits suite-inventory.json for a framework clone: its revision plus each
// suite's exact scenario set, enumerated with the framework's own selectors.
// Preflight requires this output to be byte-identical to the committed file;
// JSON.stringify with two-space indent is the canonical format.
const gatedPendingScenarios = ['json-schema-2020-12'];

async function main() {
  const [frameworkDir] = process.argv.slice(2);
  if (!frameworkDir) {
    throw new Error('usage: generate-inventory.ts <framework-dir>');
  }

  const framework = execFileSync('git', ['-C', frameworkDir, 'rev-parse', 'HEAD'], {
    encoding: 'utf8'
  }).trim();
  const modulePath = pathToFileURL(path.join(frameworkDir, 'src', 'scenarios', 'index.ts')).href;
  const { listActiveClientScenarios, listDraftClientScenarios, listPendingClientScenarios } =
    await import(modulePath);

  // Public lane names use protocol revisions instead of framework selector aliases.
  const pending = listPendingClientScenarios().sort();
  for (const scenario of gatedPendingScenarios) {
    if (!pending.includes(scenario)) {
      throw new Error(`gated pending scenario is absent from the pending suite: ${scenario}`);
    }
  }
  const suites = {
    '2025-11-25': listActiveClientScenarios().sort(),
    '2026-07-28': listDraftClientScenarios().sort(),
    pending
  };
  process.stdout.write(`${JSON.stringify({ framework, suites, gatedPendingScenarios }, null, 2)}\n`);
}

main().catch((error) => {
  throw error;
});
