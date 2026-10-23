import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

test('rootful container output stays writable by the nonroot caller', {
	skip: process.platform !== 'linux' || process.getuid?.() === 0
}, () => {
	const root = mkdtempSync(join(tmpdir(), 'visual-ownership-'));
	const run = join(root, 'run');
	const cache = join(root, 'cache');
	const owner = `${process.getuid?.()}:${process.getgid?.()}`;
	const image =
		'mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e';
	mkdirSync(run);
	mkdirSync(cache);
	const container = (...args: string[]) =>
		spawnSync(
			process.env.DOCKER_BUILDER || 'docker',
			[
				'run',
				'--rm',
				'--pull=never',
				'--security-opt',
				'label=disable',
				'--volume',
				`${run}:/visual:rw`,
				'--volume',
				`${cache}:/visual-cache:rw`,
				'--volume',
				`${import.meta.dirname}:/harness:ro`,
				...args
			],
			{ encoding: 'utf8', timeout: 120_000 }
		);
	let rootful = false;
	try {
		const legacy = container(
			image,
			'node',
			'-e',
			"const f=require('fs'); f.mkdirSync('/visual-cache/npm'); f.writeFileSync('/visual-cache/npm/legacy','old cache',{mode:0o600});"
		);
		rootful = statSync(join(cache, 'npm/legacy'), { throwIfNoEntry: false })?.uid === 0;
		assert.equal(legacy.status, 0, legacy.stderr);
		assert.equal(
			statSync(join(cache, 'npm/legacy')).uid,
			0,
			'Use a rootful container engine for this Linux ownership check.'
		);
		for (const status of [0, 1, 2]) {
			for (const side of ['reference', 'current'])
				mkdirSync(join(run, side, 'ui'), { recursive: true });
			const result = container(
				'--volume',
				'/visual/reference/ui/node_modules',
				'--volume',
				'/visual/current/ui/node_modules',
				image,
				'sh',
				'/harness/run-as-owner.sh',
				'node',
				'-e',
				"const f=require('fs'); for(const side of ['reference','current']){ f.mkdirSync('/visual/'+side+'/ui/dist'); f.writeFileSync('/visual/'+side+'/ui/dist/index.html','build'); f.writeFileSync('/visual/'+side+'/ui/node_modules/probe','dependency'); } f.writeFileSync('/visual-cache/npm/legacy','new cache'); f.mkdirSync('/visual/output',{recursive:true}); f.writeFileSync('/visual/output/timings.json','{}'); process.exitCode=Number(process.argv[1]);",
				String(status)
			);
			assert.equal(result.status, status, result.stderr);
			for (const file of [join(run, 'output/timings.json'), join(cache, 'npm/legacy')]) {
				assert.equal(`${statSync(file).uid}:${statSync(file).gid}`, owner);
				writeFileSync(file, 'host can write');
			}
			for (const side of ['reference', 'current']) rmSync(join(run, side), { recursive: true });
		}
	} finally {
		if (rootful) {
			const cleanup = container(
				image,
				'chown',
				'-R',
				'--no-dereference',
				owner,
				'/visual',
				'/visual-cache'
			);
			assert.equal(cleanup.status, 0, cleanup.stderr);
		}
		rmSync(root, { recursive: true, force: true });
	}
});
