import { expect, type Page, type Route, test } from '@playwright/test';

import { mockGateway } from './fixtures';

const mobileNavTrigger = (page: Page) => page.locator('.mobile-nav-trigger');
const mobileNavMenu = (page: Page) => page.locator('.mobile-nav-menu');

test.describe('mobile navigation', () => {
	test.use({ viewport: { width: 700, height: 900 } });

	test('closes on link navigation and browser back and forward', async ({ page }) => {
		await mockGateway(page);
		await page.goto('/llm/models');

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await mobileNavMenu(page).locator('a[href="/llm/providers"]').click();
		await expect(page).toHaveURL(/\/llm\/providers$/);
		await expect(mobileNavMenu(page)).toHaveCount(0);

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await page.goBack();
		await expect(page).toHaveURL(/\/llm\/models$/);
		await expect(mobileNavMenu(page)).toHaveCount(0);

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await page.goForward();
		await expect(page).toHaveURL(/\/llm\/providers$/);
		await expect(mobileNavMenu(page)).toHaveCount(0);
	});

	test('stays closed when history returns to the page it was opened on', async ({ page }) => {
		await mockGateway(page);
		await page.goto('/llm/models');
		await mobileNavTrigger(page).click();
		await mobileNavMenu(page).locator('a[href="/llm/providers"]').click();
		await expect(page).toHaveURL(/\/llm\/providers$/);

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await page.goBack();
		await expect(page).toHaveURL(/\/llm\/models$/);
		await expect(mobileNavMenu(page)).toHaveCount(0);
		await page.goForward();
		await expect(page).toHaveURL(/\/llm\/providers$/);
		await expect(mobileNavMenu(page)).toHaveCount(0);
	});

	test('reopens and dismisses on the current page', async ({ page }) => {
		await mockGateway(page);
		await page.goto('/llm/providers');

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toHaveCount(0);

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await page.keyboard.press('Escape');
		await expect(mobileNavMenu(page)).toHaveCount(0);

		await mobileNavTrigger(page).click();
		await expect(mobileNavMenu(page)).toBeVisible();
		await page.mouse.click(10, 880);
		await expect(mobileNavMenu(page)).toHaveCount(0);
	});
});

test.describe('policy editor scroll restore', () => {
	test.use({ viewport: { width: 1280, height: 540 } });

	test('restores list scroll and focus after closing the editor', async ({ page }) => {
		await mockGateway(page);
		await page.goto('/llm/policies');

		const tile = page.locator('button.policy-tile').last();
		await tile.scrollIntoViewIfNeeded();
		const before = await page.evaluate(() => window.scrollY);
		expect(before).toBeGreaterThan(0);

		await tile.click();
		await expect(page.locator('#drawer-title')).toBeVisible();
		await expect(page.evaluate(() => window.scrollY)).resolves.toBe(before);

		await page.keyboard.press('Escape');
		await expect(page.locator('#drawer-title')).toHaveCount(0);
		await expect(tile).toBeFocused();
		await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(before);
	});

	test('each close restores its own scroll position', async ({ page }) => {
		await mockGateway(page);
		await page.goto('/llm/policies');

		const lower = page.locator('button.policy-tile').last();
		await lower.scrollIntoViewIfNeeded();
		const lowerY = await page.evaluate(() => window.scrollY);
		await lower.click();
		await page.keyboard.press('Escape');
		await expect(page.locator('#drawer-title')).toHaveCount(0);
		await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(lowerY);

		await page.evaluate(() => window.scrollTo(0, 120));
		const upper = page.locator('button.policy-tile').nth(1);
		await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(120);
		await upper.click();
		await expect(page.locator('#drawer-title')).toBeVisible();
		await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(120);
		await page.keyboard.press('Escape');
		await expect(page.locator('#drawer-title')).toHaveCount(0);
		await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(120);
		await expect(upper).toBeFocused();
	});
});

test.describe('MCP playground arguments', () => {
	const schemaV1 = {
		type: 'object',
		properties: { text: { type: 'string', default: 'seed' } },
		required: ['text']
	};
	const schemaV2 = {
		type: 'object',
		properties: { text: { type: 'string', default: 'v2 seed' } },
		required: ['text']
	};

	test('resets on tool switch and schema change, keeps edits on unrelated renders', async ({
		page
	}) => {
		let schema: unknown = schemaV1;
		await mockGateway(page);
		await page.route('**/mcp', async (route: Route) => {
			const body = route.request().postDataJSON() as { id?: number; method?: string };
			const reply = (result: unknown) =>
				route.fulfill({
					status: 200,
					contentType: 'application/json',
					headers: { 'Mcp-Session-Id': 'session-1' },
					body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result })
				});
			if (body.method === 'initialize') return reply({ protocolVersion: '2025-03-26' });
			if (body.method === 'tools/list') {
				return reply({
					tools: [
						{ name: 'alpha', description: 'First tool', inputSchema: schema },
						{ name: 'beta', description: 'Second tool', inputSchema: structuredClone(schema) }
					]
				});
			}
			return reply({});
		});

		await page.goto('/mcp/playground');
		await page.getByRole('button', { name: 'Initialize', exact: true }).click();
		await expect(page.getByRole('combobox', { name: 'Tool' })).toContainText('alpha');
		await expect(page.getByLabel('text *')).toHaveValue('seed');

		await page.getByLabel('text *').fill('edited');
		await page.getByRole('combobox', { name: 'Tool' }).click();
		await page.getByRole('option', { name: /^beta/ }).click();
		await expect(page.getByRole('combobox', { name: 'Tool' })).toContainText('beta');
		await expect(page.getByLabel('text *')).toHaveValue('seed');

		await page.getByLabel('text *').fill('edited');
		await page.getByText('Authorization header').click();
		await page.getByRole('textbox', { name: 'Bearer token' }).fill('mcp-secret');
		await expect(page.getByLabel('text *')).toHaveValue('edited');

		// The UI only refetches tools after Reset, which also clears the selected tool.
		schema = schemaV2;
		await page.getByRole('button', { name: 'Reset', exact: true }).click();
		await page.getByRole('button', { name: 'Initialize', exact: true }).click();
		await expect(page.getByRole('combobox', { name: 'Tool' })).toContainText('alpha');
		await expect(page.getByLabel('text *')).toHaveValue('v2 seed');
	});
});
