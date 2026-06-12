import { test } from '@playwright/test';
import { verifyLogPageContents } from './utils';

const MCP_LOGS_RESOURCE_PATH = '/ui#/mcp-logs';
const MCP_LOGS_TITLE = 'MCP Logs';

test.beforeEach(async ({ page }) => { 
    // navigate to LLM Logs page
    await page.goto(MCP_LOGS_RESOURCE_PATH);
});

test('should verify MCP Log page contents are visible', async ({ page }) => { 
    await verifyLogPageContents(page, MCP_LOGS_TITLE);
});