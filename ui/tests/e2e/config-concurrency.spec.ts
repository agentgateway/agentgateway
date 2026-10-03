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

for (const refreshSucceeds of [false, true]) {
	test(`cost refresh retains its generation when ${refreshSucceeds ? 'refetch' : 'refresh'} fails`, async ({
		page
	}) => {
		await mockHybridCosts(page);
		await page.route('**/api/costs/refresh-base', async route => {
			if (refreshSucceeds) {
				await page.route('**/api/config/resources', resourceRoute =>
					resourceRoute.fulfill({ status: 500, json: 'refetch failed' })
				);
			}
			await route.fulfill({
				status: refreshSucceeds ? 200 : 500,
				json: refreshSucceeds ? { providers: 1, models: 1 } : 'refresh failed'
			});
		});
		const pins: Array<string | undefined> = [];
		await page.route('**/api/config/resources/modelCatalog', async route => {
			pins.push(route.request().headers()['if-match']);
			await route.fulfill({ status: 409, json: 'stale generation' });
		});
		await page.goto('/llm/costs');
		await page.getByRole('button', { name: 'Refresh base costs' }).click();
		await expect(
			page.getByText(refreshSucceeds ? 'refetch failed' : 'refresh failed', { exact: true })
		).toBeVisible();
		await page.getByRole('button', { name: 'Edit', exact: true }).click();
		await page.getByRole('button', { name: 'Save', exact: true }).click();
		await expect.poll(() => pins).toEqual(['7']);
	});
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
