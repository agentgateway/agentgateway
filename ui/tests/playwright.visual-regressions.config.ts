import { defineConfig, devices } from '@playwright/test';

import baseConfig from './playwright.config';

export default defineConfig({
	...baseConfig,
	testDir: './visual-regression',
	updateSnapshots: 'none',
	reporter: [['list'], ['html', { outputFolder: './playwright-report', open: 'never' }]],
	snapshotPathTemplate: '{testDir}/baselines/{projectName}/{arg}{ext}',
	expect: {
		toHaveScreenshot: {
			stylePath: './visual-regression/visual-regression.css'
		}
	},
	use: {
		...baseConfig.use,
		timezoneId: 'UTC'
	},
	projects: [
		{
			name: 'light',
			use: { ...devices['Desktop Chrome'], colorScheme: 'light' }
		},
		{
			name: 'dark',
			use: { ...devices['Desktop Chrome'], colorScheme: 'dark' }
		}
	]
});
