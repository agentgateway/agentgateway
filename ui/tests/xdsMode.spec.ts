import { expect, test, type Page } from '@playwright/test';

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

const CONFIGURATION_MANAGED_BY_XDS = "Configuration is managed by xDS";

async function verifyXdsAwareButton(dataTestId: string, page: Page) { 
    // verify button is disabled
    const button = page.getByTestId(dataTestId);
    await expect(button).toBeVisible();
    await expect(button).toBeDisabled();

    // verify tooltip is visible on hover
    await button.hover({ force: true });
    await button.page().waitForTimeout(300); // small timeout to confirm tooltip pops in UI mode
    const tooltip = page.getByRole('tooltip');
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toContainText(CONFIGURATION_MANAGED_BY_XDS);

    // move cursor off of tooltip to reset
    await page.mouse.move(0, 0);
    await expect(tooltip).toBeHidden();
}

test('xDS banner should be visible app-wide across all pages', async ({ page }) => { 
    for (const route of AGENTGATEWAY_ROUTES) { 
        // navigate to route
        await page.goto(route);
    
        // verify xDS banner is visible
        const xdsBanner = page.getByTestId('xds-mode-banner');
    
        await expect(xdsBanner).toBeVisible();
        await expect(xdsBanner).toContainText(CONFIGURATION_MANAGED_BY_XDS);
        await expect(xdsBanner).toContainText('This agentgateway is receiving its configuration from https://localhost:18000. Edits are disabled.');
    }
});

test('LLM Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to LLM Configuration page
    await page.goto('/ui#/llm-configuration');

    // verify add buttons are disabled with tooltip
    await verifyXdsAwareButton('hierarchy-tree-no-resources-add-button', page);
    await verifyXdsAwareButton('llm-config-no-resources-add-button', page);
});

test('LLM Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});

test('MCP Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to MCP Configuration page
    await page.goto('/ui#/mcp-configuration');

    // verify add buttons are disabled with tooltip
    await verifyXdsAwareButton('hierarchy-tree-no-resources-add-button', page);
    await verifyXdsAwareButton('mcp-config-no-resources-add-button', page);
});

test('MCP Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});

test('Traffic Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to Traffic Configuration page
    await page.goto('/ui#/traffic-configuration');

    // ensure Add button is disabled with tooltip
    await verifyXdsAwareButton('hierarchy-tree-header-row-add-button', page);

    // open form for first port bind node
    const firstPortBindNode = page.getByText('Port 3000');
    await firstPortBindNode.click();

    // verify Edit button disabled
    await verifyXdsAwareButton('port-bind-edit-button', page);
});

test('Traffic Editor should be in read-only mode', async ({ page }) => { 
    // TODO
});