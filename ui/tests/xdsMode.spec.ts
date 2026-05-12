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
const CANNOT_EDIT_READONLY_MODE = "Cannot edit in read-only editor";

async function verifyXdsAwareButton(dataTestId: string, page: Page) { 
    // verify button is disabled
    const button = page.getByTestId(dataTestId);
    await expect(button).toBeVisible();
    await expect(button).toBeDisabled();

    // verify tooltip is visible on hover
    await button.hover({ force: true });
    await expect(button).toHaveClass(/ant-tooltip-open/);
    const tooltipId = await button.getAttribute('aria-describedby');
    const tooltip = page.locator(`#${tooltipId}`);
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toContainText(CONFIGURATION_MANAGED_BY_XDS);

    // move cursor off of tooltip to reset
    await page.mouse.move(0, 0);
    await expect(button).not.toHaveClass(/ant-tooltip-open/);
}

async function verifyReadonlyMonacoEditor(page: Page) { 
    // locate monaco editor on page, type text to pop read-only tooltip
    const monacoEditor = page.getByText('binds');
    await monacoEditor.click();
    await monacoEditor.pressSequentially('abc123', { delay: 100 });
    const readonlyTooltip = page.locator('#root').getByText(CANNOT_EDIT_READONLY_MODE);

    await expect(readonlyTooltip).toBeVisible();
    await expect(readonlyTooltip).toContainText(CANNOT_EDIT_READONLY_MODE);
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
    // navigate to LLM Configuration Editor page
    await page.goto('/ui#/llm-configuration/editor');

    // verify buttons are disabled
    await verifyXdsAwareButton('config-editor-format-button', page);
    await verifyXdsAwareButton('config-editor-cancel-button', page);
    await verifyXdsAwareButton('config-editor-save-button', page);

    // verify text editor is in read-only mode
    await verifyReadonlyMonacoEditor(page);
});

test('MCP Configuration should be in read-only mode', async ({ page }) => { 
    // navigate to MCP Configuration page
    await page.goto('/ui#/mcp-configuration');

    // verify add buttons are disabled with tooltip
    await verifyXdsAwareButton('hierarchy-tree-no-resources-add-button', page);
    await verifyXdsAwareButton('mcp-config-no-resources-add-button', page);
});

test('MCP Editor should be in read-only mode', async ({ page }) => { 
    // navigate to MCP Configuration Editor page
    await page.goto('/ui#/mcp-configuration/editor');

    // verify buttons are disabled
    await verifyXdsAwareButton('config-editor-format-button', page);
    await verifyXdsAwareButton('config-editor-cancel-button', page);
    await verifyXdsAwareButton('config-editor-save-button', page);

    // verify text editor is in read-only mode
    await verifyReadonlyMonacoEditor(page);
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
    // navigate to Traffic Configuration Editor page
    await page.goto('/ui#/traffic-configuration/editor');

    // verify buttons are disabled
    await verifyXdsAwareButton('config-editor-format-button', page);
    await verifyXdsAwareButton('config-editor-cancel-button', page);
    await verifyXdsAwareButton('config-editor-save-button', page);

    // verify text editor is in read-only mode
    await verifyReadonlyMonacoEditor(page);
});