import assert from 'node:assert/strict';
import { test } from 'node:test';

import { fitChipCount } from '#components/multiCheckFit.ts';

const GAP = 4;
const COUNT = 30;

test('shows every chip when they all fit, without reserving room for the count', () => {
	// 50 + 4 + 50 fills the field exactly; a count would not fit, but none is needed.
	assert.equal(fitChipCount([50, 50], COUNT, 104, GAP), 2);
});

test('reserves room for the count once something overflows', () => {
	// Three chips need 158px. Two would fit bare in 120px, but not alongside "+1".
	assert.equal(fitChipCount([50, 50, 50], COUNT, 120, GAP), 1);
	assert.equal(fitChipCount([50, 50, 50], COUNT, 138, GAP), 2);
});

test('keeps the first chip when a long value is wider than the field', () => {
	assert.equal(fitChipCount([400, 50], COUNT, 120, GAP), 1);
});

test('stops at the first chip that does not fit', () => {
	// A short third chip must not be shown ahead of the longer second one.
	assert.equal(fitChipCount([50, 300, 10], COUNT, 120, GAP), 1);
});

test('handles an empty selection', () => {
	assert.equal(fitChipCount([], COUNT, 120, GAP), 0);
});
