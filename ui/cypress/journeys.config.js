/**
 * Journey Configuration for AgentGateway E2E Tests
 * 
 * This configuration defines test journeys for manual CI/CD trigger support.
 * Each journey represents a logical grouping of tests that can be executed
 * independently or in combination with other journeys.
 */

const journeys = {
  smoke: {
    name: "Smoke Tests",
    description: "Critical path validation - essential functionality",
    pattern: "cypress/e2e/smoke/**/*.cy.ts",
    estimatedDuration: "2-3 minutes",
    resourceRequirement: "minimal",
    tests: ["api-health.cy.ts", "critical-path.cy.ts"],
    priority: "critical",
    dependencies: [],
    tags: ["critical", "fast", "essential"]
  },
  
  foundation: {
    name: "Foundation Tests", 
    description: "Basic app functionality - loading and navigation",
    pattern: "cypress/e2e/foundation/**/*.cy.ts",
    estimatedDuration: "3-4 minutes",
    resourceRequirement: "low",
    tests: ["app-loads.cy.ts", "navigation-test.cy.ts"],
    priority: "high",
    dependencies: [],
    tags: ["foundation", "navigation", "basic"]
  },
  
  "setup-wizard": {
    name: "Setup Wizard",
    description: "Wizard flow testing - complete setup process",
    pattern: "cypress/e2e/setup-wizard/**/*.cy.ts", 
    estimatedDuration: "5-7 minutes",
    resourceRequirement: "medium",
    tests: ["wizard-complete-flow.cy.ts", "wizard-navigation.cy.ts"],
    priority: "high",
    dependencies: ["foundation"],
    tags: ["wizard", "setup", "flow"]
  },
  
  configuration: {
    name: "Configuration Management",
    description: "CRUD operations - backends, listeners, routes",
    pattern: "cypress/e2e/configuration/**/*.cy.ts",
    estimatedDuration: "8-10 minutes", 
    resourceRequirement: "medium",
    tests: ["backends-crud.cy.ts", "listeners-crud.cy.ts", "routes-crud.cy.ts"],
    priority: "high",
    dependencies: ["foundation"],
    tags: ["crud", "configuration", "management"]
  },
  
  playground: {
    name: "Protocol Testing",
    description: "Protocol testing - HTTP, MCP, A2A functionality",
    pattern: "cypress/e2e/playground/**/*.cy.ts",
    estimatedDuration: "10-12 minutes",
    resourceRequirement: "high",
    tests: ["http-testing.cy.ts", "mcp-testing.cy.ts", "a2a-testing.cy.ts"],
    priority: "medium",
    dependencies: ["foundation", "configuration"],
    tags: ["protocol", "http", "mcp", "a2a"]
  },
  
  integration: {
    name: "Integration Tests",
    description: "End-to-end workflows - configuration persistence",
    pattern: "cypress/e2e/integration/**/*.cy.ts",
    estimatedDuration: "6-8 minutes",
    resourceRequirement: "medium",
    tests: ["configuration-persistence.cy.ts", "end-to-end-configuration.cy.ts"],
    priority: "medium",
    dependencies: ["configuration"],
    tags: ["integration", "persistence", "workflow"]
  },
  
  "error-handling": {
    name: "Error Handling",
    description: "Error scenarios and recovery testing",
    pattern: "cypress/e2e/error-handling/**/*.cy.ts",
    estimatedDuration: "4-6 minutes",
    resourceRequirement: "low",
    tests: ["connection-errors.cy.ts", "form-validation.cy.ts"],
    priority: "medium",
    dependencies: ["foundation"],
    tags: ["error", "validation", "recovery"]
  },
  
  navigation: {
    name: "Navigation Tests",
    description: "Navigation testing - deep linking and sidebar",
    pattern: "cypress/e2e/navigation/**/*.cy.ts",
    estimatedDuration: "3-5 minutes",
    resourceRequirement: "low", 
    tests: ["deep-linking.cy.ts", "sidebar-navigation.cy.ts"],
    priority: "low",
    dependencies: ["foundation"],
    tags: ["navigation", "routing", "ui"]
  }
};

// Preset combinations for common use cases
const presets = {
  "quick-validation": {
    name: "Quick Validation",
    description: "Essential functionality check - fastest validation",
    journeys: ["smoke", "foundation"],
    estimatedDuration: "5-10 minutes",
    resourceRequirement: "minimal",
    useCase: "Pull request validation, quick feedback",
    tags: ["quick", "essential", "pr"]
  },
  
  "feature-testing": {
    name: "Feature Testing", 
    description: "Core feature validation - comprehensive feature check",
    journeys: ["setup-wizard", "configuration", "playground"],
    estimatedDuration: "15-25 minutes",
    resourceRequirement: "medium",
    useCase: "Feature development, integration testing",
    tags: ["feature", "comprehensive", "development"]
  },
  
  "comprehensive": {
    name: "Comprehensive Testing",
    description: "Complete test suite - full validation",
    journeys: ["smoke", "foundation", "setup-wizard", "configuration", "playground", "integration", "error-handling", "navigation"],
    estimatedDuration: "30-45 minutes",
    resourceRequirement: "high",
    useCase: "Release validation, complete regression testing",
    tags: ["complete", "release", "regression"]
  },
  
  "core-functionality": {
    name: "Core Functionality",
    description: "Essential features without protocols - core app testing",
    journeys: ["smoke", "foundation", "setup-wizard", "configuration"],
    estimatedDuration: "18-24 minutes",
    resourceRequirement: "medium",
    useCase: "Core feature validation, UI testing",
    tags: ["core", "ui", "essential"]
  },
  
  "protocol-focus": {
    name: "Protocol Focus",
    description: "Protocol and integration testing - communication validation",
    journeys: ["smoke", "playground", "integration"],
    estimatedDuration: "18-23 minutes",
    resourceRequirement: "high",
    useCase: "Protocol development, API testing",
    tags: ["protocol", "api", "integration"]
  }
};

// Resource requirement mappings
const resourceLevels = {
  minimal: {
    workers: 1,
    memoryLimit: 60,
    description: "Single worker, conservative memory usage"
  },
  low: {
    workers: 2,
    memoryLimit: 65,
    description: "Two workers, low memory usage"
  },
  medium: {
    workers: 3,
    memoryLimit: 70,
    description: "Three workers, moderate memory usage"
  },
  high: {
    workers: 4,
    memoryLimit: 80,
    description: "Four workers, higher memory usage"
  }
};

// Environment-specific optimizations
const environmentProfiles = {
  "ci-optimized": {
    name: "CI Optimized",
    description: "Conservative settings for reliable CI execution",
    workerMultiplier: 0.75,
    memoryReduction: 10,
    timeoutMultiplier: 1.5,
    retryCount: 2
  },
  
  "standard": {
    name: "Standard",
    description: "Balanced settings for general use",
    workerMultiplier: 1.0,
    memoryReduction: 0,
    timeoutMultiplier: 1.0,
    retryCount: 1
  },
  
  "high-performance": {
    name: "High Performance",
    description: "Aggressive settings for powerful systems",
    workerMultiplier: 1.25,
    memoryReduction: -5,
    timeoutMultiplier: 0.8,
    retryCount: 0
  },
  
  "conservative": {
    name: "Conservative",
    description: "Ultra-safe settings for minimal systems",
    workerMultiplier: 0.5,
    memoryReduction: 15,
    timeoutMultiplier: 2.0,
    retryCount: 3
  }
};

// Utility functions for journey management
const utils = {
  /**
   * Get all journey names
   */
  getAllJourneyNames() {
    return Object.keys(journeys);
  },
  
  /**
   * Get all preset names
   */
  getAllPresetNames() {
    return Object.keys(presets);
  },
  
  /**
   * Validate journey selection
   */
  validateJourneySelection(selection) {
    const validJourneys = this.getAllJourneyNames();
    const validPresets = this.getAllPresetNames();
    const validSelections = ['all', ...validJourneys, ...validPresets, 'custom'];
    
    return validSelections.includes(selection);
  },
  
  /**
   * Resolve journey list from selection
   */
  resolveJourneyList(selection, customJourneys = '') {
    if (selection === 'all') {
      return this.getAllJourneyNames();
    }
    
    if (selection === 'custom') {
      return customJourneys.split(',').map(j => j.trim()).filter(Boolean);
    }
    
    if (presets[selection]) {
      return presets[selection].journeys;
    }
    
    if (journeys[selection]) {
      return [selection];
    }
    
    throw new Error(`Unable to resolve journey selection: ${selection}`);
  },
  
  /**
   * Calculate total estimated duration
   */
  calculateDuration(journeyList) {
    let totalMinutes = 0;
    
    journeyList.forEach(journeyName => {
      const journey = journeys[journeyName];
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
  },
  
  /**
   * Calculate resource requirements
   */
  calculateResourceRequirements(journeyList) {
    const maxRequirement = journeyList.reduce((max, journeyName) => {
      const journey = journeys[journeyName];
      const requirement = journey?.resourceRequirement || 'low';
      
      const levels = { minimal: 1, low: 2, medium: 3, high: 4 };
      return Math.max(max, levels[requirement] || 2);
    }, 1);
    
    const requirementNames = ['minimal', 'low', 'medium', 'high'];
    const requirementName = requirementNames[maxRequirement - 1] || 'low';
    
    return {
      level: requirementName,
      ...resourceLevels[requirementName]
    };
  },
  
  /**
   * Generate test patterns for Cypress
   */
  generateTestPatterns(journeyList) {
    return journeyList.map(journeyName => {
      const journey = journeys[journeyName];
      return journey ? journey.pattern : null;
    }).filter(Boolean);
  },
  
  /**
   * Check journey dependencies
   */
  checkDependencies(journeyList) {
    const missing = [];
    
    journeyList.forEach(journeyName => {
      const journey = journeys[journeyName];
      if (journey && journey.dependencies) {
        journey.dependencies.forEach(dep => {
          if (!journeyList.includes(dep)) {
            missing.push({ journey: journeyName, missingDependency: dep });
          }
        });
      }
    });
    
    return missing;
  },
  
  /**
   * Get journey information
   */
  getJourneyInfo(journeyName) {
    return journeys[journeyName] || null;
  },
  
  /**
   * Get preset information
   */
  getPresetInfo(presetName) {
    return presets[presetName] || null;
  },
  
  /**
   * Apply environment profile optimizations
   */
  applyEnvironmentProfile(baseResources, profileName) {
    const profile = environmentProfiles[profileName];
    if (!profile) {
      return baseResources;
    }
    
    return {
      ...baseResources,
      workers: Math.max(1, Math.round(baseResources.workers * profile.workerMultiplier)),
      memoryLimit: Math.max(50, Math.min(90, baseResources.memoryLimit - profile.memoryReduction)),
      timeoutMultiplier: profile.timeoutMultiplier,
      retryCount: profile.retryCount
    };
  }
};

// Export configuration
module.exports = {
  journeys,
  presets,
  resourceLevels,
  environmentProfiles,
  utils
};

// For ES6 modules compatibility
if (typeof exports !== 'undefined') {
  exports.journeys = journeys;
  exports.presets = presets;
  exports.resourceLevels = resourceLevels;
  exports.environmentProfiles = environmentProfiles;
  exports.utils = utils;
}
