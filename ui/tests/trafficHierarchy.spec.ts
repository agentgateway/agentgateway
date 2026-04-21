import { expect, test, type Locator, type Page } from '@playwright/test';
import * as fs from 'fs';
import * as yaml from 'js-yaml';
import { getPolicyTypesForScope, type PolicyScope } from '../src/components/TrafficHierarchy/policyTypes';

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

test('should traverse Traffic Hierarchy tree and verify all nodes', async ({ page }) => { 
    // ── Helper functions ─────────────────────────────────────────────────────
    function getTreeNode(nodeText: string, exact = false): Locator {
        return page.getByRole('tree')
            .locator('.ant-tree-treenode')
            .filter(exact ? { has: page.getByText(nodeText, { exact: true }) } : { hasText: nodeText })
            .first();
    }

    async function verifyAddPolicyMenu(nodeText: string, scope: PolicyScope, exact = false) {
        // 'listener', 'route', 'backend' use a different key convention than
        // 'mcp', 'llm', 'mcpTarget' — see HierarchyTree.tsx for the asymmetry
        const isScopedKey = ['listener', 'route', 'backend'].includes(scope);
        const addPolicyMenuIdSuffix = isScopedKey ? '-addPolicy' : '-add-policy';
        const policyItemSuffix = (key: string) =>
            isScopedKey ? `-add-policy-${key}` : `-${key}`;

        const openDropdown = page.locator('.ant-dropdown:not(.ant-dropdown-hidden)');
        const submenuPopup = page.locator('.ant-dropdown-menu-submenu-popup').filter({ visible: true });

        // Single atomic retry: click to open dropdown, then hover to open submenu.
        // State is reset at the start of each retry — if the dropdown is still open
        // from a prior attempt, close it first so the click opens (not toggles) it.
        await expect(async () => {
            if (await openDropdown.isVisible()) {
                await page.keyboard.press('Escape');
                await openDropdown.waitFor({ state: 'hidden', timeout: 2000 });
            }
            await getTreeNode(nodeText, exact).locator('button').click();
            await expect(openDropdown).toBeVisible({ timeout: 2000 });
            await openDropdown.locator(`[data-menu-id$="${addPolicyMenuIdSuffix}"]`).hover();
            await expect(submenuPopup).toBeVisible({ timeout: 2000 });
        }).toPass({ timeout: 15_000, intervals: [500, 1000, 2000, 3000] });


        for (const pt of getPolicyTypesForScope(scope)) {
            await expect(
                submenuPopup.locator(`[data-menu-id$="${policyItemSuffix(pt.key)}"]`)
            ).toBeVisible();
        }

        // Press Escape twice: first closes the submenu, second closes the parent dropdown.
        // Then wait for the overlay to fully disappear before the next tree interaction —
        // avoids triggering re-renders via a stray mouse click that destabilises the tree.
        await page.keyboard.press('Escape');
        await page.keyboard.press('Escape');
        await page.waitForSelector('.ant-dropdown:not(.ant-dropdown-hidden)', { state: 'hidden' });
    }

    async function clickMenuItemAndExpect(
        triggerNode: string,
        menuItemSuffix: string,
        expectedText: string
    ) {
        const openDropdown = page.locator('.ant-dropdown:not(.ant-dropdown-hidden)');
        await expect(async () => {
            if (await openDropdown.isVisible()) {
                await page.keyboard.press('Escape');
                await openDropdown.waitFor({ state: 'hidden', timeout: 2000 });
            }
            await getTreeNode(triggerNode).locator('button').click();
            await expect(openDropdown).toBeVisible({ timeout: 2000 });
            await page.locator(`[data-menu-id$="${menuItemSuffix}"]`).click();
            await expect(tree.getByText(expectedText)).toBeVisible({ timeout: 2000 });
        }).toPass({ timeout: 15_000, intervals: [500, 1000, 2000, 3000] });
    }

    async function addBackendOfType(backendType: string, expectedTreeLabel: string) {
        const openDropdown = page.locator('.ant-dropdown:not(.ant-dropdown-hidden)');
        const backendSubmenu = page.locator('.ant-dropdown-menu-submenu-popup').filter({ visible: true });
        await expect(async () => {
            if (await openDropdown.isVisible()) {
                await page.keyboard.press('Escape');
                await openDropdown.waitFor({ state: 'hidden', timeout: 2000 });
            }
            await getTreeNode('route').locator('button').click();
            await expect(openDropdown).toBeVisible({ timeout: 2000 });
            await page.locator('[data-menu-id$="-addBackend"]').hover();
            await expect(backendSubmenu).toBeVisible({ timeout: 2000 });
            // Scope the click to the submenu to avoid ambiguous selectors (e.g. -mcp)
            await backendSubmenu.locator(`[data-menu-id$="-${backendType}"]`).click();
            await expect(tree.getByText(expectedTreeLabel)).toBeVisible({ timeout: 2000 });
        }).toPass({ timeout: 15_000, intervals: [500, 1000, 2000, 3000] });
    }

    // Start from a completely blank config so every node is added fresh
    fs.writeFileSync(E2E_CONFIG_PATH, yaml.dump({}));
    await page.reload();

    // In the empty state the "Add" button is inside the ant-empty component;
    // once content exists it moves to the card header. Both are inside .ant-card.
    const addButton = page.locator('.ant-card').getByRole('button', { name: 'Add' }).first();
    const tree = page.getByRole('tree');
    
    // ── MCP Configuration ─────────────────────────────────────────────────────
    // Add MCP Config via top-level Add menu (key: "mcp")
    await addButton.click();
    await page.locator('[data-menu-id$="-mcp"]').click();
    await expect(tree.getByText('MCP Configuration')).toBeVisible();
    // Verify all mcp-scope policies are in the Add Policy submenu
    await verifyAddPolicyMenu('MCP Configuration', 'mcp');
    // Add MCP Target (handler names it "target-1", so tree shows "target-1")
    await clickMenuItemAndExpect('MCP Configuration', '-add-target', 'target-1');
    // Verify all mcpTarget-scope policies are in the Add Policy submenu
    await verifyAddPolicyMenu('target-1', 'mcpTarget');
    
    // ── LLM Configuration ─────────────────────────────────────────────────────
    // Add LLM Config via top-level Add menu (key: "llm")
    await addButton.click();
    await page.locator('[data-menu-id$="-llm"]').click();
    await expect(tree.getByText('LLM Configuration')).toBeVisible();
    // Verify all llm-scope policies are in the Add Policy submenu
    await verifyAddPolicyMenu('LLM Configuration', 'llm');
    // Add LLM Model (handler names it "model-1", so tree shows "model-1")
    // Note: model nodes have no "Add Policy" — llm policies live on the
    // LLM Configuration node itself (already verified above)
    await clickMenuItemAndExpect('LLM Configuration', '-add-model', 'model-1');

    // ── Port Bind ─────────────────────────────────────────────────────────────
    // Add Bind via top-level Add menu (key: "bind"); defaults to port 8080
    await addButton.click();
    await page.locator('[data-menu-id$="-bind"]').click();
    await expect(tree.getByText('Port 8080')).toBeVisible();
    // Bind nodes have no "Add Policy" in their context menu
    // Add Listener (handler sets name: "listener", so tree shows "listener")
    await clickMenuItemAndExpect('Port 8080', '-addListener', 'listener');
    // Verify all listener-scope policies are in the Add Policy submenu
    await verifyAddPolicyMenu('listener', 'listener');
    // Add Route (handler sets name: "route", so tree shows "route")
    await clickMenuItemAndExpect('listener', '-addRoute', 'route');
    // Verify all route-scope policies are in the Add Policy submenu
    await verifyAddPolicyMenu('route', 'route');
    // Add Backend — Host and verify policies
    await addBackendOfType('host', 'Host');
    await verifyAddPolicyMenu('Host', 'backend');
    // Add Backend - Service and verify poliices
    await addBackendOfType('service', 'Service');
    await verifyAddPolicyMenu('Service', 'backend');
    // Add Backend - Dynamic and verify policies
    await addBackendOfType('dynamic', 'Backend');
    await verifyAddPolicyMenu('Backend', 'backend');
    // Add Backend - MCP and verify policies
    await addBackendOfType('mcp', 'MCP');
    await verifyAddPolicyMenu('MCP', 'backend', true);
    // Add Backend - AI and verify policies
    await addBackendOfType('ai', 'AI');
    await verifyAddPolicyMenu('AI', 'backend', true);
});