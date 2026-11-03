import assert from 'node:assert/strict';
import { test } from 'node:test';

import { apiBaseFrom, uiBaseFrom, withBasePath } from '#uiBase.ts';

test('absolute builds keep their configured base', () => {
	assert.equal(uiBaseFrom('/ui/', 'https://gw.example.com/proxy/ui/'), '/ui/');
	assert.equal(apiBaseFrom('/ui/', 'https://gw.example.com/proxy/ui/'), '');
	assert.equal(apiBaseFrom('/', undefined), '');
});

test('relative builds served at the root', () => {
	assert.equal(uiBaseFrom('./', 'http://localhost:15000/ui/'), '/ui/');
	assert.equal(apiBaseFrom('./', 'http://localhost:15000/ui/'), '');
});

test('builds served under a base path', () => {
	const href = 'https://tools.example.com/team/agentgateway/ui/';
	assert.equal(uiBaseFrom('./', href), '/team/agentgateway/ui/');
	assert.equal(apiBaseFrom('./', href), '/team/agentgateway');
});

test('relative builds without a base element fall back to the root layout', () => {
	assert.equal(uiBaseFrom('./', undefined), '/ui/');
	assert.equal(apiBaseFrom('./', undefined), '');
});

test('login returnTo gets the base path once', () => {
	assert.equal(withBasePath('/ui/llm', ''), '/ui/llm');
	assert.equal(withBasePath('/ui/llm', '/team/agentgateway'), '/team/agentgateway/ui/llm');
	assert.equal(
		withBasePath('/team/agentgateway/ui/llm', '/team/agentgateway'),
		'/team/agentgateway/ui/llm'
	);
	assert.equal(withBasePath('/ui', 'http://localhost:15000'), '/ui');
});
