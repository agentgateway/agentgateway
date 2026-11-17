import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { copySources, isSourcePath, resolveRevision } from './compare.ts';

test('source selection excludes local configuration and generated files', () => {
	for (const path of [
		'ui/.env',
		'ui/.env.local',
		'ui/.npmrc',
		'ui/.npmrc.local',
		'ui/node_modules/package/index.js',
		'ui/dist/index.html',
		'ui/tests/visual-regression/results.local/run/current/ui/src/main.tsx',
		'../ui/src/main.tsx',
		'/ui/src/main.tsx'
	])
		assert.equal(isSourcePath(path), false, path);
	for (const path of ['ui/src/main.tsx', 'ui/package.json', 'schema/config.json']) {
		assert.equal(isSourcePath(path), true, path);
	}
});

test('candidate snapshot keeps current and new content while omitting deleted files', () => {
	const root = mkdtempSync(join(tmpdir(), 'visual-source-'));
	try {
		mkdirSync(join(root, 'source/ui'), { recursive: true });
		writeFileSync(join(root, 'source/ui/changed.ts'), 'current working tree');
		writeFileSync(join(root, 'source/ui/new.ts'), 'new source');
		const copied = copySources(join(root, 'source'), join(root, 'copy'), [
			'ui/changed.ts',
			'ui/new.ts',
			'ui/deleted.ts'
		]);
		assert.deepEqual(copied, ['ui/changed.ts', 'ui/new.ts']);
		assert.equal(readFileSync(join(root, 'copy/ui/changed.ts'), 'utf8'), 'current working tree');
		assert.equal(readFileSync(join(root, 'copy/ui/new.ts'), 'utf8'), 'new source');
	} finally {
		rmSync(root, { recursive: true, force: true });
	}
});

test('snapshot refuses source symlinks instead of reading outside its source tree', () => {
	const root = mkdtempSync(join(tmpdir(), 'visual-symlink-'));
	try {
		mkdirSync(join(root, 'source/ui'), { recursive: true });
		writeFileSync(join(root, 'outside'), 'outside source');
		symlinkSync(join(root, 'outside'), join(root, 'source/ui/link'));
		assert.throws(
			() => copySources(join(root, 'source'), join(root, 'copy'), ['ui/link']),
			/symlink/
		);
	} finally {
		rmSync(root, { recursive: true, force: true });
	}
});

test('reference resolution rejects invalid and option-shaped revisions', () => {
	const root = join(import.meta.dirname, '../../..');
	assert.match(resolveRevision(root, 'HEAD'), /^[0-9a-f]{40}$/);
	assert.throws(() => resolveRevision(root, 'visual-reference-does-not-exist'), /reference/);
	assert.throws(() => resolveRevision(root, '--help'), /reference/);
});

test('snapshot refuses paths through symlinked directories', () => {
	const root = mkdtempSync(join(tmpdir(), 'visual-linked-directory-'));
	try {
		mkdirSync(join(root, 'source/ui'), { recursive: true });
		mkdirSync(join(root, 'outside'));
		writeFileSync(join(root, 'outside/file.ts'), 'outside source');
		symlinkSync(join(root, 'outside'), join(root, 'source/ui/linked'));
		assert.throws(
			() => copySources(join(root, 'source'), join(root, 'copy'), ['ui/linked/file.ts']),
			/outside/
		);
	} finally {
		rmSync(root, { recursive: true, force: true });
	}
});
