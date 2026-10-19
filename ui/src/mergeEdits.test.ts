import assert from 'node:assert/strict';
import { test } from 'node:test';

// biome-ignore lint/style/noRestrictedImports: node --test cannot resolve the @/ alias.
import { mergeEdits } from './mergeEdits.ts';

test('keeps fields the editor does not model', () => {
	const original = { host: 'a', cache: { ttl: '10s' }, policies: { backendTLS: {} } };
	assert.deepEqual(mergeEdits(original, { host: 'a' }, { host: 'b' }), {
		value: { host: 'b', cache: { ttl: '10s' }, policies: { backendTLS: {} } },
		lost: []
	});
});

test('removes fields the user cleared', () => {
	const original = { host: 'a', path: '/check', cache: {} };
	const baseline = { host: 'a', path: '/check' };
	assert.deepEqual(mergeEdits(original, baseline, { host: 'a' }).value, { host: 'a', cache: {} });
});

test('does not write defaults the editor fills in', () => {
	const original = { issuer: 'a' };
	const baseline = { issuer: 'a', mode: 'strict' };
	assert.deepEqual(mergeEdits(original, baseline, { issuer: 'b', mode: 'strict' }).value, {
		issuer: 'b'
	});
});

test('untouched values round-trip exactly, even when the editor reshapes them', () => {
	const original = { limits: [{ key: 'x' }, { key: 'y' }], tls: { spiffe: {} } };
	const baseline = { limits: [{}], tls: null };
	assert.deepEqual(mergeEdits(original, baseline, baseline), { value: original, lost: [] });
});

test('aligns list entries across removals, in-place edits, and additions', () => {
	const original = [
		{ name: 'a', scope: ['messages'] },
		{ name: 'b', scope: ['messages'] },
		{ name: 'c', scope: ['messages'] }
	];
	const baseline = [{ name: 'a' }, { name: 'b' }, { name: 'c' }];
	assert.deepEqual(mergeEdits(original, baseline, [{ name: 'b' }, { name: 'c2' }]), {
		value: [
			{ name: 'b', scope: ['messages'] },
			{ name: 'c2', scope: ['messages'] }
		],
		lost: []
	});
	assert.deepEqual(mergeEdits(original, baseline, [...baseline, { name: 'd' }]).value, [
		...original,
		{ name: 'd' }
	]);
});

test('pairs edits in order when entries are appended in the same save', () => {
	const original = [{ name: 'a', scope: ['messages'] }];
	assert.deepEqual(mergeEdits(original, [{ name: 'a' }], [{ name: 'a2' }, { name: 'b' }]), {
		value: [{ name: 'a2', scope: ['messages'] }, { name: 'b' }],
		lost: []
	});
});

test('does not mix up rows that differ only in hidden fields', () => {
	const original = [
		{ host: 'a', policies: { key: 'A' } },
		{ host: 'a', policies: { key: 'B' } }
	];
	const baseline = [{ host: 'a' }, { host: 'a' }];
	assert.deepEqual(mergeEdits(original, baseline, [{ host: 'b' }, { host: 'a' }]), {
		value: [
			{ host: 'b', policies: { key: 'A' } },
			{ host: 'a', policies: { key: 'B' } }
		],
		lost: []
	});
	assert.deepEqual(mergeEdits(original, baseline, [{ host: 'a' }]), {
		value: [{ host: 'a' }],
		lost: ['[0]', '[1]']
	});
});

test('does not move hidden fields between entries when pairing is ambiguous', () => {
	const original = [
		{ name: 'a', hidden: 'A' },
		{ name: 'b', hidden: 'B' },
		{ name: 'c', hidden: 'C' }
	];
	const baseline = [{ name: 'a' }, { name: 'b' }, { name: 'c' }];
	assert.deepEqual(mergeEdits(original, baseline, [{ name: 'b2' }, { name: 'c' }]), {
		value: [{ name: 'b2' }, { name: 'c', hidden: 'C' }],
		lost: ['[0]', '[1]']
	});
});

test('keeps hidden fields when every field has hidden subfields', () => {
	const original = {
		openAIModeration: { model: 'a', failureMode: 'failOpen' },
		rejection: { body: 'blocked', headers: { set: { 'x-blocked': 'true' } } }
	};
	const baseline = { openAIModeration: { model: 'a' }, rejection: { body: 'blocked' } };
	const edited = { openAIModeration: { model: 'b' }, rejection: { body: 'blocked' } };
	assert.deepEqual(mergeEdits(original, baseline, edited), {
		value: { ...original, openAIModeration: { model: 'b', failureMode: 'failOpen' } },
		lost: []
	});
});

test('keeps hidden fields of list entries under an object', () => {
	const original = { request: [{ webhook: { host: 'a' }, scope: ['messages'] }] };
	const baseline = { request: [{ webhook: { host: 'a' } }] };
	const edited = { request: [{ webhook: { host: 'b' } }] };
	assert.deepEqual(mergeEdits(original, baseline, edited), {
		value: { request: [{ webhook: { host: 'b' }, scope: ['messages'] }] },
		lost: []
	});
});

test('does not mix fields of an object the editor rewrote into another shape', () => {
	const original = { name: 'a', mcp: { backend: 'b', path: '/mcp' } };
	const baseline = { name: 'a', mcp: { host: 'http://b/mcp' } };
	const edited = { name: 'a', mcp: { host: 'http://c/mcp' } };
	assert.deepEqual(mergeEdits(original, baseline, edited), { value: edited, lost: ['mcp'] });
	const hostPortPath = { name: 'a', mcp: { host: 'b', port: 3001, path: '/mcp' } };
	assert.deepEqual(mergeEdits(hostPortPath, baseline, edited), { value: edited, lost: ['mcp'] });
});

test('replacing an empty original is not reported as lost', () => {
	const original = { request: [], response: [] };
	const edited = { request: [{ regex: {} }] };
	assert.deepEqual(mergeEdits(original, {}, edited), {
		value: { request: [{ regex: {} }], response: [] },
		lost: []
	});
});

test('reports content replaced where the editor reshaped the original', () => {
	const original = { port: 80, limits: [{ key: 'x' }, { key: 'y' }], tls: { spiffe: {} } };
	const baseline = { port: 80, limits: [{}], tls: null };
	const edited = { port: 80, limits: [{ burst: 1 }], tls: { cert: 'c' } };
	assert.deepEqual(mergeEdits(original, baseline, edited), {
		value: edited,
		lost: ['limits', 'tls']
	});
});

test('passes fields and list entries the editor adds to `added`', () => {
	const seen: string[] = [];
	const added = (value: unknown, at: string[]) => {
		seen.push(at.join('.'));
		return { ...(value as object), stripped: true };
	};
	const original = { targets: [{ model: 'a' }] };
	const baseline = { targets: [{ model: 'a', weight: 1 }] };
	const edited = {
		targets: [
			{ model: 'a', weight: 1 },
			{ model: 'b', weight: 1 }
		],
		extra: {}
	};
	assert.deepEqual(mergeEdits(original, baseline, edited, added).value, {
		targets: [{ model: 'a' }, { model: 'b', weight: 1, stripped: true }],
		extra: { stripped: true }
	});
	assert.deepEqual(seen, ['targets.1', 'extra']);
});
