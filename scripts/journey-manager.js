#!/usr/bin/env node

/**
 * Journey Manager for AgentGateway E2E Tests
 * 
 * This script manages the execution of test journeys for manual CI/CD triggers.
 * It integrates with the existing smart defaults system and parallel test runner
 * to provide intelligent journey selection and execution.
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

class JourneyManager {
  constructor() {
    this.initializeWorkingDirectory();
    this.loadJourneyConfiguration();
    this.loadSmartDefaults();
    this.colors = {
      reset: '\x1b[0m',
      bright: '\x1b[1m',
      red: '\x1b[31m',
      green: '\x1b[32m',
      yellow: '\x1b[33m',
      blue: '\x1b[34m',
      magenta: '\x1b[35m',
      cyan: '\x1b[36m'
    };
  }

  /**
   * Initialize working directory detection and path helpers
   */
  initializeWorkingDirectory() {
    const currentDir = process.cwd();
    this.projectRoot = this.findProjectRoot(currentDir);
    
    if (!this.projectRoot) {
      throw new Error('Could not find AgentGateway project root. Please run from within the project directory.');
    }
    
    this.paths = {
      root: this.projectRoot,
      scripts: path.join(this.projectRoot, 'scripts'),
      ui: path.join(this.projectRoot, 'ui'),
      cypress: path.join(this.projectRoot, 'ui', 'cypress'),
      journeyConfig: path.join(this.projectRoot, 'ui', 'cypress', 'journeys.config.js'),
      
      // Helper methods for consistent path resolution
      toRoot: (relativePath) => path.join(this.projectRoot, relativePath),
      toScripts: (relativePath) => path.join(this.projectRoot, 'scripts', relativePath),
      toUI: (relativePath) => path.join(this.projectRoot, 'ui', relativePath),
      
      // Relative path helpers based on current working directory
      relativeToRoot: (targetPath) => path.relative(currentDir, path.join(this.projectRoot, targetPath)),
      relativeToScripts: (targetPath) => path.relative(currentDir, path.join(this.projectRoot, 'scripts', targetPath))
    };
  }

  /**
   * Find project root by looking for key indicators
   */
  findProjectRoot(startDir) {
    const indicators = ['Cargo.toml', 'rust-toolchain.toml', '.gitignore'];
    let currentDir = startDir;
    
    while (currentDir !== path.dirname(currentDir)) {
      const hasIndicators = indicators.some(indicator => 
        fs.existsSync(path.join(currentDir, indicator))
      );
      
      if (hasIndicators && fs.existsSync(path.join(currentDir, 'Cargo.toml'))) {
        const cargoContent = fs.readFileSync(path.join(currentDir, 'Cargo.toml'), 'utf8');
        if (cargoContent.includes('agentgateway') || cargoContent.includes('workspace')) {
          return currentDir;
        }
      }
      
      currentDir = path.dirname(currentDir);
    }
    
    return null;
  }

  /**
   * Load journey configuration from journeys.config.js
   */
  loadJourneyConfiguration() {
    if (fs.existsSync(this.paths.journeyConfig)) {
      // Clear require cache to ensure fresh load
      delete require.cache[require.resolve(this.paths.journeyConfig)];
      const config = require(this.paths.journeyConfig);
      
      this.journeys = config.journeys;
      this.presets = config.presets;
      this.resourceLevels = config.resourceLevels;
      this.environmentProfiles = config.environmentProfiles;
      this.utils = config.utils;
    } else {
      throw new Error(`Journey configuration not found at ${this.paths.journeyConfig}. Please ensure the file exists.`);
    }
  }

  /**
   * Load smart defaults system for resource optimization
   */
  loadSmartDefaults() {
    const smartDefaultsPath = this.paths.toScripts('smart-defaults-system.js');
    if (fs.existsSync(smartDefaultsPath)) {
      try {
        // Clear require cache
        delete require.cache[require.resolve(smartDefaultsPath)];
        const SmartDefaultsModule = require(smartDefaultsPath);
        
        // Handle different export patterns
        if (typeof SmartDefaultsModule === 'function') {
          this.smartDefaults = new SmartDefaultsModule();
        } else if (SmartDefaultsModule.SmartDefaultsSystem) {
          this.smartDefaults = new SmartDefaultsModule.SmartDefaultsSystem();
        } else if (SmartDefaultsModule.default) {
          this.smartDefaults = new SmartDefaultsModule.default();
        } else {
          this.log('⚠️  Smart defaults system found but could not instantiate', 'yellow');
          this.smartDefaults = null;
        }
      } catch (error) {
        this.log(`⚠️  Could not load smart defaults system: ${error.message}`, 'yellow');
        this.smartDefaults = null;
      }
    } else {
      this.log('⚠️  Smart defaults system not found, using fallback calculations', 'yellow');
      this.smartDefaults = null;
    }
  }

  /**
   * Colored logging utility
   */
  log(message, color = 'reset') {
    const colorCode = this.colors[color] || this.colors.reset;
    console.log(`${colorCode}${message}${this.colors.reset}`);
  }

  /**
   * Validate journey selection
   */
  validateJourneySelection(selection, customJourneys = '') {
    const validJourneys = Object.keys(this.journeys);
    const validPresets = Object.keys(this.presets);
    const validSelections = ['all', ...validJourneys, ...validPresets, 'custom'];

    if (!validSelections.includes(selection)) {
      throw new Error(`Invalid journey selection: ${selection}. Valid options: ${validSelections.join(', ')}`);
    }

    if (selection === 'custom') {
      if (!customJourneys) {
        throw new Error('Custom journeys must be specified when using custom selection');
      }
      
      const customList = customJourneys.split(',').map(j => j.trim()).filter(Boolean);
      const invalidJourneys = customList.filter(j => !validJourneys.includes(j));
      
      if (invalidJourneys.length > 0) {
        throw new Error(`Invalid custom journeys: ${invalidJourneys.join(', ')}. Valid journeys: ${validJourneys.join(', ')}`);
      }
    }

    return true;
  }

  /**
   * Resolve journey list from selection
   */
  resolveJourneyList(selection, customJourneys = '') {
    if (selection === 'all') {
      return Object.keys(this.journeys);
    }
    
    if (selection === 'custom') {
      return customJourneys.split(',').map(j => j.trim()).filter(Boolean);
    }
    
    if (this.presets[selection]) {
      return this.presets[selection].journeys;
    }
    
    if (this.journeys[selection]) {
      return [selection];
    }
    
    throw new Error(`Unable to resolve journey selection: ${selection}`);
  }

  /**
   * Estimate execution time for journey list
   */
  estimateExecutionTime(journeyList) {
    let totalMinutes = 0;
    
    journeyList.forEach(journeyName => {
      const journey = this.journeys[journeyName];
      if (journey && journey.estimatedDuration) {
        // Parse duration like "5-7 minutes" and take average
        const match = journey.estimatedDuration.match(/(\d+)-(\d+)/);
        if (match) {
          const min = parseInt(match[1]);
          const max = parseInt(match[2]);
          totalMinutes += (min + max) / 2;
        }
      }
    });
    
    return {
      totalMinutes,
      formattedDuration: `${Math.floor(totalMinutes)}-${Math.ceil(totalMinutes * 1.2)} minutes`
    };
  }

  /**
   * Calculate optimal resources for journey execution
   */
  calculateOptimalResources(journeyList, environmentProfile = 'standard') {
    // Calculate base resource requirements
    const maxResourceRequirement = journeyList.reduce((max, journeyName) => {
      const journey = this.journeys[journeyName];
      const requirement = journey?.resourceRequirement || 'low';
      
      const levels = { minimal: 1, low: 2, medium: 3, high: 4 };
      return Math.max(max, levels[requirement] || 2);
    }, 1);

    const requirementNames = ['minimal', 'low', 'medium', 'high'];
    const requirementName = requirementNames[maxResourceRequirement - 1] || 'low';
    const baseResources = this.resourceLevels[requirementName];

    // Use smart defaults system if available
    if (this.smartDefaults && typeof this.smartDefaults.generateRecommendations === 'function') {
      try {
        const recommendations = this.smartDefaults.generateRecommendations({
          environmentProfile,
          maxResourceRequirement,
          journeyCount: journeyList.length,
          testType: 'journey'
        });
        
        if (recommendations && typeof recommendations === 'object') {
          return {
            ...baseResources,
            ...recommendations,
            source: 'smart-defaults'
          };
        }
      } catch (error) {
        this.log(`⚠️  Smart defaults calculation failed: ${error.message}`, 'yellow');
      }
    }

    // Fallback resource calculation
    const fallbackResources = {
      workers: Math.min(journeyList.length, baseResources.workers),
      memoryLimit: baseResources.memoryLimit,
      description: baseResources.description,
      source: 'fallback'
    };

    // Apply environment profile optimizations
    if (this.environmentProfiles[environmentProfile]) {
      const profile = this.environmentProfiles[environmentProfile];
      fallbackResources.workers = Math.max(1, Math.round(fallbackResources.workers * profile.workerMultiplier));
      fallbackResources.memoryLimit = Math.max(50, Math.min(90, fallbackResources.memoryLimit - profile.memoryReduction));
      fallbackResources.timeoutMultiplier = profile.timeoutMultiplier;
      fallbackResources.retryCount = profile.retryCount;
    }

    return fallbackResources;
  }

  /**
   * Generate test patterns for Cypress execution
   */
  generateTestPattern(journeyList) {
    const patterns = journeyList.map(journeyName => {
      const journey = this.journeys[journeyName];
      return journey ? journey.pattern : null;
    }).filter(Boolean);

    return patterns.join(',');
  }

  /**
   * Check journey dependencies
   */
  checkDependencies(journeyList) {
    const missing = [];
    
    journeyList.forEach(journeyName => {
      const journey = this.journeys[journeyName];
      if (journey && journey.dependencies && journey.dependencies.length > 0) {
        journey.dependencies.forEach(dep => {
          if (!journeyList.includes(dep)) {
            missing.push({ journey: journeyName, missingDependency: dep });
          }
        });
      }
    });
    
    return missing;
  }

  /**
   * Display journey information
   */
  displayJourneyInfo(journeyList) {
    this.log('\n📋 Journey Information:', 'cyan');
    
    journeyList.forEach(journeyName => {
      const journey = this.journeys[journeyName];
      if (journey) {
        this.log(`\n  ${journey.name}`, 'bright');
        this.log(`    Description: ${journey.description}`, 'reset');
        this.log(`    Duration: ${journey.estimatedDuration}`, 'reset');
        this.log(`    Resource Requirement: ${journey.resourceRequirement}`, 'reset');
        this.log(`    Tests: ${journey.tests.join(', ')}`, 'reset');
        if (journey.dependencies && journey.dependencies.length > 0) {
          this.log(`    Dependencies: ${journey.dependencies.join(', ')}`, 'yellow');
        }
      }
    });
  }

  /**
   * Execute journeys using the existing parallel test runner
   */
  async executeJourneys(options) {
    const {
      journeySelection,
      customJourneys = '',
      browser = 'chrome',
      executionMode = 'headless',
      workerCount = 'auto',
      environmentProfile = 'standard',
      dryRun = false
    } = options;

    try {
      // Validate and resolve journey selection
      this.log('🔍 Validating journey selection...', 'cyan');
      this.validateJourneySelection(journeySelection, customJourneys);
      const journeyList = this.resolveJourneyList(journeySelection, customJourneys);
      
      // Check dependencies
      const missingDeps = this.checkDependencies(journeyList);
      if (missingDeps.length > 0) {
        this.log('\n⚠️  Missing dependencies detected:', 'yellow');
        missingDeps.forEach(({ journey, missingDependency }) => {
          this.log(`  ${journey} requires ${missingDependency}`, 'yellow');
        });
        this.log('\n💡 Consider adding missing dependencies to your journey selection.', 'yellow');
      }
      
      // Calculate optimal resources
      this.log('\n🔧 Calculating optimal resources...', 'cyan');
      const resources = this.calculateOptimalResources(journeyList, environmentProfile);
      const finalWorkerCount = workerCount === 'auto' ? resources.workers : parseInt(workerCount);
      
      // Generate test pattern
      const testPattern = this.generateTestPattern(journeyList);
      
      // Estimate execution time
      const timeEstimate = this.estimateExecutionTime(journeyList);
      
      // Display execution plan
      this.log('\n🚀 Execution Plan:', 'green');
      this.log(`  Journey Selection: ${journeySelection}`, 'reset');
      this.log(`  Journeys: ${journeyList.join(', ')}`, 'reset');
      this.log(`  Estimated Duration: ${timeEstimate.formattedDuration}`, 'reset');
      this.log(`  Workers: ${finalWorkerCount}`, 'reset');
      this.log(`  Memory Limit: ${resources.memoryLimit}%`, 'reset');
      this.log(`  Browser: ${browser}`, 'reset');
      this.log(`  Execution Mode: ${executionMode}`, 'reset');
      this.log(`  Environment Profile: ${environmentProfile}`, 'reset');
      this.log(`  Resource Source: ${resources.source || 'unknown'}`, 'reset');
      
      if (dryRun) {
        this.log('\n🔍 Dry run mode - execution plan displayed above', 'yellow');
        this.displayJourneyInfo(journeyList);
        return;
      }
      
      // Execute using existing parallel test runner
      this.log('\n▶️  Starting journey execution...', 'green');
      const parallelRunnerPath = this.paths.toScripts('parallel-test-runner.js');
      
      if (!fs.existsSync(parallelRunnerPath)) {
        throw new Error(`Parallel test runner not found at ${parallelRunnerPath}`);
      }
      
      // Create a custom parallel test runner instance with journey support
      await this.executeWithJourneySupport({
        journeyList,
        finalWorkerCount,
        resources,
        browser,
        executionMode,
        environmentProfile,
        journeySelection
      });
      
      // Generate journey execution report
      this.generateExecutionReport({
        journeySelection,
        journeyList,
        timeEstimate,
        resources: { 
          workers: finalWorkerCount, 
          memoryLimit: resources.memoryLimit,
          source: resources.source 
        },
        browser,
        executionMode,
        environmentProfile,
        success: true,
        timestamp: new Date().toISOString()
      });
      
      this.log('\n✅ Journey execution completed successfully!', 'green');
      
    } catch (error) {
      this.log(`\n❌ Journey execution failed: ${error.message}`, 'red');
      
      // Generate failure report
      this.generateExecutionReport({
        journeySelection,
        journeyList: this.resolveJourneyList(journeySelection, customJourneys).catch(() => []),
        timeEstimate: { totalMinutes: 0, formattedDuration: 'unknown' },
        resources: { workers: 0, memoryLimit: 0, source: 'error' },
        browser,
        executionMode,
        environmentProfile,
        success: false,
        error: error.message,
        timestamp: new Date().toISOString()
      });
      
      process.exit(1);
    }
  }

  /**
   * Execute with journey support using the parallel test runner
   */
  async executeWithJourneySupport(options) {
    const {
      journeyList,
      finalWorkerCount,
      resources,
      browser,
      executionMode,
      environmentProfile,
      journeySelection
    } = options;

    try {
      // Generate journey patterns for the test scheduler
      const journeyPatterns = journeyList.map(journeyName => {
        const journey = this.journeys[journeyName];
        return journey ? journey.pattern : null;
      }).filter(Boolean);

      this.log(`🎯 Journey patterns: ${journeyPatterns.join(', ')}`, 'blue');

      // Load and configure the parallel test runner
      const ParallelTestRunner = require(this.paths.toScripts('parallel-test-runner.js'));
      
      const runner = new ParallelTestRunner({
        baseDir: this.paths.ui,
        maxWorkers: finalWorkerCount,
        memoryLimit: resources.memoryLimit,
        browser: browser,
        headless: executionMode === 'headless',
        video: true,
        quiet: false,
        debug: false,
        ci: true,
        journeyFilter: journeySelection,
        journeyPatterns: journeyPatterns
      });

      // Initialize and run the tests
      await runner.initialize();
      const results = await runner.run();

      this.log(`\n🎉 Journey execution completed!`, 'green');
      this.log(`📊 Results: ${results.tests.passed}/${results.tests.total} tests passed`, 'green');

      if (results.tests.failed > 0) {
        throw new Error(`${results.tests.failed} test(s) failed`);
      }

      return results;

    } catch (error) {
      this.log(`❌ Journey execution failed: ${error.message}`, 'red');
      throw error;
    }
  }

  /**
   * Generate execution report
   */
  generateExecutionReport(executionData) {
    const report = {
      ...executionData,
      journeyDetails: (executionData.journeyList || []).map(journeyName => ({
        name: journeyName,
        ...this.journeys[journeyName]
      }))
    };

    const reportPath = this.paths.toRoot('journey-execution-report.json');
    
    try {
      fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
      this.log(`\n📊 Execution report generated: ${reportPath}`, 'cyan');
    } catch (error) {
      this.log(`⚠️  Could not generate execution report: ${error.message}`, 'yellow');
    }
  }

  /**
   * Display available journeys and presets
   */
  displayAvailableOptions() {
    this.log('\n📋 Available Journeys:', 'cyan');
    Object.entries(this.journeys).forEach(([key, journey]) => {
      this.log(`  ${key}: ${journey.name} - ${journey.description}`, 'reset');
    });
    
    this.log('\n🎯 Available Presets:', 'cyan');
    Object.entries(this.presets).forEach(([key, preset]) => {
      this.log(`  ${key}: ${preset.name} - ${preset.description}`, 'reset');
      this.log(`    Journeys: ${preset.journeys.join(', ')}`, 'reset');
      this.log(`    Duration: ${preset.estimatedDuration}`, 'reset');
    });
  }

  /**
   * Validate only mode - check journey selection without execution
   */
  validateOnly(options) {
    const { journeySelection, customJourneys = '' } = options;
    
    try {
      this.validateJourneySelection(journeySelection, customJourneys);
      const journeyList = this.resolveJourneyList(journeySelection, customJourneys);
      const timeEstimate = this.estimateExecutionTime(journeyList);
      const resources = this.calculateOptimalResources(journeyList, options.environmentProfile);
      const missingDeps = this.checkDependencies(journeyList);
      
      this.log('✅ Journey selection validated successfully', 'green');
      this.log(`📋 Resolved journeys: ${journeyList.join(', ')}`, 'reset');
      this.log(`⏱️  Estimated duration: ${timeEstimate.formattedDuration}`, 'reset');
      this.log(`🔧 Recommended workers: ${resources.workers}`, 'reset');
      this.log(`💾 Recommended memory limit: ${resources.memoryLimit}%`, 'reset');
      
      if (missingDeps.length > 0) {
        this.log('\n⚠️  Dependency warnings:', 'yellow');
        missingDeps.forEach(({ journey, missingDependency }) => {
          this.log(`  ${journey} recommends ${missingDependency}`, 'yellow');
        });
      }
      
      this.displayJourneyInfo(journeyList);
      
    } catch (error) {
      this.log(`❌ Validation failed: ${error.message}`, 'red');
      process.exit(1);
    }
  }
}

// CLI Interface
if (require.main === module) {
  const args = process.argv.slice(2);
  const options = {};
  
  // Parse command line arguments
  args.forEach(arg => {
    if (arg.startsWith('--')) {
      const [key, value] = arg.substring(2).split('=');
      const camelKey = key.replace(/-([a-z])/g, (g) => g[1].toUpperCase());
      options[camelKey] = value || true;
    }
  });

  async function main() {
    try {
      const manager = new JourneyManager();
      
      if (options.help) {
        console.log(`
Journey Manager for AgentGateway E2E Tests

Usage:
  node journey-manager.js [options]

Options:
  --validate-only              Validate journey selection without execution
  --execute                    Execute selected journeys
  --dry-run                    Show execution plan without running tests
  --list                       List available journeys and presets
  
  --journey-selection=<name>   Journey or preset to execute (required)
  --custom-journeys=<list>     Comma-separated list for custom selection
  --browser=<name>             Browser to use (chrome, firefox, edge)
  --execution-mode=<mode>      Execution mode (headless, headed)
  --worker-count=<number>      Number of parallel workers (auto, 1, 2, 4, 8)
  --environment-profile=<name> Environment profile (standard, conservative, high-performance, ci-optimized)
  
  --help                       Show this help message

Examples:
  node journey-manager.js --validate-only --journey-selection=smoke
  node journey-manager.js --execute --journey-selection=quick-validation --browser=chrome
  node journey-manager.js --dry-run --journey-selection=custom --custom-journeys=smoke,foundation
  node journey-manager.js --list
`);
        process.exit(0);
      }
      
      if (options.list) {
        manager.displayAvailableOptions();
        process.exit(0);
      }
      
      if (options.validateOnly) {
        manager.validateOnly(options);
      } else if (options.execute) {
        await manager.executeJourneys(options);
      } else if (options.dryRun) {
        await manager.executeJourneys({ ...options, dryRun: true });
      } else {
        console.log('Usage: node journey-manager.js --validate-only OR --execute OR --dry-run OR --list [options]');
        console.log('Use --help for detailed usage information');
        process.exit(1);
      }
    } catch (error) {
      console.error(`❌ Error: ${error.message}`);
      process.exit(1);
    }
  }

  main().catch(error => {
    console.error(`❌ Unhandled error: ${error.message}`);
    process.exit(1);
  });
}

module.exports = JourneyManager;
