import { test } from '@playwright/test';
import { verifyLogPageContents } from './utils';

const TRAFFIC_LOGS_RESOURCE_PATH = '/ui#/traffic-logs';
const TRAFFIC_LOGS_TITLE = 'Traffic Logs';

test.beforeEach(async ({ page }) => { 
    // navigate to Traffic Logs page
    await page.goto(TRAFFIC_LOGS_RESOURCE_PATH);
});

test('should verify Traffic Log page contents are visible', async ({ page }) => { 
    await verifyLogPageContents(page, TRAFFIC_LOGS_TITLE);
});