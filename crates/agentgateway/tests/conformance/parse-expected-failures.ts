import { pathToFileURL } from 'node:url';
import path from 'node:path';

async function main() {
  const [frameworkDir, ...expectedFailuresPaths] = process.argv.slice(2);
  if (!frameworkDir || expectedFailuresPaths.length === 0) {
    throw new Error('usage: parse-expected-failures.ts <framework-dir> <expected-failures.yml>...');
  }

  // Keep expected-failure semantics owned by the pinned conformance framework.
  const modulePath = pathToFileURL(
    path.join(frameworkDir, 'src', 'expected-failures.ts')
  ).href;
  const { loadExpectedFailures } = await import(modulePath);
  const expectedFailures = await Promise.all(
    expectedFailuresPaths.map(async (expectedFailuresPath) => ({
      path: expectedFailuresPath,
      expectedFailures: await loadExpectedFailures(expectedFailuresPath)
    }))
  );
  process.stdout.write(`${JSON.stringify(expectedFailures)}\n`);
}

main().catch((error) => {
  throw error;
});
