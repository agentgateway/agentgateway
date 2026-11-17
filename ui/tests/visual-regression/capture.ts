import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { runStep } from './compare.ts';
import type { ComparisonSource } from './report.ts';

type Phase = 'reference' | 'current';

const output = '/visual/output';
const timings: Record<string, number> = {};
const step = (name: string, command: string, args: string[], cwd: string, env = process.env) =>
	runStep(name, command, args, { cwd, env, output, timings });

function prepare(phase: Phase) {
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

function recordRuntime() {
	const sourcePath = join(output, 'source.json');
	const source: ComparisonSource = JSON.parse(readFileSync(sourcePath, 'utf8'));
	source.runtime = {
		node: process.version,
		architecture: process.arch,
		playwright: JSON.parse(
			readFileSync('/visual/current/ui/node_modules/@playwright/test/package.json', 'utf8')
		).version
	};
	writeFileSync(sourcePath, `${JSON.stringify(source, null, 2)}\n`);
	return source;
}

// The reference run writes the baseline screenshots that the current run compares against.
function capture(phase: Phase, reference: string) {
	return step(
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
		{
			...process.env,
			VISUAL_REFERENCE_DIR: reference,
			VISUAL_UI_DIR: `/visual/${phase}/ui`,
			VISUAL_RESULTS_DIR: join(output, `${phase}-test-results`),
			VISUAL_JSON_REPORT: join(output, `${phase}-results.json`),
			VISUAL_ACTUAL_DIR: phase === 'current' ? join(output, 'current') : ''
		}
	);
}

// Returns the exit code: 0 when every capture matched, 1 for stable differences, 2 for an incomplete report.
async function compare() {
	prepare('reference');
	prepare('current');
	const source = recordRuntime();
	const reference = join(output, 'reference');
	mkdirSync(reference);
	if (capture('reference', reference))
		throw new Error(
			'Reference capture failed; no valid comparison can be made. See reference-results.json.'
		);
	const mismatched = capture('current', reference) !== 0;
	const { createReport, renderFrames } = await import('./report.ts');
	let start = performance.now();
	const complete = createReport(output);
	timings.report = (performance.now() - start) / 1000;
	if (!complete) return 2;
	if (source.media.video || source.media.gif) {
		start = performance.now();
		await renderFrames(output);
		timings.media_frames = (performance.now() - start) / 1000;
	}
	return mismatched ? 1 : 0;
}

// Keep whatever report the finished captures allow, so a failed run still shows its evidence.
async function reportFailure(error: unknown) {
	console.error(error);
	writeFileSync(join(output, 'error.txt'), `${error}\n`);
	if (!existsSync(join(output, 'current-results.json'))) return;
	try {
		const { createReport } = await import('./report.ts');
		createReport(output);
	} catch (reportError) {
		console.error(reportError);
	}
}

function writeMetrics() {
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

try {
	process.exitCode = await compare();
} catch (error) {
	process.exitCode = 2;
	await reportFailure(error);
} finally {
	writeMetrics();
}
