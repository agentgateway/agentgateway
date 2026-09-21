import { expect, test } from '@playwright/test';

import { mockGateway, mockXdsGateway, unconfiguredConfig } from '../fixtures';

const routes = [
	['cel', '/cel', 'CEL Playground'],
	['llm-analytics', '/llm/analytics', 'Analytics'],
	['llm-client-setup', '/llm/client-setup', 'Client Setup'],
	['llm-costs', '/llm/costs', 'LLM Costs'],
	['llm-get-started', '/llm/get-started', 'Enable LLM', 'unconfigured'],
	['llm-guardrails', '/llm/guardrails', 'LLM Guardrails'],
	['llm-keys', '/llm/keys', 'Virtual API Keys'],
	['llm-logs', '/llm/logs', 'Logs'],
	['llm-models', '/llm/models', 'LLM Models'],
	['llm-playground', '/llm/playground', 'LLM Playground'],
	['llm-policies', '/llm/policies', 'LLM Policies'],
	['llm-providers', '/llm/providers', 'LLM Providers'],
	['mcp-get-started', '/mcp/get-started', 'Enable MCP', 'unconfigured'],
	['mcp-playground', '/mcp/playground', 'MCP Playground'],
	['mcp-policies', '/mcp/policies', 'MCP Policies'],
	['mcp-servers', '/mcp/servers', 'MCP Servers'],
	['overview', '/', 'Gateway Overview'],
	['raw-config', '/raw-config', 'Raw Configuration'],
	['settings', '/settings', 'UI Settings'],
	['traffic-gateways', '/traffic/gateways', 'Traffic Gateways'],
	['traffic-get-started', '/traffic/get-started', 'Enable Traffic', 'unconfigured'],
	['traffic-listeners', '/traffic/listeners', 'Traffic Listeners'],
	['traffic-policies', '/traffic/policies', 'Policies', 'xds'],
	['traffic-routes', '/traffic/routes', 'Traffic Routes']
] as const;

for (const [name, path, heading, scenario = 'populated'] of routes) {
	test(`visual baseline: ${heading}`, async ({ page }) => {
		if (scenario === 'xds') await mockXdsGateway(page);
		else await mockGateway(page, scenario === 'unconfigured' ? unconfiguredConfig() : undefined);
		await page.goto(path);
		const pageHeading = page.getByRole('heading', { name: heading, exact: true, level: 2 });
		await expect(pageHeading).toBeVisible();
		if (name === 'cel') {
			await expect(page.locator('.monaco-editor .view-lines')).toHaveCount(2);
		}
		if (name === 'raw-config') {
			await expect(page.locator('.monaco-editor .view-lines')).toContainText('config:');
		}
		const mask = [
			page.locator('.log-td-time'),
			page.locator('.message-meta-chip').filter({ hasText: /^\d+(?:\.\d+)?(?:ms|s)$/ })
		];
		await expect(page).toHaveScreenshot(`${name}-full-page.png`, {
			fullPage: true,
			mask
		});
	});
}
