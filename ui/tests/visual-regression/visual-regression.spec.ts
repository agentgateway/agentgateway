import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, type Locator, type Page, type TestInfo, test } from '@playwright/test';

import { json, mockGateway, unconfiguredConfig, xdsDump } from '../fixtures';
import { routes, testTitle } from './pages';

async function mockVisualXdsGateway(page: Page) {
	await mockGateway(page);
	await page.route('**/api/runtime', route =>
		json(route, {
			build: {
				version: 'test',
				gitRevision: 'test',
				rustVersion: 'test',
				buildProfile: 'test',
				buildTarget: 'test'
			},
			ui: { gatewayMode: 'xds', configStoreMode: 'readOnly' }
		})
	);
	await page.route('**/config_dump', route =>
		json(route, {
			...xdsDump([]),
			policies: [
				{
					key: 'default/api-timeout',
					name: { kind: 'TrafficPolicy', namespace: 'default', name: 'api-timeout' },
					target: {
						route: { namespace: 'default', name: 'api', kind: 'HTTPRoute' }
					},
					inheritance: 'Override',
					policy: {
						traffic: { phase: 'Route', timeout: { requestTimeout: '30s' } }
					}
				}
			]
		})
	);
}

const ready: Partial<Record<(typeof routes)[number][0], (page: Page) => Promise<void>>> = {
	cel: async page => {
		await expect(page.locator('.monaco-editor .view-lines')).toHaveCount(2);
	},
	'raw-config': async page => {
		await expect(page.locator('.monaco-editor .view-lines')).toContainText('config:');
	},
	'llm-analytics': async page => {
		await expect(page.getByText('$0.0042 / 340 tokens / 7 calls', { exact: true })).toBeVisible();
	},
	'llm-logs': async page => {
		await expect(page.locator('.log-call-preview')).toHaveText('Summarize the result.');
	},
	'llm-client-setup': async page => {
		await expect(page.getByRole('combobox', { name: 'Model' })).toContainText('openai/*');
	},
	'mcp-playground': async page => {
		await expect(page.getByText('No MCP servers', { exact: true })).toHaveCount(0);
	}
};

async function captureEvidence(page: Page, info: TestInfo, name: string, mask: Locator[]) {
	const output = process.env.VISUAL_ACTUAL_DIR;
	if (!output) return;
	try {
		const directory = join(output, info.project.name);
		mkdirSync(directory, { recursive: true });
		await page.screenshot({
			path: join(directory, `${name}-full-page.png`),
			fullPage: true,
			mask,
			animations: 'disabled',
			caret: 'hide',
			style: readFileSync(join(import.meta.dirname, 'visual-regression.css'), 'utf8')
		});
	} catch (error) {
		// Keep the assertion error. Without this image the report marks the capture incomplete.
		info.annotations.push({ type: 'evidence capture failed', description: String(error) });
	}
}

for (const [name, path, heading, scenario = 'populated'] of routes) {
	test(testTitle(heading), async ({ page }, info) => {
		if (scenario === 'xds') await mockVisualXdsGateway(page);
		else await mockGateway(page, scenario === 'unconfigured' ? unconfiguredConfig() : undefined);
		await page.goto(path);
		const pageHeading = page.getByRole('heading', { name: heading, exact: true, level: 2 });
		await expect(pageHeading).toBeVisible();
		await expect(page.locator('.status-banner.loading')).toHaveCount(0);
		await ready[name]?.(page);
		await page.evaluate(() => document.fonts.ready);
		await expect
			.poll(() =>
				page
					.locator('img')
					.evaluateAll(images =>
						images.every(
							image => image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0
						)
					)
			)
			.toBe(true);
		const mask = [
			page.locator('.log-td-time'),
			page.locator('.message-meta-chip').filter({ hasText: /^\d+(?:\.\d+)?(?:ms|s)$/ })
		];
		try {
			await expect(page).toHaveScreenshot(`${name}-full-page.png`, { fullPage: true, mask });
		} finally {
			await captureEvidence(page, info, name, mask);
		}
	});
}
