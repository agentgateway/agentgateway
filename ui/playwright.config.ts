import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
	testDir: './tests/e2e',
	timeout: 30_000,
	expect: {
		timeout: 5_000
	},
	fullyParallel: true,
	workers: 4,
	reporter: [['list']],
	use: {
		baseURL: 'http://127.0.0.1:19100',
		channel: 'chrome',
		trace: 'retain-on-failure'
	},
	webServer: {
		command: 'pnpm preview:e2e',
		url: 'http://127.0.0.1:19100',
		reuseExistingServer: !process.env.CI,
		timeout: 120_000
	},
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] }
		}
	]
});
