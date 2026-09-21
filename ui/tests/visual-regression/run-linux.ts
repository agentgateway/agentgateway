import { type SpawnSyncOptions, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const args = process.argv.slice(2);
const uiDir = join(import.meta.dirname, '../..');
const repoDir = join(uiDir, '..');
const pnpmStorePath = run('pnpm', ['store', 'path'], {
	cwd: uiDir,
	encoding: 'utf8',
	stdio: 'pipe'
});

run(process.env.DOCKER_BUILDER || 'docker', [
	'run',
	'--rm',
	'--ipc=host',
	'--volume',
	`${repoDir}:/work:rw`,
	'--volume',
	`agentgateway-ui-visual-node-modules-${process.arch}:/work/ui/node_modules`,
	'--volume',
	`${pnpmStorePath}:/pnpm/store/v11`,
	'--volume',
	'agentgateway-ui-visual-npm-cache:/npm-cache',
	'--env',
	'NPM_CONFIG_CACHE=/npm-cache',
	'--workdir',
	'/work/ui',
	'mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e',
	'npm',
	'exec',
	'--yes',
	`--package=node@${readFileSync(join(uiDir, '.nvmrc'), 'utf8').trim()}`,
	'--package=pnpm@11.20.0',
	'--',
	'sh',
	'-c',
	'pnpm install --frozen-lockfile --store-dir /pnpm/store && pnpm test:visual-regressions "$@"',
	'visual-regressions',
	...args
]);

function run(command: string, commandArgs: string[], options: SpawnSyncOptions = {}) {
	const result = spawnSync(command, commandArgs, { stdio: 'inherit', ...options });
	if (result.error) {
		console.error(`${command} is required to run visual regressions on ${process.platform}.`);
		process.exit(1);
	}
	if (result.status !== 0) {
		if (result.stderr) process.stderr.write(result.stderr);
		process.exit(result.status ?? 1);
	}
	return typeof result.stdout === 'string' ? result.stdout.trim() : '';
}
