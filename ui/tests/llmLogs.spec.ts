import { test } from '@playwright/test';
import { verifyLogPageContents } from './utils';

const LLM_LOGS_RESOURCE_PATH = '/ui#/llm-logs';
const LLM_LOGS_TITLE = 'LLM Logs';

test.beforeEach(async ({ page }) => { 
    // navigate to LLM Logs page
    await page.goto(LLM_LOGS_RESOURCE_PATH);
});

test('should verify LLM Log page contents are visible', async ({ page }) => { 
    await verifyLogPageContents(page, LLM_LOGS_TITLE);
});