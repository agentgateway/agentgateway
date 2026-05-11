import { expect, test } from '@playwright/test';

test('xDS banner should be visible app-wide across all pages', async ({ page }) => { 
    // TODO: update with merics/log routes when these are made available again
    const routes = [
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

    for (const route of routes) { 
        // navigate to route
        await page.goto(route);
    
        // verify xDS banner is visible
        const xdsBanner = page.getByTestId('xds-mode-banner');
    
        await expect(xdsBanner).toBeVisible();
        await expect(xdsBanner).toContainText('Configuration is managed by xDS');
        await expect(xdsBanner).toContainText('This agentgateway is receiving its configuration from https://localhost:18000. Edits are disabled.');
    }
});