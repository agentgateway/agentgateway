import assert from 'node:assert/strict';
import { test } from 'node:test';

import { modelDetail, modelMatchHeaders, modelOptionValue, modelSubtitle } from '#modelLabel.ts';
import type { LlmModel } from '#types.ts';

const vertex = (region: string, team?: string): LlmModel => ({
	name: '*',
	provider: 'vertex',
	params: { vertexProject: 'infra', vertexRegion: region },
	matches: team ? [{ headers: [{ name: 'x-team', value: { exact: team } }] }] : undefined
});

test('describes what sets a model apart', () => {
	assert.equal(modelDetail(vertex('us-east5', 'ml')), 'infra · us-east5 · x-team: ml');
	assert.equal(modelDetail({ name: 'gpt-*', provider: 'openAI' }), '');
	assert.equal(
		modelSubtitle({ ...vertex('us-east5', 'ml'), displayName: 'Gemini' }),
		'* · infra · us-east5 · x-team: ml'
	);
	assert.deepEqual(modelMatchHeaders(vertex('us-east5', 'ml')), { 'x-team': 'ml' });
});

test('gives models with the same name their own picker value', () => {
	const models: LlmModel[] = [
		vertex('us-east5'),
		{ name: 'gpt-*', provider: 'openAI' },
		vertex('europe-west4')
	];
	assert.deepEqual(
		models.map((_, index) => modelOptionValue(models, index)),
		['*', 'gpt-*', '*#infra · europe-west4']
	);
	assert.equal(modelOptionValue([models[0], { ...models[2], id: 'b' }], 1), '*#b');
	assert.equal(modelOptionValue([models[0], models[2], models[2]], 2), '*#2');
});
