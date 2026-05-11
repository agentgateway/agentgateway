import { defineConfig, devices } from '@playwright/test';

/**
 * Read environment variables from file.
 * https://github.com/motdotla/dotenv
 */
// import dotenv from 'dotenv';
// import path from 'path';
// dotenv.config({ path: path.resolve(__dirname, '.env') });

const STANDARD_BASE_URL = process.env.BASE_URL || 'http://127.0.0.1:15000';
const XDS_BASE_URL = process.env.BASE_URL ||'http://127.0.0.1:15001';

const XDS_SPEC = /xdsMode\.spec\.ts/;

/**
 * See https://playwright.dev/docs/test-configuration.
 */
export default defineConfig({
  timeout: 120000,
  testDir: './tests',
  /* Run tests in files in parallel */
  fullyParallel: true,
  /* Fail the build on CI if you accidentally left test.only in the source code. */
  forbidOnly: !!process.env.CI,
  /* Retry on CI only */
  retries: process.env.CI ? 2 : 0,
  /* Opt out of parallel tests on CI and local development (same config file used for all tests). */
  workers: 1,
  /* Reporter to use. See https://playwright.dev/docs/test-reporters */
  reporter: 'html',
  /* Shared settings for all the projects below. See https://playwright.dev/docs/api/class-testoptions. */
  use: {
    /* Base URL to use in actions like `await page.goto('')`. */
    baseURL: process.env.BASE_URL ||'http://127.0.0.1:15000',

    /* Collect trace when retrying the failed test. See https://playwright.dev/docs/trace-viewer */
    trace: 'on-first-retry',

    /* Record video of each test run (stored under `test-results/` directory) */
    video: {
      mode: 'on',
      size: { width: 1280, height: 720 },
    }
  },

  /* Configure projects for major browsers */
  projects: [
    // standard mode - ie all tests other than xDS-mode (port 15000)
    {
      name: 'chromium',
      testIgnore: XDS_SPEC,
      use: { ...devices['Desktop Chrome'], baseURL: STANDARD_BASE_URL },
    },
    {
      name: 'firefox',
      testIgnore: XDS_SPEC,
      use: { ...devices['Desktop Firefox'], baseURL: STANDARD_BASE_URL },
    },
    {
      name: 'webkit',
      testIgnore: XDS_SPEC,
      use: { ...devices['Desktop Safari'], baseURL: STANDARD_BASE_URL },
    },

    // xDS mode: ONLY xdsMode.spec.ts (port 15001)
    {
      name: 'xds-chromium',
      testMatch: XDS_SPEC,
      use: { ...devices['Desktop Chrome'], baseURL: XDS_BASE_URL },
    },
    {
      name: 'xds-firefox',
      testMatch: XDS_SPEC,
      use: { ...devices['Desktop Firefox'], baseURL: XDS_BASE_URL },
    },
    {
      name: 'xds-webkit',
      testMatch: XDS_SPEC,
      use: { ...devices['Desktop Safari'], baseURL: XDS_BASE_URL },
    },

    /* Test against mobile viewports. */
    // {
    //   name: 'Mobile Chrome',
    //   use: { ...devices['Pixel 5'] },
    // },
    // {
    //   name: 'Mobile Safari',
    //   use: { ...devices['iPhone 12'] },
    // },

    /* Test against branded browsers. */
    // {
    //   name: 'Microsoft Edge',
    //   use: { ...devices['Desktop Edge'], channel: 'msedge' },
    // },
    // {
    //   name: 'Google Chrome',
    //   use: { ...devices['Desktop Chrome'], channel: 'chrome' },
    // },
  ],

  /* Run your local dev server before starting the tests */
  // webServer: {
  //   command: 'npm run start',
  //   url: 'http://localhost:3000',
  //   reuseExistingServer: !process.env.CI,
  // },
});
