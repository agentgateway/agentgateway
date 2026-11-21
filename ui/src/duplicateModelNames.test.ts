import assert from 'node:assert/strict';
import { test } from 'node:test';

import { duplicateModelNamesWarning, isDuplicateModelNamesWarning } from '#duplicateModelNames.ts';

test('tells the operator how to resolve duplicate model names', () => {
	assert.equal(
		duplicateModelNamesWarning(['*', 'gpt-*', '*']),
		'Duplicate model names: *, gpt-*. If entries with the same name match a request, the first one listed is used. Set a unique id on each entry to clear this warning.'
	);
});

test('recognises only the duplicate model names warning', () => {
	assert.equal(isDuplicateModelNamesWarning(duplicateModelNamesWarning(['*'])), true);
	assert.equal(isDuplicateModelNamesWarning('Duplicate model IDs: a.'), false);
	assert.equal(isDuplicateModelNamesWarning('Duplicate MCP server names: a.'), false);
});
