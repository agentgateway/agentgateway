const { defineConfig } = require('cypress');

module.exports = defineConfig({
  e2e: {
    // Base URL for the application
    baseUrl: 'http://localhost:3000',
    
    // Test files location
    specPattern: 'ui/cypress/e2e/**/*.cy.{js,jsx,ts,tsx}',
    
    // Support file
    supportFile: 'ui/cypress/support/e2e.ts',
    
    // Fixtures
    fixturesFolder: 'ui/cypress/fixtures',
    
    // Screenshots and videos
    screenshotsFolder: 'ui/cypress/screenshots',
    videosFolder: 'ui/cypress/videos',
    
    // Video recording
    video: true,
    videoCompression: 32,
    
    // Screenshots
    screenshotOnRunFailure: true,
    
    // Viewport
    viewportWidth: 1280,
    viewportHeight: 720,
    
    // Timeouts
    defaultCommandTimeout: 10000,
    requestTimeout: 15000,
    responseTimeout: 15000,
    pageLoadTimeout: 30000,
    
    // Retry configuration
    retries: {
      runMode: 2,
      openMode: 0
    },
    
    // Test isolation
    testIsolation: true,
    
    // Experimental features
    experimentalStudio: false,
    experimentalWebKitSupport: false,
    
    // Environment variables
    env: {
      // Add any environment-specific variables here
      PARALLEL_MODE: false // Will be overridden by parallel runner
    },
    
    setupNodeEvents(on, config) {
      // Implement node event listeners here
      
      // Task for logging from tests
      on('task', {
        log(message) {
          console.log(message);
          return null;
        },
        
        // Task for setting test metadata
        setTestMetadata(metadata) {
          console.log('Test metadata:', metadata);
          return null;
        }
      });
      
      // Before browser launch
      on('before:browser:launch', (browser = {}, launchOptions) => {
        // Configure browser launch options
        if (browser.name === 'chrome') {
          launchOptions.args.push('--disable-dev-shm-usage');
          launchOptions.args.push('--no-sandbox');
          launchOptions.args.push('--disable-gpu');
        }
        
        return launchOptions;
      });
      
      // After screenshot
      on('after:screenshot', (details) => {
        console.log('Screenshot taken:', details.path);
      });
      
      // Before spec
      on('before:spec', (spec) => {
        console.log('Running spec:', spec.relative);
      });
      
      // After spec
      on('after:spec', (spec, results) => {
        console.log('Completed spec:', spec.relative);
        console.log('Results:', {
          tests: results.stats.tests,
          passes: results.stats.passes,
          failures: results.stats.failures,
          duration: results.stats.duration
        });
      });
      
      return config;
    }
  },
  
  // Component testing configuration (if needed in the future)
  component: {
    devServer: {
      framework: 'next',
      bundler: 'webpack'
    },
    specPattern: 'ui/src/**/*.cy.{js,jsx,ts,tsx}',
    supportFile: 'ui/cypress/support/component.ts'
  }
});
