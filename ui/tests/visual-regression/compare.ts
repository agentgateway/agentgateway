import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
	closeSync,
	copyFileSync,
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	openSync,
	readFileSync,
	realpathSync,
	rmSync,
	writeFileSync
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

export function isSourcePath(path: string) {
	const parts = path.split('/');
	return (
		['ui', 'schema'].includes(parts[0]) &&
		!parts.some(
			part =>
				part === '..' ||
				part === '.' ||
				part === '' ||
				part.startsWith('.env') ||
				part.startsWith('.npmrc') ||
				part.endsWith('.local') ||
				['node_modules', 'dist', 'test-results', 'playwright-report'].includes(part)
		)
	);
}

export function copySources(source: string, destination: string, paths: string[]) {
	const copied: string[] = [];
	for (const path of [...new Set(paths)].filter(isSourcePath).sort()) {
		const input = join(source, path);
		const stat = lstatSync(input, { throwIfNoEntry: false });
		if (!stat) continue;
		if (stat.isSymbolicLink()) throw new Error(`Source symlink is unsupported: ${path}`);
		if (!realpathSync(input).startsWith(`${realpathSync(source)}/${path.split('/')[0]}/`)) {
			throw new Error(`Source resolves outside its UI/schema tree: ${path}`);
		}
		if (!stat.isFile()) throw new Error(`Expected a source file: ${path}`);
		const output = join(destination, path);
		mkdirSync(dirname(output), { recursive: true });
		copyFileSync(input, output);
		copied.push(path);
	}
	return copied;
}

function git(root: string, args: string[]) {
	return execFileSync('git', args, {
		cwd: root,
		encoding: 'utf8',
		stdio: ['ignore', 'pipe', 'pipe'],
		maxBuffer: 16 * 1024 * 1024
	});
}

export function resolveRevision(root: string, revision: string) {
	try {
		return git(root, ['rev-parse', '--verify', '--end-of-options', `${revision}^{commit}`]).trim();
	} catch {
		throw new Error(`Cannot resolve reference ${JSON.stringify(revision)} to a local commit.`);
	}
}

export function runStep(
	name: string,
	command: string,
	args: string[],
	options: {
		cwd: string;
		output: string;
		timings: Record<string, number>;
		env?: NodeJS.ProcessEnv;
	}
) {
	const log = join(options.output, `${name}.log`);
	const fd = openSync(log, 'w');
	const start = performance.now();
	try {
		const result = spawnSync(command, args, {
			cwd: options.cwd,
			env: options.env,
			stdio: ['ignore', fd, fd]
		});
		if (result.error) throw result.error;
		return result.status ?? 2;
	} finally {
		closeSync(fd);
		options.timings[name] = (performance.now() - start) / 1000;
		writeFileSync(
			join(options.output, 'timings.json'),
			`${JSON.stringify(options.timings, null, 2)}\n`
		);
		console.log(`${name}: ${options.timings[name].toFixed(1)}s (${log})`);
	}
}

async function main() {
	const { values } = parseArgs({
		options: {
			base: { type: 'string', default: 'main' },
			video: { type: 'boolean' },
			gif: { type: 'boolean' },
			help: { type: 'boolean', short: 'h' }
		}
	});
	if (values.help) {
		console.log(
			'Usage: pnpm test:visual-regressions [--base main] [--video] [--gif]\nCompares a local Git revision with the working tree. Media exports require FFmpeg.'
		);
		return;
	}
	const root = resolve(import.meta.dirname, '../../..');
	const reference = resolveRevision(root, values.base);
	const engine = process.env.DOCKER_BUILDER || 'docker';
	const renderer =
		'mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e';
	for (const command of [engine, ...(values.video || values.gif ? ['ffmpeg'] : [])]) {
		const result = spawnSync(command, [command === 'ffmpeg' ? '-version' : '--version'], {
			stdio: 'ignore'
		});
		if (result.error || result.status !== 0)
			throw new Error(
				`${command} is required. ${command === 'ffmpeg' ? 'Omit --video/--gif to compare without media.' : 'Set DOCKER_BUILDER=podman to use Podman.'}`
			);
	}
	const workspace = join(import.meta.dirname, 'results.local');
	mkdirSync(workspace, { recursive: true });
	const run = mkdtempSync(join(workspace, 'run-'));
	const output = join(run, 'output');
	mkdirSync(output);
	const timings: Record<string, number> = {};
	const start = performance.now();
	let exitCode = 2;
	try {
		const snapshotStart = performance.now();
		const referenceFiles = git(root, [
			'ls-tree',
			'-r',
			'--name-only',
			'-z',
			reference,
			'--',
			'ui',
			'schema'
		])
			.split('\0')
			.filter(isSourcePath);
		for (const required of [
			'ui/package.json',
			'ui/.nvmrc',
			'schema/config.json',
			'schema/admin.json',
			'schema/cel.json'
		]) {
			if (!referenceFiles.includes(required))
				throw new Error(`Reference ${values.base} lacks required UI build input ${required}.`);
		}
		const archive = join(run, 'reference.tar');
		execFileSync('git', ['archive', '--output', archive, reference, '--', ...referenceFiles], {
			cwd: root
		});
		mkdirSync(join(run, 'reference'));
		execFileSync('tar', ['-xf', archive, '-C', join(run, 'reference')]);
		rmSync(archive);
		for (const path of referenceFiles) {
			if (lstatSync(join(run, 'reference', path)).isSymbolicLink())
				throw new Error(`Reference source symlink is unsupported: ${path}`);
		}
		const files = git(root, [
			'ls-files',
			'--cached',
			'--others',
			'--exclude-standard',
			'-z',
			'--',
			'ui',
			'schema'
		]).split('\0');
		const copied = copySources(root, join(run, 'current'), files);
		const hash = createHash('sha256');
		for (const path of copied)
			hash
				.update(path)
				.update('\0')
				.update(readFileSync(join(run, 'current', path)));
		writeFileSync(
			join(output, 'source.json'),
			`${JSON.stringify(
				{
					renderer,
					reference: { ref: values.base, commit: reference },
					candidate: {
						commit: resolveRevision(root, 'HEAD'),
						branch: git(root, ['branch', '--show-current']).trim(),
						dirty: Boolean(git(root, ['status', '--porcelain', '--', 'ui', 'schema']).trim()),
						sha256: hash.digest('hex')
					},
					media: { video: Boolean(values.video), gif: Boolean(values.gif) }
				},
				null,
				2
			)}\n`
		);
		timings.snapshot = (performance.now() - snapshotStart) / 1000;
		console.log(`Reference: ${values.base} (${reference})\nResults: ${output}`);
		const config = process.env.NPM_CONFIG_USERCONFIG;
		const configArgs = config
			? [
					'--volume',
					`${resolve(config)}:/visual-npmrc:ro`,
					'--env',
					'NPM_CONFIG_USERCONFIG=/visual-npmrc'
				]
			: [];
		const nodeVersion = readFileSync(join(run, 'current/ui/.nvmrc'), 'utf8').trim();
		const pkg = JSON.parse(readFileSync(join(run, 'current/ui/package.json'), 'utf8'));
		const state = `agentgateway-ui-visual-${createHash('sha256')
			.update(JSON.stringify([root, renderer, nodeVersion, pkg.packageManager]))
			.digest('hex')
			.slice(0, 16)}`;
		exitCode = runStep(
			'container',
			engine,
			[
				'run',
				'--rm',
				'--ipc=host',
				'--volume',
				`${run}:/visual:rw`,
				'--volume',
				`${state}-packages:/visual-cache`,
				'--volume',
				`${state}-reference:/visual/reference/ui/node_modules`,
				'--volume',
				`${state}-current:/visual/current/ui/node_modules`,
				...configArgs,
				'--env',
				'NPM_CONFIG_CACHE=/visual-cache/npm',
				'--env',
				'CI=1',
				'--workdir',
				'/visual/current/ui',
				renderer,
				'sh',
				'tests/visual-regression/run-as-owner.sh',
				'npm',
				'exec',
				'--yes',
				`--package=node@${nodeVersion}`,
				`--package=${pkg.packageManager}`,
				'--',
				'node',
				'tests/visual-regression/capture.ts'
			],
			{ cwd: root, output, timings }
		);
		const frames = join(output, 'frames.txt');
		if ((exitCode !== 0 && exitCode !== 1) || !existsSync(join(output, 'index.html'))) exitCode = 2;
		if (exitCode !== 2 && (values.video || values.gif) && !existsSync(frames))
			throw new Error('Requested media frames are missing. See container.log.');
		if (existsSync(frames)) {
			for (const format of ['video', 'gif'] as const) {
				if (!values[format]) continue;
				const encoding =
					format === 'video'
						? [
								'-c:v',
								'libx264',
								'-preset',
								'medium',
								'-crf',
								'20',
								'-pix_fmt',
								'yuv420p',
								'-r',
								'15',
								'-movflags',
								'+faststart'
							]
						: [
								'-filter_complex',
								'[0:v]fps=1,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:diff_mode=rectangle',
								'-loop',
								'0'
							];
				const result = runStep(
					`encode_${format}`,
					'ffmpeg',
					[
						'-hide_banner',
						'-loglevel',
						'error',
						'-y',
						'-f',
						'concat',
						'-safe',
						'1',
						'-i',
						frames,
						...encoding,
						join(output, `comparison.${format === 'video' ? 'mp4' : 'gif'}`)
					],
					{ cwd: output, output, timings }
				);
				if (result !== 0) exitCode = 2;
			}
		}
	} catch (error) {
		writeFileSync(join(output, 'error.txt'), `${error}\n`);
		throw error;
	} finally {
		timings.total = (performance.now() - start) / 1000;
		const phases = existsSync(join(output, 'phase-timings.json'))
			? JSON.parse(readFileSync(join(output, 'phase-timings.json'), 'utf8'))
			: {};
		writeFileSync(
			join(output, 'timings.json'),
			`${JSON.stringify({ ...phases, ...timings }, null, 2)}\n`
		);
		for (const directory of ['reference', 'current'])
			rmSync(join(run, directory), { recursive: true, force: true });
		console.log(
			`Total: ${timings.total.toFixed(1)}s\n${existsSync(join(output, 'index.html')) ? `Report: ${join(output, 'index.html')}` : `Diagnostics: ${output}`}\nTimings: ${join(output, 'timings.json')}`
		);
	}
	process.exitCode = exitCode;
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
	main().catch(error => {
		console.error(String(error));
		process.exitCode = 2;
	});
}
