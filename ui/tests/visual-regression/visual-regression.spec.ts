import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

import { mockGateway, mockVisualXdsGateway, unconfiguredConfig } from '../fixtures';
import { routes } from './pages';

for (const [name, path, heading, scenario = 'populated'] of routes) {
	test(`visual baseline: ${heading}`, async ({ page }, info) => {
		if (scenario === 'xds') await mockVisualXdsGateway(page);
		else await mockGateway(page, scenario === 'unconfigured' ? unconfiguredConfig() : undefined);
		await page.goto(path);
		const pageHeading = page.getByRole('heading', { name: heading, exact: true, level: 2 });
		await expect(pageHeading).toBeVisible();
		await expect(page.locator('.status-banner.loading')).toHaveCount(0);
		if (name === 'cel') {
			await expect(page.locator('.monaco-editor .view-lines')).toHaveCount(2);
		}
		if (name === 'raw-config') {
			await expect(page.locator('.monaco-editor .view-lines')).toContainText('config:');
		}
		if (name === 'llm-analytics') {
			await expect(page.getByText('$0.0042 / 340 tokens / 7 calls', { exact: true })).toBeVisible();
		}
		if (name === 'llm-logs') {
			await expect(page.locator('.log-call-preview')).toHaveText('Summarize the result.');
		}
		if (name === 'llm-client-setup') {
			await expect(page.getByRole('combobox', { name: 'Model' })).toContainText('openai/*');
		}
		if (name === 'mcp-playground') {
			await expect(page.getByText('No MCP servers', { exact: true })).toHaveCount(0);
		}
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
			const output = process.env.VISUAL_ACTUAL_DIR;
			if (output) {
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
			}
		}
	});
}
