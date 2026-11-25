import assert from 'node:assert/strict';
import { test } from 'node:test';

import { contentKeys } from '#contentKeys.ts';

test('keys values by content and numbers repeats', () => {
	assert.deepEqual(contentKeys(['a', 'b', 'a']), ['0:a', '0:b', '1:a']);
	assert.deepEqual(contentKeys([]), []);
});

test('keeps keys unique when a value looks like a numbered key', () => {
	const keys = contentKeys(['a', 'a', '1:a', '0:a']);
	assert.equal(new Set(keys).size, keys.length);
});

test('keeps an item key when another item is inserted before it', () => {
	assert.deepEqual(contentKeys(['x', 'b']).slice(1), contentKeys(['b']));
});
