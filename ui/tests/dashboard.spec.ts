import { expect, test } from '@playwright/test';

const DASHBOARD_RESOURCE_PATH = '/ui#/dashboard'

test.beforeEach(async ({ page }) => {
    await page.goto(DASHBOARD_RESOURCE_PATH);
    await page.waitForLoadState('networkidle').catch(() => {});
});

test('should display Gateway Overview heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: /Gateway Overview/i }).first()).toBeVisible();
});

test('should display onboarding card or overview depending on config state', async ({ page }) => {
    const hasWelcome = await page.getByText('Welcome to Agentgateway').isVisible().catch(() => false);
    const hasOverview = await page.getByRole('heading', { name: /Gateway Overview/i }).isVisible().catch(() => false);
    expect(hasWelcome || hasOverview).toBe(true);
});

test('should show LLM surface row', async ({ page }) => {
    await expect(page.getByText('LLM').first()).toBeVisible();
});

test('should show MCP surface row', async ({ page }) => {
    await expect(page.getByText('MCP').first()).toBeVisible();
});

test('should show Traffic surface row', async ({ page }) => {
    await expect(page.getByText('Traffic').first()).toBeVisible();
});

test('should show onboarding surfaces when config is empty', async ({ page }) => {
    // If no surfaces enabled, the welcome card shows surface options to enable
    const hasWelcome = await page.getByText('Welcome to Agentgateway').isVisible().catch(() => false);
    if (hasWelcome) {
        await expect(page.getByText('LLM').first()).toBeVisible();
        await expect(page.getByText('MCP').first()).toBeVisible();
    }
});

test('should navigate to LLM Models from overview row action link', async ({ page }) => {
    const hasOverview = await page.getByRole('heading', { name: /Gateway Overview/i }).isVisible().catch(() => false);
    if (!hasOverview) test.skip();
    const modelsLink = page.getByRole('link', { name: /Models/i }).first();
    const hasLink = await modelsLink.isVisible().catch(() => false);
    if (hasLink) {
        await modelsLink.click();
        await expect(page).toHaveURL(/#\/llm-models/);
    }
});
