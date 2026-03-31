import { expect, test } from '@playwright/test';

const DASHBOARD_RESOURCE_PATH = '/ui#/dashboard'

test.beforeEach(async ({ page }) => {
    // navigate to dashboard page
    await page.goto(DASHBOARD_RESOURCE_PATH);
});

test(`should display banner, header, and subtext`, async ({ page }) => { 
    // validate Dashboard banner
    const banner = page.getByRole('banner');
    await expect(banner).toContainText('Dashboard');

    // validate Home + subtext header
    const header = page.getByRole('heading', { name: 'Home'});
    await expect(header).toHaveText('Home');

    const headerSubtext = page.getByText('AgentGateway configuration overview');
    await expect(headerSubtext).toHaveText('AgentGateway configuration overview');
});

test(`should validate status cards`, async ({ page }) => { 
    const validateCardElement = async (cardTitle: string, expectedPath: string) => { 
        // validate card is visible
        const card = page.locator('.ant-card-body')
            .filter({ hasText: cardTitle })
            .nth(0);
        await expect(card).toBeVisible();

        // validate card inner value is a number
        const cardValue = await card.locator('.ant-statistic-content-value').textContent();
        const isPortBindsValueNumber = !isNaN(Number(cardValue));
        expect(isPortBindsValueNumber).toBe(true);

        // validate tool tip on hover
        await card.hover();
        const toolTip = page.getByRole('tooltip', { name: `Go to ${cardTitle}` });
        await expect(toolTip).toBeVisible();

        // validate card link works
        await card.click();
        await expect(page).toHaveURL(new RegExp(`/ui#${expectedPath}`));
        await page.goBack();
    };

    const cards = [
        {title: 'Port Binds', path: '/traffic'},
        {title: 'Listeners', path: '/traffic'},
        {title: 'Routes', path: '/traffic'},
        {title: 'Named Backends', path: '/traffic'},
        {title: 'LLM Models', path: '/llm-configuration'},
        {title: 'MCP Targets', path: '/mcp-configuration'},
        {title: 'Issues', path: '/traffic-configuration'},
    ];

    for (const card of cards) { 
        await validateCardElement(card.title, card.path);
    }
});

test(`should validate section cards`, async ({ page }) => { 
    // validate Traffic section
    // header
    const trafficSection = page.getByTestId('traffic');
    await expect(trafficSection).toBeVisible();

    // status badge
    const statusBadge = trafficSection.locator('.ant-tag');
    await expect(statusBadge).toBeVisible();

    // subtext
    const trafficSectionSubtext = trafficSection.getByText('Manage port binds, listeners');
    await expect(trafficSectionSubtext).toBeVisible();

    // stats
    const binds = trafficSection.getByText('Binds', { exact: true});
    await expect(binds).toBeVisible();

    const listeners = trafficSection.getByText('Listeners', { exact: true});
    await expect(listeners).toBeVisible();

    const routes = trafficSection.getByText('Routes', { exact: true});
    await expect(routes).toBeVisible();

    // validate LLM section
    // header
    const llmSection = page.getByTestId('llm');
    await expect(llmSection).toBeVisible();

    // subtext
    const llmSectionSubtext = llmSection.getByText('Configure large language model providers and models');
    await expect(llmSectionSubtext).toBeVisible();

    // stats
    const models = llmSection.getByText('Models', { exact: true});
    await expect(models).toBeVisible();

    const policies = llmSection.getByText('Policies', { exact: true});
    await expect(policies).toBeVisible();

    // validate MCP section
    // header
    const mcpSection = page.getByTestId('mcp');
    await expect(mcpSection).toBeVisible();

    // subtext
    const mcpSectionSubtext = mcpSection.getByText('Model Context Protocol server targets and configuration');
    await expect(mcpSectionSubtext).toBeVisible();

    // stats
    const targets = mcpSection.getByText('Targets', { exact: true});
    await expect(targets).toBeVisible();    
});