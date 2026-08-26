import { defineConfig, devices } from '@playwright/test';

import baseConfig from './playwright.config';

if (!process.env.VISUAL_REFERENCE_DIR || !process.env.VISUAL_UI_DIR) {
	throw new Error(
		'Run pnpm test:visual-regressions to generate a reference and compare the working tree.'
	);
}

export default defineConfig({
	...baseConfig,
	testDir: './visual-regression',
	testMatch: 'visual-regression.spec.ts',
	workers: 1,
	outputDir: process.env.VISUAL_RESULTS_DIR,
	updateSnapshots: 'none',
	reporter: [['list'], ['json', { outputFile: process.env.VISUAL_JSON_REPORT }]],
	snapshotPathTemplate: `${process.env.VISUAL_REFERENCE_DIR}/{projectName}/{arg}{ext}`,
	expect: {
		toHaveScreenshot: {
			stylePath: './visual-regression/visual-regression.css'
		}
	},
	use: {
		...baseConfig.use,
		timezoneId: 'UTC'
	},
	webServer: {
		command: 'pnpm exec vite preview --mode e2e --strictPort',
		cwd: process.env.VISUAL_UI_DIR,
		url: 'http://127.0.0.1:19100',
		reuseExistingServer: false,
		timeout: 120_000
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
