import { expect, test } from '@playwright/test';

// TODO: update with merics/log routes when these are made available again
const AGENTGATEWAY_ROUTES = [
    '/ui#/dashboard',
    '/ui#/llm-configuration',
    '/ui#/llm-playground',
    '/ui#/llm-configuration/editor',
    '/ui#/mcp-configuration',
    '/ui#/mcp-playground',
    '/ui#/mcp-configuration/editor',
    '/ui#/traffic-configuration',
    '/ui#/traffic-configuration/editor',
    '/ui#/cel-playground',
];

test('xDS banner should be visible app-wide across all pages', async ({ page }) => { 

    for (const route of AGENTGATEWAY_ROUTES) { 
        // navigate to route
        await page.goto(route);
    
        // verify xDS banner is visible
        const xdsBanner = page.getByTestId('xds-mode-banner');
    
        await expect(xdsBanner).toBeVisible();
        await expect(xdsBanner).toContainText('Configuration is managed by xDS');
        await expect(xdsBanner).toContainText('This agentgateway is receiving its configuration from https://localhost:18000. Edits are disabled.');
    }
});

test('LLM Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to LLM Configuration page
    await page.goto('/ui#/llm-configuration');

    // verify add buttons are disabled with tooltip
    const hierarchyTreeAddButton = page.getByTestId('hierarchy-tree-no-resources-add-button');
    await expect(hierarchyTreeAddButton).toBeVisible();
    await expect(hierarchyTreeAddButton).toBeDisabled();

    const llmConfigAddButton = page.getByTestId('llm-config-no-resources-add-button');
    await expect(hierarchyTreeAddButton).toBeVisible();
    await expect(hierarchyTreeAddButton).toBeDisabled();

    await expect(llmConfigAddButton).toBeVisible();
    await expect(llmConfigAddButton).toBeDisabled();
});

test('LLM Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});

test('MCP Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to MCP Configuration page
    await page.goto('/ui#/mcp-configuration');

    // verify add buttons are disabled with tooltip
    const hierarchyTreeAddButton = page.getByTestId('hierarchy-tree-no-resources-add-button');
    await expect(hierarchyTreeAddButton).toBeVisible();
    await expect(hierarchyTreeAddButton).toBeDisabled();

    const llmConfigAddButton = page.getByTestId('mcp-config-no-resources-add-button');
    await expect(hierarchyTreeAddButton).toBeVisible();
    await expect(hierarchyTreeAddButton).toBeDisabled();

    await expect(llmConfigAddButton).toBeVisible();
    await expect(llmConfigAddButton).toBeDisabled();
});

test('MCP Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});

test('Traffic Configuration should be in read-only mode', async ({ page }) => { 
    // TODO
});

test('Traffic Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});