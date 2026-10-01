import { expect, type Page, test } from '@playwright/test';

import { emptyConfig, mockGateway } from './fixtures';

async function mockHybridCosts(page: Page) {
	await mockGateway(page, emptyConfig());
	await page.route('**/api/runtime', route =>
		route.fulfill({
			json: { ui: { gatewayMode: 'standalone', configStoreMode: 'hybrid' } }
		})
	);
	await page.route('**/api/config/resources', route =>
		route.fulfill({ json: { resources: [], generation: 7 } })
	);
}

for (const message of ['config store changed since generation 7 (now 8)', 'rename collision']) {
	test(`config writes retain their generation after ${message}`, async ({ page }) => {
		await mockHybridCosts(page);
		const pins: Array<string | undefined> = [];
		await page.route('**/api/config/resources/modelCatalog', async route => {
			pins.push(route.request().headers()['if-match']);
			await route.fulfill({ status: 409, json: message });
		});
		await page.goto('/llm/costs');
		await expect(page.getByText('Configuration API unavailable')).toHaveCount(0);
		await page.getByRole('button', { name: 'Edit', exact: true }).click();
		await page.getByRole('button', { name: 'Save', exact: true }).click();
		await expect(page.getByText(message, { exact: true })).toBeVisible();
		await page.getByRole('button', { name: 'Save', exact: true }).click();
		await expect.poll(() => pins).toEqual(['7', '7']);
	});
}
