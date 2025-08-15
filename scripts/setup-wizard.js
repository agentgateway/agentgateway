#!/usr/bin/env node

/**
 * Interactive Setup Wizard for AgentGateway E2E Testing
 * 
 * Provides a step-by-step guided setup process for new developers,
 * with environment-specific recommendations and configuration explanations.
 * 
 * Features:
 * - Interactive prompts for setup preferences
 * - Environment-specific recommendations
 * - Configuration explanation and rationale
 * - Setup validation and testing
 * - Integration with existing setup infrastructure
 */

const fs = require('fs');
const path = require('path');
const readline = require('readline');
const { execSync } = require('child_process');

// Import existing systems
const SmartDefaultsSystem = require('./smart-defaults-system.js');

class SetupWizard {
    constructor() {
        this.rl = readline.createInterface({
            input: process.stdin,
            output: process.stdout
        });
        
        // Initialize working directory detection and correction
        this.initializeWorkingDirectory();
        
        this.smartDefaults = new SmartDefaultsSystem();
        this.setupConfig = {
            preferences: {},
            environment: {},
            systemInfo: {},
            selectedOptions: {}
        };
        
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
     * Working Directory Detection and Correction Pattern
     * 
     * This pattern automatically detects the current working directory and
     * adjusts all paths to work correctly regardless of where the script is run from.
     * 
     * Supports running from:
     * - Project root: /path/to/agentgateway
     * - UI directory: /path/to/agentgateway/ui
     * - Scripts directory: /path/to/agentgateway/scripts
     * 
     * Pattern for reuse in other scripts:
     * 1. Detect current working directory
     * 2. Find project root by looking for key files (Cargo.toml, package.json)
     * 3. Set up path helpers that work from any location
     * 4. Use path helpers consistently throughout the script
     */
    initializeWorkingDirectory() {
        const currentDir = process.cwd();
        const scriptDir = path.dirname(__filename);
        
        // Detect project root by looking for key files
        this.projectRoot = this.findProjectRoot(currentDir);
        
        if (!this.projectRoot) {
            // Fallback: use script directory to find project root
            this.projectRoot = this.findProjectRoot(path.dirname(scriptDir));
        }
        
        if (!this.projectRoot) {
            throw new Error('Could not find AgentGateway project root. Please run from project directory.');
        }
        
        // Set up path helpers
        this.paths = {
            root: this.projectRoot,
            scripts: path.join(this.projectRoot, 'scripts'),
            ui: path.join(this.projectRoot, 'ui'),
            target: path.join(this.projectRoot, 'target'),
            
            // Helper methods for consistent path resolution
            toRoot: (relativePath) => path.join(this.projectRoot, relativePath),
            toScripts: (relativePath) => path.join(this.projectRoot, 'scripts', relativePath),
            toUI: (relativePath) => path.join(this.projectRoot, 'ui', relativePath),
            
            // Relative path helpers based on current working directory
            relativeToRoot: (targetPath) => path.relative(currentDir, path.join(this.projectRoot, targetPath)),
            relativeToScripts: (targetPath) => path.relative(currentDir, path.join(this.projectRoot, 'scripts', targetPath)),
            relativeToUI: (targetPath) => path.relative(currentDir, path.join(this.projectRoot, 'ui', targetPath))
        };
        
        console.log(`🔍 Working directory detection:`);
        console.log(`   Current: ${currentDir}`);
        console.log(`   Project root: ${this.projectRoot}`);
        console.log(`   Running from: ${this.getRunningLocation()}`);
    }
    
    /**
     * Find project root by looking for key indicator files
     */
    findProjectRoot(startDir) {
        const indicators = ['Cargo.toml', 'rust-toolchain.toml', '.gitignore'];
        let currentDir = startDir;
        
        while (currentDir !== path.dirname(currentDir)) {
            // Check if this directory contains project indicators
            const hasIndicators = indicators.some(indicator => 
                fs.existsSync(path.join(currentDir, indicator))
            );
            
            // Additional check: ensure it's the AgentGateway project
            const cargoToml = path.join(currentDir, 'Cargo.toml');
            if (hasIndicators && fs.existsSync(cargoToml)) {
                const cargoContent = fs.readFileSync(cargoToml, 'utf8');
                if (cargoContent.includes('agentgateway') || cargoContent.includes('workspace')) {
                    return currentDir;
                }
            }
            
            currentDir = path.dirname(currentDir);
        }
        
        return null;
    }
    
    /**
     * Get human-readable description of where script is running from
     */
    getRunningLocation() {
        const currentDir = process.cwd();
        const relativePath = path.relative(this.projectRoot, currentDir);
        
        if (relativePath === '') return 'project root';
        if (relativePath === 'ui') return 'ui directory';
        if (relativePath === 'scripts') return 'scripts directory';
        return `${relativePath} directory`;
    }

    /**
     * Main wizard entry point
     */
    async runWizard() {
        try {
            this.displayWelcome();
            
            // Step 1: Gather user preferences
            await this.gatherUserPreferences();
            
            // Step 2: Analyze system and environment
            await this.analyzeSystemEnvironment();
            
            // Step 3: Present recommendations
            await this.presentRecommendations();
            
            // Step 4: Configure setup options
            await this.configureSetupOptions();
            
            // Step 5: Execute setup
            await this.executeSetup();
            
            // Step 6: Validate and test
            await this.validateSetup();
            
            // Step 7: Provide next steps
            this.provideNextSteps();
            
        } catch (error) {
            this.displayError('Setup wizard encountered an error:', error.message);
            process.exit(1);
        } finally {
            this.rl.close();
        }
    }

    /**
     * Display welcome message and introduction
     */
    displayWelcome() {
        console.log(`
${this.colors.cyan}╔══════════════════════════════════════════════════════════════╗
║                                                              ║
║           🚀 AgentGateway E2E Testing Setup Wizard          ║
║                                                              ║
║  Welcome! This wizard will guide you through setting up     ║
║  the E2E testing environment for AgentGateway.              ║
║                                                              ║
║  We'll help you:                                             ║
║  • Configure optimal settings for your system               ║
║  • Install required dependencies                             ║
║  • Set up the testing environment                           ║
║  • Validate everything works correctly                      ║
║                                                              ║
╚══════════════════════════════════════════════════════════════╝${this.colors.reset}

${this.colors.yellow}📋 This process typically takes 5-10 minutes.${this.colors.reset}
${this.colors.green}💡 You can exit anytime with Ctrl+C${this.colors.reset}
`);
    }

    /**
     * Gather user preferences and experience level
     */
    async gatherUserPreferences() {
        console.log(`\n${this.colors.bright}Step 1: Tell us about yourself${this.colors.reset}\n`);
        
        // Experience level
        const experienceLevel = await this.askMultipleChoice(
            'What\'s your experience level with E2E testing?',
            [
                'New to E2E testing - I need guidance',
                'Some experience - I know the basics',
                'Experienced - I want optimal performance',
                'Expert - Just give me the fastest setup'
            ]
        );
        
        this.setupConfig.preferences.experienceLevel = experienceLevel;
        
        // Primary use case
        const useCase = await this.askMultipleChoice(
            'What\'s your primary use case?',
            [
                'Contributing to AgentGateway development',
                'Testing my own AgentGateway integration',
                'Learning how AgentGateway works',
                'Running tests in CI/CD pipeline'
            ]
        );
        
        this.setupConfig.preferences.useCase = useCase;
        
        // Performance vs stability preference
        const performancePreference = await this.askMultipleChoice(
            'What\'s more important to you?',
            [
                'Stability - I want tests that always work',
                'Balanced - Good performance with reliability',
                'Performance - I want the fastest test execution'
            ]
        );
        
        this.setupConfig.preferences.performancePreference = performancePreference;
        
        // Development environment
        const devEnvironment = await this.askMultipleChoice(
            'What\'s your development environment?',
            [
                'Local development machine',
                'Remote development server',
                'Docker container',
                'CI/CD environment',
                'WSL (Windows Subsystem for Linux)'
            ]
        );
        
        this.setupConfig.preferences.devEnvironment = devEnvironment;
        
        console.log(`\n${this.colors.green}✅ Preferences recorded!${this.colors.reset}`);
    }

    /**
     * Analyze system and environment
     */
    async analyzeSystemEnvironment() {
        console.log(`\n${this.colors.bright}Step 2: Analyzing your system...${this.colors.reset}\n`);
        
        this.displayProgress('Detecting environment and system capabilities...');
        
        try {
            // Use smart defaults system for analysis
            const analysis = await this.smartDefaults.generateSmartDefaults({
                prefer_stability: this.setupConfig.preferences.performancePreference === 0,
                prefer_speed: this.setupConfig.preferences.performancePreference === 2
            });
            
            this.setupConfig.environment = analysis.environment;
            this.setupConfig.systemInfo = analysis.systemInfo;
            this.setupConfig.recommendations = analysis.recommendations;
            
            console.log(`${this.colors.green}✅ System analysis complete!${this.colors.reset}`);
            
            // Display key findings
            console.log(`\n${this.colors.cyan}🔍 System Analysis Results:${this.colors.reset}`);
            console.log(`   Environment: ${analysis.environment.profile} (${analysis.environment.confidence}% confidence)`);
            console.log(`   System: ${analysis.systemInfo.cpu_cores} cores, ${analysis.systemInfo.memory_gb}GB RAM`);
            console.log(`   Load: ${analysis.systemInfo.load_1m.toFixed(2)} (1-minute average)`);
            
            if (analysis.recommendations.length > 0) {
                console.log(`\n${this.colors.yellow}💡 Recommendations:${this.colors.reset}`);
                analysis.recommendations.forEach(rec => {
                    console.log(`   ${rec.type.toUpperCase()}: ${rec.message}`);
                });
            }
            
        } catch (error) {
            console.log(`${this.colors.yellow}⚠️  System analysis had issues, using conservative approach${this.colors.reset}`);
            this.setupConfig.environment = { type: 'unknown', profile: 'conservative' };
            this.setupConfig.systemInfo = { cpu_cores: 2, memory_gb: 4 };
            this.setupConfig.recommendations = [];
        }
    }

    /**
     * Present recommendations based on analysis
     */
    async presentRecommendations() {
        console.log(`\n${this.colors.bright}Step 3: Personalized recommendations${this.colors.reset}\n`);
        
        const recommendations = this.generatePersonalizedRecommendations();
        
        console.log(`${this.colors.cyan}📊 Based on your preferences and system analysis:${this.colors.reset}\n`);
        
        recommendations.forEach((rec, index) => {
            console.log(`${this.colors.bright}${index + 1}. ${rec.title}${this.colors.reset}`);
            console.log(`   ${rec.description}`);
            console.log(`   ${this.colors.green}✓${this.colors.reset} ${rec.benefit}\n`);
        });
        
        const proceed = await this.askYesNo('Do these recommendations look good to you?', true);
        
        if (!proceed) {
            console.log(`\n${this.colors.yellow}Let's customize your setup...${this.colors.reset}`);
            await this.customizeRecommendations();
        }
    }

    /**
     * Generate personalized recommendations
     */
    generatePersonalizedRecommendations() {
        const recommendations = [];
        const { experienceLevel, performancePreference, useCase } = this.setupConfig.preferences;
        const { environment, systemInfo } = this.setupConfig;
        
        // Worker count recommendation
        let workerCount = 2; // Conservative default
        if (systemInfo.cpu_cores >= 8 && performancePreference === 2) {
            workerCount = Math.min(6, systemInfo.cpu_cores);
        } else if (systemInfo.cpu_cores >= 4 && performancePreference === 1) {
            workerCount = 4;
        }
        
        recommendations.push({
            title: `Use ${workerCount} parallel workers`,
            description: `Based on your ${systemInfo.cpu_cores}-core system and ${performancePreference === 2 ? 'performance' : performancePreference === 1 ? 'balanced' : 'stability'} preference`,
            benefit: `Optimal balance of speed and reliability for your system`
        });
        
        // Memory allocation recommendation
        const memoryMB = Math.min(
            Math.floor(systemInfo.memory_gb * 1024 * 0.6),
            workerCount * 1024
        );
        
        recommendations.push({
            title: `Allocate ${memoryMB}MB memory for tests`,
            description: `Conservative allocation from your ${systemInfo.memory_gb}GB total RAM`,
            benefit: `Prevents system slowdown while ensuring test reliability`
        });
        
        // Browser mode recommendation
        const headless = experienceLevel === 0 || environment.type === 'ci';
        recommendations.push({
            title: `Run tests in ${headless ? 'headless' : 'headed'} mode`,
            description: `${headless ? 'Background execution for faster, more reliable tests' : 'Visible browser for learning and debugging'}`,
            benefit: `${headless ? 'Better for automated testing and CI/CD' : 'Easier to understand what tests are doing'}`
        });
        
        // Setup approach recommendation
        if (experienceLevel === 0) {
            recommendations.push({
                title: 'Full automated setup with explanations',
                description: 'Install all dependencies and configure everything automatically',
                benefit: 'Get up and running quickly with detailed explanations'
            });
        } else if (experienceLevel === 3) {
            recommendations.push({
                title: 'Minimal setup with performance optimizations',
                description: 'Skip explanations, focus on optimal configuration',
                benefit: 'Fastest setup for experienced developers'
            });
        }
        
        return recommendations;
    }

    /**
     * Configure setup options based on recommendations
     */
    async configureSetupOptions() {
        console.log(`\n${this.colors.bright}Step 4: Configure setup options${this.colors.reset}\n`);
        
        // Dependency installation
        const installDeps = await this.askYesNo(
            'Should we automatically install missing dependencies (Rust, Node.js)?',
            true
        );
        this.setupConfig.selectedOptions.installDependencies = installDeps;
        
        // Build AgentGateway
        const buildGateway = await this.askYesNo(
            'Should we build the AgentGateway binary?',
            true
        );
        this.setupConfig.selectedOptions.buildGateway = buildGateway;
        
        // Run health checks
        const runHealthChecks = await this.askYesNo(
            'Should we run comprehensive health checks?',
            this.setupConfig.preferences.experienceLevel <= 1
        );
        this.setupConfig.selectedOptions.runHealthChecks = runHealthChecks;
        
        // Create test run
        const runTestDemo = await this.askYesNo(
            'Should we run a quick test to verify everything works?',
            true
        );
        this.setupConfig.selectedOptions.runTestDemo = runTestDemo;
        
        // Save configuration
        const saveConfig = await this.askYesNo(
            'Should we save this configuration for future use?',
            true
        );
        this.setupConfig.selectedOptions.saveConfiguration = saveConfig;
        
        console.log(`\n${this.colors.green}✅ Configuration complete!${this.colors.reset}`);
    }

    /**
     * Execute the setup process
     */
    async executeSetup() {
        console.log(`\n${this.colors.bright}Step 5: Setting up your environment...${this.colors.reset}\n`);
        
        const steps = this.getSetupSteps();
        let currentStep = 1;
        
        for (const step of steps) {
            console.log(`\n${this.colors.cyan}[${currentStep}/${steps.length}] ${step.name}${this.colors.reset}`);
            
            if (step.description) {
                console.log(`${this.colors.yellow}💡 ${step.description}${this.colors.reset}`);
            }
            
            try {
                await step.execute();
                console.log(`${this.colors.green}✅ ${step.name} completed${this.colors.reset}`);
            } catch (error) {
                console.log(`${this.colors.red}❌ ${step.name} failed: ${error.message}${this.colors.reset}`);
                
                if (step.critical) {
                    const retry = await this.askYesNo(`This step is critical. Would you like to retry?`, true);
                    if (retry) {
                        try {
                            await step.execute();
                            console.log(`${this.colors.green}✅ ${step.name} completed on retry${this.colors.reset}`);
                        } catch (retryError) {
                            console.log(`${this.colors.red}❌ Retry failed: ${retryError.message}${this.colors.reset}`);
                            throw new Error(`Critical step failed: ${step.name}`);
                        }
                    } else {
                        throw new Error(`Setup aborted at critical step: ${step.name}`);
                    }
                } else {
                    console.log(`${this.colors.yellow}⚠️  Continuing with non-critical step failure${this.colors.reset}`);
                }
            }
            
            currentStep++;
        }
        
        console.log(`\n${this.colors.green}🎉 Setup execution complete!${this.colors.reset}`);
    }

    /**
     * Get setup steps based on configuration
     */
    getSetupSteps() {
        const steps = [];
        
        // Generate smart defaults
        steps.push({
            name: 'Generate smart defaults configuration',
            description: 'Creating optimal test configuration for your system',
            critical: true,
            execute: async () => {
                await this.smartDefaults.generateSmartDefaults({
                    prefer_stability: this.setupConfig.preferences.performancePreference === 0,
                    prefer_speed: this.setupConfig.preferences.performancePreference === 2
                });
            }
        });
        
        // Install dependencies
        if (this.setupConfig.selectedOptions.installDependencies) {
            steps.push({
                name: 'Install system dependencies',
                description: 'Installing Rust, Node.js, and other required tools',
                critical: true,
                execute: async () => {
                    const setupScript = this.paths.relativeToScripts('setup-first-time.sh');
                    await this.executeCommand(`${setupScript} --skip-build --skip-resource-check`);
                }
            });
        }
        
        // Build AgentGateway
        if (this.setupConfig.selectedOptions.buildGateway) {
            steps.push({
                name: 'Build AgentGateway binary',
                description: 'Compiling the AgentGateway proxy server',
                critical: true,
                execute: async () => {
                    // Change to project root for cargo build
                    process.chdir(this.projectRoot);
                    await this.executeCommand('cargo build --release --bin agentgateway');
                }
            });
        }
        
        // Setup UI dependencies
        steps.push({
            name: 'Setup UI dependencies',
            description: 'Installing Node.js packages for the UI',
            critical: true,
            execute: async () => {
                // Change to UI directory for npm install
                process.chdir(this.paths.ui);
                await this.executeCommand('npm install');
            }
        });
        
        // Run health checks
        if (this.setupConfig.selectedOptions.runHealthChecks) {
            steps.push({
                name: 'Run health checks',
                description: 'Validating system configuration and dependencies',
                critical: false,
                execute: async () => {
                    const healthScript = this.paths.relativeToScripts('health-check-validator.js');
                    await this.executeCommand(`node ${healthScript} --verbose`);
                }
            });
        }
        
        // Save configuration
        if (this.setupConfig.selectedOptions.saveConfiguration) {
            steps.push({
                name: 'Save configuration',
                description: 'Saving your preferences for future use',
                critical: false,
                execute: async () => {
                    await this.saveWizardConfiguration();
                }
            });
        }
        
        return steps;
    }

    /**
     * Validate the setup
     */
    async validateSetup() {
        console.log(`\n${this.colors.bright}Step 6: Validating setup...${this.colors.reset}\n`);
        
        const validations = [
            {
                name: 'AgentGateway binary',
                check: () => this.checkAgentGatewayBinary()
            },
            {
                name: 'UI dependencies',
                check: () => this.checkUIDependencies()
            },
            {
                name: 'Test configuration',
                check: () => this.checkTestConfiguration()
            }
        ];
        
        let allValid = true;
        
        for (const validation of validations) {
            try {
                const result = await validation.check();
                if (result) {
                    console.log(`${this.colors.green}✅ ${validation.name} - OK${this.colors.reset}`);
                } else {
                    console.log(`${this.colors.yellow}⚠️  ${validation.name} - Warning${this.colors.reset}`);
                }
            } catch (error) {
                console.log(`${this.colors.red}❌ ${validation.name} - Failed: ${error.message}${this.colors.reset}`);
                allValid = false;
            }
        }
        
        if (this.setupConfig.selectedOptions.runTestDemo && allValid) {
            console.log(`\n${this.colors.cyan}🧪 Running quick test demo...${this.colors.reset}`);
            
            try {
                await this.executeCommand('node ../scripts/test-e2e-minimal.js --resource-only');
                console.log(`${this.colors.green}✅ Quick test demo completed${this.colors.reset}`);
            } catch (error) {
                console.log(`${this.colors.yellow}⚠️  Test demo failed, but setup is complete${this.colors.reset}`);
            }
        }
        
        return allValid;
    }

    /**
     * Provide next steps and usage guidance
     */
    provideNextSteps() {
        console.log(`\n${this.colors.bright}🎉 Setup Complete!${this.colors.reset}\n`);
        
        console.log(`${this.colors.cyan}🚀 You're ready to run E2E tests! Here's what you can do:${this.colors.reset}\n`);
        
        // Basic usage
        console.log(`${this.colors.bright}Basic Usage:${this.colors.reset}`);
        console.log(`   npm run test:e2e:smart              # Run tests with smart defaults`);
        console.log(`   npm run test:e2e:smart:headed       # Run with visible browser`);
        console.log(`   npm run test:e2e:smart:sequential   # Run tests one at a time`);
        
        // Advanced usage
        if (this.setupConfig.preferences.experienceLevel >= 2) {
            console.log(`\n${this.colors.bright}Advanced Usage:${this.colors.reset}`);
            console.log(`   ./scripts/run-e2e-tests.sh --workers 6 --verbose`);
            console.log(`   npm run test:e2e:smart-defaults:template  # Customize settings`);
            console.log(`   npm run test:e2e:health-check             # Check system health`);
        }
        
        // Learning resources
        if (this.setupConfig.preferences.experienceLevel === 0) {
            console.log(`\n${this.colors.bright}Learning Resources:${this.colors.reset}`);
            console.log(`   📖 E2E Testing Guide: ui/cypress/README.md`);
            console.log(`   🔧 Troubleshooting: E2E_TESTING_FIXES.md`);
            console.log(`   💡 Run with --headed to see what tests do`);
        }
        
        // Configuration info
        console.log(`\n${this.colors.bright}Your Configuration:${this.colors.reset}`);
        console.log(`   Environment: ${this.setupConfig.environment.profile || 'detected automatically'}`);
        console.log(`   Performance: ${['Stability-focused', 'Balanced', 'Performance-focused'][this.setupConfig.preferences.performancePreference]}`);
        console.log(`   Experience: ${['Beginner', 'Intermediate', 'Advanced', 'Expert'][this.setupConfig.preferences.experienceLevel]}`);
        
        // Support info
        console.log(`\n${this.colors.bright}Need Help?${this.colors.reset}`);
        console.log(`   🐛 Issues: https://github.com/agentgateway/agentgateway/issues`);
        console.log(`   📚 Docs: Check README.md and documentation files`);
        console.log(`   🔄 Re-run wizard: node scripts/setup-wizard.js`);
        
        console.log(`\n${this.colors.green}Happy testing! 🎯${this.colors.reset}\n`);
    }

    // Helper methods
    
    async askQuestion(question) {
        return new Promise((resolve) => {
            this.rl.question(`${this.colors.cyan}❓ ${question}${this.colors.reset} `, resolve);
        });
    }
    
    async askYesNo(question, defaultValue = true) {
        const defaultText = defaultValue ? 'Y/n' : 'y/N';
        const answer = await this.askQuestion(`${question} (${defaultText}): `);
        
        if (answer.toLowerCase() === '') return defaultValue;
        return answer.toLowerCase().startsWith('y');
    }
    
    async askMultipleChoice(question, options) {
        console.log(`${this.colors.cyan}❓ ${question}${this.colors.reset}`);
        
        options.forEach((option, index) => {
            console.log(`   ${this.colors.bright}${index + 1}.${this.colors.reset} ${option}`);
        });
        
        while (true) {
            const answer = await this.askQuestion('Enter your choice (1-' + options.length + '): ');
            const choice = parseInt(answer) - 1;
            
            if (choice >= 0 && choice < options.length) {
                return choice;
            }
            
            console.log(`${this.colors.red}Please enter a number between 1 and ${options.length}${this.colors.reset}`);
        }
    }
    
    displayProgress(message) {
        console.log(`${this.colors.blue}⏳ ${message}${this.colors.reset}`);
    }
    
    displayError(message, details) {
        console.log(`${this.colors.red}❌ ${message}${this.colors.reset}`);
        if (details) {
            console.log(`   ${details}`);
        }
    }
    
    async executeCommand(command) {
        return new Promise((resolve, reject) => {
            try {
                const result = execSync(command, { 
                    encoding: 'utf8',
                    stdio: this.setupConfig.preferences.experienceLevel === 0 ? 'inherit' : 'pipe'
                });
                resolve(result);
            } catch (error) {
                reject(error);
            }
        });
    }
    
    async checkAgentGatewayBinary() {
        const binaryPaths = [
            this.paths.toRoot('target/release/agentgateway'),
            this.paths.toRoot('target/debug/agentgateway')
        ];
        
        for (const binaryPath of binaryPaths) {
            if (fs.existsSync(binaryPath)) {
                return true;
            }
        }
        
        throw new Error('AgentGateway binary not found');
    }
    
    async checkUIDependencies() {
        return fs.existsSync(this.paths.toUI('node_modules'));
    }
    
    async checkTestConfiguration() {
        return fs.existsSync(this.paths.toRoot('smart-defaults.json')) || 
               fs.existsSync(this.paths.toRoot('test-config-optimized.yaml'));
    }
    
    async saveWizardConfiguration() {
        const configPath = path.join(__dirname, '..', '.wizard-config.json');
        const config = {
            ...this.setupConfig,
            savedAt: new Date().toISOString(),
            version: '1.0'
        };
        
        fs.writeFileSync(configPath, JSON.stringify(config, null, 2));
        console.log(`Configuration saved to ${path.relative(process.cwd(), configPath)}`);
    }
    
    async customizeRecommendations() {
        console.log(`\n${this.colors.yellow}🔧 Let's customize your setup...${this.colors.reset}\n`);
        
        // Allow customization of key settings
        const customWorkers = await this.askQuestion('How many parallel workers would you like? (1-8): ');
        if (customWorkers && !isNaN(customWorkers)) {
            this.setupConfig.customWorkers = Math.max(1, Math.min(8, parseInt(customWorkers)));
        }
        
        const customMemory = await this.askQuestion('Memory limit in MB (1024-8192): ');
        if (customMemory && !isNaN(customMemory)) {
            this.setupConfig.customMemory = Math.max(1024, Math.min(8192, parseInt(customMemory)));
        }
        
        console.log(`${this.colors.green}✅ Customizations applied!${this.colors.reset}`);
    }
}

// CLI interface
async function main() {
    const args = process.argv.slice(2);
    
    if (args.includes('--help') || args.includes('-h')) {
        console.log(`
Interactive Setup Wizard for AgentGateway E2E Testing

Usage:
  node scripts/setup-wizard.js [options]

Options:
  --help, -h           Show this help message
  --quick              Skip detailed explanations (for experienced users)
  --config-only        Only generate configuration, don't install dependencies

Examples:
  node scripts/setup-wizard.js
  node scripts/setup-wizard.js --quick
`);
        process.exit(0);
    }
    
    const wizard = new SetupWizard();
    
    // Handle quick mode
    if (args.includes('--quick')) {
        wizard.setupConfig.preferences.experienceLevel = 3; // Expert level
    }
    
    // Handle config-only mode
    if (args.includes('--config-only')) {
        wizard.setupConfig.selectedOptions.installDependencies = false;
        wizard.setupConfig.selectedOptions.buildGateway = false;
    }
    
    await wizard.runWizard();
}

// Run if called directly
if (require.main === module) {
    main().catch(error => {
        console.error('Setup wizard failed:', error.message);
        process.exit(1);
    });
}

module.exports = SetupWizard;
