import { expect, test } from '@playwright/test';
import * as fs from 'fs';
import * as yaml from 'js-yaml';

const TRAFFIC_HIERARCHY_RESOURCE_PATH = '/ui#/traffic-configuration';
const TRAFFIC_CONFIGURATION = 'Traffic Configuration';
const DEFAULT_FORM_TEXT = 'Choose a bind, listener, route, backend, or policy from the hierarchy tree on the left to view and edit its configuration.';
const HEADER_SUBTEXT = 'View and edit the full agentgateway configuration.';
const LISTENER_NEW_NAME = 'e2e-test';

const E2E_CONFIG_PATH = process.env.CI ? '../e2e-test-config.yaml' : 'tests/fixtures/e2e-config.yaml';
const initialE2EConfig = yaml.load(fs.readFileSync(E2E_CONFIG_PATH, 'utf8')) as any;

test.beforeEach(async ({page}) => { 
    // navigate to Traffic Hierarchy page
    await page.goto(TRAFFIC_HIERARCHY_RESOURCE_PATH);
});

test.afterEach(async () => { 
    // restore initial state of e2e-config.yaml file
    fs.writeFileSync(E2E_CONFIG_PATH, yaml.dump(initialE2EConfig));
});

test('should verify Traffic Hierarchy page contents are visible', async ({ page }) => { 
    // verify banner
    const banner = page.getByRole('banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(TRAFFIC_CONFIGURATION);

    // verify page heading
    const heading = page.locator('h1').filter({ hasText: TRAFFIC_CONFIGURATION });
    await expect(heading).toBeVisible();
    await expect(heading).toHaveText(TRAFFIC_CONFIGURATION);

    // verify subtext
    const subtext = page.getByText(HEADER_SUBTEXT);
    await expect(subtext).toBeVisible();
    await expect(subtext).toHaveText(HEADER_SUBTEXT);

    // verify traffic hierarchy form
    const trafficHierarchyForm = page.locator('.ant-card').filter({ hasText: TRAFFIC_CONFIGURATION });
    await expect(trafficHierarchyForm).toBeVisible();
    await expect(trafficHierarchyForm).toContainText(TRAFFIC_CONFIGURATION);
    
    const addButton = trafficHierarchyForm.getByRole('button', { name: 'Add' });
    await expect(addButton).toBeVisible();
    await expect(addButton).toHaveText('Add');

    // verify default form text
    const defaultFormText = page.getByText(DEFAULT_FORM_TEXT);
    await expect(defaultFormText).toBeVisible();
    await expect(defaultFormText).toHaveText(DEFAULT_FORM_TEXT);
});

test('should edit and save bind configuration', async ({ page }) => { 
    // given:
    //      existing bind configuraton from e2e-config.yaml file

    // when:
    //      clicking edit buttojn
    //      updating listener name
    //      clicking save button
    let listenerFormOption = page.getByRole('tree').getByText('(unnamed listener)');
    await expect(listenerFormOption).toBeVisible();
    await listenerFormOption.click();

    const trafficFormEditButton = page.getByRole('button', { name: 'Edit', exact: true });
    await expect(trafficFormEditButton).toBeVisible();
    await trafficFormEditButton.click();

    const nameInput = page.locator('#root_name');
    await expect(nameInput).toBeVisible();
    await nameInput.click();
    await page.keyboard.type(LISTENER_NEW_NAME);

    const trafficFormSaveButton = page.getByRole('button', { name: 'Save'});
    await expect(trafficFormSaveButton).toBeVisible();
    await trafficFormSaveButton.click();

    // then:
    //      bind configuration should be updated on front end
    //      bind configuration should be updated on in e2e-config.yaml file
    listenerFormOption = page.getByRole('tree').getByText(LISTENER_NEW_NAME);
    await expect(listenerFormOption).toBeVisible();

    const e2eConfig = fs.readFileSync(E2E_CONFIG_PATH, 'utf8');
    const e2eConfigYaml = yaml.load(e2eConfig) as any;
    expect(e2eConfigYaml.binds[0].listeners[0].name).toBe(LISTENER_NEW_NAME);
});