import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { runStep } from './compare.ts';

const output = '/visual/output';
const timings: Record<string, number> = {};
let status = 2;
const step = (name: string, command: string, args: string[], cwd: string, env = process.env) =>
	runStep(name, command, args, { cwd, env, output, timings });

try {
	for (const phase of ['reference', 'current']) {
		const cwd = `/visual/${phase}/ui`;
		if (
			step(
				`install_${phase}`,
				'pnpm',
				['install', '--frozen-lockfile', '--store-dir', '/visual-cache/pnpm'],
				cwd
			)
		) {
			throw new Error(`${phase} dependency installation failed. See install_${phase}.log.`);
		}
		if (step(`build_${phase}`, 'pnpm', ['build', '--mode', 'e2e'], cwd)) {
			throw new Error(`${phase} UI build failed. See build_${phase}.log.`);
		}
	}
	const sourcePath = join(output, 'source.json');
	const source = JSON.parse(readFileSync(sourcePath, 'utf8'));
	source.runtime = {
		node: process.version,
		architecture: process.arch,
		playwright: JSON.parse(
			readFileSync('/visual/current/ui/node_modules/@playwright/test/package.json', 'utf8')
		).version
	};
	writeFileSync(sourcePath, `${JSON.stringify(source, null, 2)}\n`);
	const reference = join(output, 'reference');
	mkdirSync(reference);
	for (const phase of ['reference', 'current']) {
		const env = {
			...process.env,
			VISUAL_REFERENCE_DIR: reference,
			VISUAL_UI_DIR: `/visual/${phase}/ui`,
			VISUAL_RESULTS_DIR: join(output, `${phase}-test-results`),
			VISUAL_JSON_REPORT: join(output, `${phase}-results.json`),
			VISUAL_ACTUAL_DIR: phase === 'current' ? join(output, 'current') : ''
		};
		const result = step(
			`capture_${phase}`,
			'pnpm',
			[
				'exec',
				'playwright',
				'test',
				'--config',
				'tests/playwright.visual-regressions.config.ts',
				`--max-failures=${phase === 'reference' ? '1' : '0'}`,
				`--update-snapshots=${phase === 'reference' ? 'all' : 'none'}`
			],
			'/visual/current/ui',
			env
		);
		if (phase === 'reference' && result)
			throw new Error(
				'Reference capture failed; no valid comparison can be made. See reference-results.json.'
			);
		if (phase === 'current') status = result ? 1 : 0;
	}
	const { createReport, renderFrames } = await import('./report.ts');
	let start = performance.now();
	const complete = createReport(output);
	if (!complete) status = 2;
	timings.report = (performance.now() - start) / 1000;
	const { media } = JSON.parse(readFileSync(join(output, 'source.json'), 'utf8'));
	if (complete && (media.video || media.gif)) {
		start = performance.now();
		await renderFrames(output);
		timings.media_frames = (performance.now() - start) / 1000;
	}
} catch (error) {
	console.error(error);
	writeFileSync(join(output, 'error.txt'), `${error}\n`);
	if (existsSync(join(output, 'current-results.json'))) {
		const { createReport } = await import('./report.ts');
		createReport(output);
	}
	status = 2;
} finally {
	writeFileSync(join(output, 'phase-timings.json'), `${JSON.stringify(timings, null, 2)}\n`);
	const metrics: Record<string, number | string> = {
		architecture: process.arch,
		node: process.version
	};
	for (const [file, key] of [
		['memory.peak', 'peak_memory_bytes'],
		['cpu.stat', 'cpu_seconds']
	] as const) {
		try {
			const value = readFileSync(`/sys/fs/cgroup/${file}`, 'utf8');
			metrics[key] =
				file === 'cpu.stat'
					? Number(value.match(/usage_usec (\d+)/)?.[1]) / 1_000_000
					: Number(value.trim());
		} catch {
			// Some container engines do not expose these cgroup counters.
		}
	}
	writeFileSync(join(output, 'resources.json'), `${JSON.stringify(metrics, null, 2)}\n`);
}
process.exitCode = status;
