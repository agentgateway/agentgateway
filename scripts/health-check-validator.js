#!/usr/bin/env node

/**
 * Health Check Validation System
 * 
 * Comprehensive pre-flight checks for E2E testing environment
 * Validates all dependencies, configurations, and system readiness
 * 
 * Features:
 * - Pre-flight dependency validation
 * - Backend/UI connectivity validation
 * - Resource availability verification
 * - Clear pass/fail reporting with actionable next steps
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync, spawn } = require('child_process');
const http = require('http');
const https = require('https');

class HealthCheckValidator {
    constructor() {
        this.projectRoot = path.join(__dirname, '..');
        this.checks = [];
        this.warnings = [];
        this.errors = [];
        this.verbose = false;
    }

    /**
     * Main validation entry point
     */
    async runHealthChecks(options = {}) {
        this.verbose = options.verbose || false;
        
        console.log('🏥 AgentGateway E2E Testing Health Check');
        console.log('═'.repeat(50));
        console.log('Validating system readiness for E2E testing...\n');

        try {
            // System-level checks
            await this.checkSystemRequirements();
            await this.checkDependencies();
            await this.checkProjectStructure();
            
            // Build and configuration checks
            await this.checkAgentGatewayBinary();
            await this.checkTestConfiguration();
            await this.checkUISetup();
            
            // Runtime checks (if requested)
            if (options.includeRuntime) {
                await this.checkRuntimeConnectivity();
            }
            
            // Resource checks
            await this.checkSystemResources();
            
            // Generate report
            this.generateHealthReport();
            
            return this.getOverallStatus();

        } catch (error) {
            this.addError('health_check_system', `Health check system error: ${error.message}`);
            this.generateHealthReport();
            return false;
        }
    }

    /**
     * Check system requirements
     */
    async checkSystemRequirements() {
        this.log('🖥️  Checking system requirements...');

        // Operating System
        const platform = process.platform;
        const arch = process.arch;
        const release = os.release();
        
        this.addCheck('system_os', 'Operating System', `${platform} ${arch} (${release})`, true);

        // Memory
        const totalMemoryGB = Math.round(os.totalmem() / (1024 * 1024 * 1024) * 10) / 10;
        const freeMemoryGB = Math.round(os.freemem() / (1024 * 1024 * 1024) * 10) / 10;
        
        if (totalMemoryGB < 4) {
            this.addWarning('system_memory', `Low total memory: ${totalMemoryGB}GB (recommend 8GB+)`);
        }
        
        if (freeMemoryGB < 2) {
            this.addWarning('system_memory_free', `Low free memory: ${freeMemoryGB}GB (recommend 4GB+)`);
        }
        
        this.addCheck('system_memory', 'System Memory', `${totalMemoryGB}GB total, ${freeMemoryGB}GB free`, true);

        // CPU
        const cpuCount = os.cpus().length;
        const cpuModel = os.cpus()[0].model;
        
        if (cpuCount < 2) {
            this.addWarning('system_cpu', `Low CPU count: ${cpuCount} cores (recommend 4+ cores)`);
        }
        
        this.addCheck('system_cpu', 'CPU', `${cpuCount} cores (${cpuModel})`, true);

        // Disk space
        try {
            const stats = fs.statSync(this.projectRoot);
            const diskUsage = execSync(`df -h "${this.projectRoot}" | tail -1`, { encoding: 'utf8' });
            const availableSpace = diskUsage.split(/\s+/)[3];
            
            this.addCheck('system_disk', 'Disk Space', `${availableSpace} available`, true);
            
            // Parse available space and warn if low
            const spaceMatch = availableSpace.match(/^(\d+(?:\.\d+)?)([KMGT]?)$/);
            if (spaceMatch) {
                const [, amount, unit] = spaceMatch;
                const amountNum = parseFloat(amount);
                const isLowSpace = (unit === 'M' && amountNum < 2048) || 
                                 (unit === 'G' && amountNum < 2) || 
                                 (unit === '' && amountNum < 2048000);
                
                if (isLowSpace) {
                    this.addWarning('system_disk_space', `Low disk space: ${availableSpace} (recommend 2GB+)`);
                }
            }
        } catch (error) {
            this.addWarning('system_disk', `Could not check disk space: ${error.message}`);
        }
    }

    /**
     * Check required dependencies
     */
    async checkDependencies() {
        this.log('📦 Checking dependencies...');

        // Rust and Cargo
        try {
            const rustVersion = execSync('rustc --version', { encoding: 'utf8' }).trim();
            const cargoVersion = execSync('cargo --version', { encoding: 'utf8' }).trim();
            
            this.addCheck('dep_rust', 'Rust', rustVersion, true);
            this.addCheck('dep_cargo', 'Cargo', cargoVersion, true);
            
            // Check required toolchain
            const toolchainFile = path.join(this.projectRoot, 'rust-toolchain.toml');
            if (fs.existsSync(toolchainFile)) {
                try {
                    const toolchainContent = fs.readFileSync(toolchainFile, 'utf8');
                    const channelMatch = toolchainContent.match(/channel\s*=\s*"([^"]+)"/);
                    if (channelMatch) {
                        const requiredChannel = channelMatch[1];
                        const installedToolchains = execSync('rustup show', { encoding: 'utf8' });
                        
                        if (installedToolchains.includes(requiredChannel)) {
                            this.addCheck('dep_rust_toolchain', 'Rust Toolchain', `${requiredChannel} ✓`, true);
                        } else {
                            this.addError('dep_rust_toolchain', `Required Rust toolchain not installed: ${requiredChannel}`);
                        }
                    }
                } catch (error) {
                    this.addWarning('dep_rust_toolchain', `Could not verify Rust toolchain: ${error.message}`);
                }
            }
        } catch (error) {
            this.addError('dep_rust', 'Rust/Cargo not found - required for building AgentGateway');
        }

        // Node.js and npm
        try {
            const nodeVersion = execSync('node --version', { encoding: 'utf8' }).trim();
            const npmVersion = execSync('npm --version', { encoding: 'utf8' }).trim();
            
            this.addCheck('dep_node', 'Node.js', nodeVersion, true);
            this.addCheck('dep_npm', 'npm', npmVersion, true);
            
            // Check Node.js version (require >= 18)
            const nodeVersionNum = parseInt(nodeVersion.replace('v', '').split('.')[0]);
            if (nodeVersionNum < 18) {
                this.addWarning('dep_node_version', `Node.js version ${nodeVersion} is older than recommended (18+)`);
            }
        } catch (error) {
            this.addError('dep_node', 'Node.js/npm not found - required for UI development');
        }

        // Git
        try {
            const gitVersion = execSync('git --version', { encoding: 'utf8' }).trim();
            this.addCheck('dep_git', 'Git', gitVersion, true);
        } catch (error) {
            this.addWarning('dep_git', 'Git not found - may be needed for some operations');
        }

        // curl (for health checks)
        try {
            const curlVersion = execSync('curl --version', { encoding: 'utf8' }).split('\n')[0];
            this.addCheck('dep_curl', 'curl', curlVersion, true);
        } catch (error) {
            this.addWarning('dep_curl', 'curl not found - may affect connectivity checks');
        }
    }

    /**
     * Check project structure
     */
    async checkProjectStructure() {
        this.log('📁 Checking project structure...');

        const requiredDirs = [
            'ui',
            'scripts',
            'crates/agentgateway',
            'crates/agentgateway/src'
        ];

        const requiredFiles = [
            'Cargo.toml',
            'test-config.yaml',
            'ui/package.json',
            'ui/cypress.config.ts',
            'scripts/run-e2e-tests.sh',
            'scripts/setup-first-time.sh'
        ];

        // Check directories
        for (const dir of requiredDirs) {
            const dirPath = path.join(this.projectRoot, dir);
            if (fs.existsSync(dirPath) && fs.statSync(dirPath).isDirectory()) {
                this.addCheck('structure_dir', `Directory: ${dir}`, 'exists', true);
            } else {
                this.addError('structure_dir', `Required directory missing: ${dir}`);
            }
        }

        // Check files
        for (const file of requiredFiles) {
            const filePath = path.join(this.projectRoot, file);
            if (fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
                this.addCheck('structure_file', `File: ${file}`, 'exists', true);
            } else {
                this.addError('structure_file', `Required file missing: ${file}`);
            }
        }

        // Check script permissions
        const scripts = [
            'scripts/run-e2e-tests.sh',
            'scripts/setup-first-time.sh'
        ];

        for (const script of scripts) {
            const scriptPath = path.join(this.projectRoot, script);
            if (fs.existsSync(scriptPath)) {
                try {
                    fs.accessSync(scriptPath, fs.constants.X_OK);
                    this.addCheck('structure_perms', `Executable: ${script}`, 'executable', true);
                } catch (error) {
                    this.addWarning('structure_perms', `Script not executable: ${script} (run: chmod +x ${script})`);
                }
            }
        }
    }

    /**
     * Check AgentGateway binary
     */
    async checkAgentGatewayBinary() {
        this.log('🦀 Checking AgentGateway binary...');

        const binaryPaths = [
            'target/release/agentgateway',
            'target/debug/agentgateway',
            'target/release/agentgateway-app',
            'target/debug/agentgateway-app'
        ];

        let binaryFound = false;
        let binaryPath = '';

        for (const binPath of binaryPaths) {
            const fullPath = path.join(this.projectRoot, binPath);
            if (fs.existsSync(fullPath)) {
                binaryFound = true;
                binaryPath = binPath;
                
                // Check if it's executable
                try {
                    fs.accessSync(fullPath, fs.constants.X_OK);
                    this.addCheck('agentgateway_binary', 'AgentGateway Binary', `${binPath} (executable)`, true);
                } catch (error) {
                    this.addError('agentgateway_binary', `AgentGateway binary not executable: ${binPath}`);
                }
                break;
            }
        }

        if (!binaryFound) {
            this.addError('agentgateway_binary', 'AgentGateway binary not found - run: cargo build --release --bin agentgateway');
        }

        // Check if we can build if binary is missing
        if (!binaryFound) {
            try {
                // Test if we can at least check the build
                execSync('cargo check --bin agentgateway', { 
                    cwd: this.projectRoot, 
                    stdio: 'pipe',
                    timeout: 30000 
                });
                this.addCheck('agentgateway_build', 'AgentGateway Build Check', 'can build', true);
            } catch (error) {
                this.addError('agentgateway_build', `Cannot build AgentGateway: ${error.message}`);
            }
        }
    }

    /**
     * Check test configuration
     */
    async checkTestConfiguration() {
        this.log('⚙️  Checking test configuration...');

        // Check test-config.yaml
        const testConfigPath = path.join(this.projectRoot, 'test-config.yaml');
        if (fs.existsSync(testConfigPath)) {
            try {
                const configContent = fs.readFileSync(testConfigPath, 'utf8');
                this.addCheck('config_test', 'Test Configuration', 'test-config.yaml exists', true);
                
                // Basic validation of YAML content
                if (configContent.includes('listeners:') && configContent.includes('upstreams:')) {
                    this.addCheck('config_test_content', 'Test Config Content', 'valid structure', true);
                } else {
                    this.addWarning('config_test_content', 'test-config.yaml may be incomplete');
                }
            } catch (error) {
                this.addError('config_test', `Cannot read test-config.yaml: ${error.message}`);
            }
        } else {
            this.addError('config_test', 'test-config.yaml not found');
        }

        // Check for optimized configuration
        const optimizedConfigPath = path.join(this.projectRoot, 'test-config-optimized.yaml');
        if (fs.existsSync(optimizedConfigPath)) {
            this.addCheck('config_optimized', 'Optimized Configuration', 'test-config-optimized.yaml exists', true);
        } else {
            this.addWarning('config_optimized', 'Optimized configuration not found - run: node scripts/intelligent-test-config.js');
        }

        // Check UI test settings
        const uiSettingsPath = path.join(this.projectRoot, 'ui', '.test-settings.json');
        if (fs.existsSync(uiSettingsPath)) {
            try {
                const settings = JSON.parse(fs.readFileSync(uiSettingsPath, 'utf8'));
                this.addCheck('config_ui_settings', 'UI Test Settings', 'persistent settings available', true);
                
                if (settings.workers && settings.memory_limit_mb) {
                    this.addCheck('config_ui_content', 'UI Settings Content', `${settings.workers} workers, ${settings.memory_limit_mb}MB limit`, true);
                }
            } catch (error) {
                this.addWarning('config_ui_settings', `Cannot parse UI test settings: ${error.message}`);
            }
        } else {
            this.addWarning('config_ui_settings', 'UI test settings not found - will use defaults');
        }
    }

    /**
     * Check UI setup
     */
    async checkUISetup() {
        this.log('🎨 Checking UI setup...');

        const uiDir = path.join(this.projectRoot, 'ui');
        
        // Check package.json
        const packageJsonPath = path.join(uiDir, 'package.json');
        if (fs.existsSync(packageJsonPath)) {
            try {
                const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
                this.addCheck('ui_package', 'UI Package.json', 'valid', true);
                
                // Check for required dependencies
                const requiredDeps = ['cypress', 'next', 'react'];
                const missingDeps = requiredDeps.filter(dep => 
                    !packageJson.dependencies?.[dep] && !packageJson.devDependencies?.[dep]
                );
                
                if (missingDeps.length === 0) {
                    this.addCheck('ui_deps_config', 'UI Dependencies Config', 'all required deps in package.json', true);
                } else {
                    this.addError('ui_deps_config', `Missing dependencies in package.json: ${missingDeps.join(', ')}`);
                }
                
                // Check for E2E test scripts
                const requiredScripts = ['e2e', 'test:e2e:parallel', 'test:e2e:optimized'];
                const missingScripts = requiredScripts.filter(script => !packageJson.scripts?.[script]);
                
                if (missingScripts.length === 0) {
                    this.addCheck('ui_scripts', 'UI Test Scripts', 'all E2E scripts available', true);
                } else {
                    this.addWarning('ui_scripts', `Missing npm scripts: ${missingScripts.join(', ')}`);
                }
            } catch (error) {
                this.addError('ui_package', `Cannot parse UI package.json: ${error.message}`);
            }
        } else {
            this.addError('ui_package', 'UI package.json not found');
        }

        // Check node_modules
        const nodeModulesPath = path.join(uiDir, 'node_modules');
        if (fs.existsSync(nodeModulesPath)) {
            this.addCheck('ui_node_modules', 'UI Dependencies', 'node_modules exists', true);
            
            // Check for key dependencies
            const keyDeps = ['cypress', 'next'];
            for (const dep of keyDeps) {
                const depPath = path.join(nodeModulesPath, dep);
                if (fs.existsSync(depPath)) {
                    this.addCheck('ui_dep_installed', `UI Dependency: ${dep}`, 'installed', true);
                } else {
                    this.addError('ui_dep_installed', `UI dependency not installed: ${dep} (run: cd ui && npm install)`);
                }
            }
        } else {
            this.addError('ui_node_modules', 'UI dependencies not installed - run: cd ui && npm install');
        }

        // Check Cypress configuration
        const cypressConfigPath = path.join(uiDir, 'cypress.config.ts');
        if (fs.existsSync(cypressConfigPath)) {
            this.addCheck('ui_cypress_config', 'Cypress Configuration', 'cypress.config.ts exists', true);
        } else {
            this.addError('ui_cypress_config', 'Cypress configuration not found');
        }

        // Check for test files
        const testDir = path.join(uiDir, 'cypress', 'e2e');
        if (fs.existsSync(testDir)) {
            const testFiles = this.findTestFiles(testDir);
            if (testFiles.length > 0) {
                this.addCheck('ui_test_files', 'E2E Test Files', `${testFiles.length} test files found`, true);
            } else {
                this.addWarning('ui_test_files', 'No E2E test files found');
            }
        } else {
            this.addError('ui_test_files', 'E2E test directory not found');
        }
    }

    /**
     * Check runtime connectivity (optional)
     */
    async checkRuntimeConnectivity() {
        this.log('🌐 Checking runtime connectivity...');

        // This is an optional check that requires services to be running
        const backendUrl = 'http://localhost:8080';
        const uiUrl = 'http://localhost:3000';

        try {
            await this.checkUrlConnectivity(backendUrl + '/health', 'AgentGateway Backend');
        } catch (error) {
            this.addWarning('runtime_backend', `Backend not running at ${backendUrl} (this is OK if not started yet)`);
        }

        try {
            await this.checkUrlConnectivity(uiUrl, 'UI Development Server');
        } catch (error) {
            this.addWarning('runtime_ui', `UI server not running at ${uiUrl} (this is OK if not started yet)`);
        }
    }

    /**
     * Check system resources
     */
    async checkSystemResources() {
        this.log('📊 Checking system resources...');

        // Load average
        const loadAvg = os.loadavg();
        const cpuCount = os.cpus().length;
        const loadPercent = Math.round((loadAvg[0] / cpuCount) * 100);

        if (loadPercent > 80) {
            this.addWarning('resource_load', `High system load: ${loadPercent}% (${loadAvg[0].toFixed(2)})`);
        } else {
            this.addCheck('resource_load', 'System Load', `${loadPercent}% (${loadAvg[0].toFixed(2)})`, true);
        }

        // Memory pressure
        const totalMem = os.totalmem();
        const freeMem = os.freemem();
        const usedPercent = Math.round(((totalMem - freeMem) / totalMem) * 100);

        if (usedPercent > 85) {
            this.addWarning('resource_memory', `High memory usage: ${usedPercent}%`);
        } else {
            this.addCheck('resource_memory', 'Memory Usage', `${usedPercent}% used`, true);
        }

        // Check if resource detection script is available
        const resourceScriptPath = path.join(this.projectRoot, 'scripts', 'detect-system-resources.js');
        if (fs.existsSync(resourceScriptPath)) {
            try {
                const resourceOutput = execSync('node scripts/detect-system-resources.js --quiet', {
                    cwd: this.projectRoot,
                    encoding: 'utf8',
                    timeout: 10000
                });
                
                if (resourceOutput && !resourceOutput.includes('error')) {
                    this.addCheck('resource_detection', 'Resource Detection', 'script available and working', true);
                } else {
                    this.addWarning('resource_detection', 'Resource detection script had issues');
                }
            } catch (error) {
                this.addWarning('resource_detection', `Resource detection script failed: ${error.message}`);
            }
        } else {
            this.addWarning('resource_detection', 'Resource detection script not found');
        }
    }

    /**
     * Helper methods
     */
    addCheck(category, name, value, passed) {
        this.checks.push({ category, name, value, passed });
    }

    addWarning(category, message) {
        this.warnings.push({ category, message });
    }

    addError(category, message) {
        this.errors.push({ category, message });
    }

    log(message) {
        if (this.verbose) {
            console.log(message);
        }
    }

    async checkUrlConnectivity(url, serviceName) {
        return new Promise((resolve, reject) => {
            const urlObj = new URL(url);
            const client = urlObj.protocol === 'https:' ? https : http;
            
            const req = client.get(url, { timeout: 5000 }, (res) => {
                if (res.statusCode >= 200 && res.statusCode < 400) {
                    this.addCheck('runtime_connectivity', serviceName, `responding (${res.statusCode})`, true);
                    resolve();
                } else {
                    this.addWarning('runtime_connectivity', `${serviceName} returned ${res.statusCode}`);
                    resolve();
                }
            });

            req.on('error', (error) => {
                reject(error);
            });

            req.on('timeout', () => {
                req.destroy();
                reject(new Error('Request timeout'));
            });
        });
    }

    findTestFiles(dir) {
        const testFiles = [];
        const items = fs.readdirSync(dir);
        
        for (const item of items) {
            const itemPath = path.join(dir, item);
            const stat = fs.statSync(itemPath);
            
            if (stat.isDirectory()) {
                testFiles.push(...this.findTestFiles(itemPath));
            } else if (item.endsWith('.cy.ts') || item.endsWith('.cy.js')) {
                testFiles.push(itemPath);
            }
        }
        
        return testFiles;
    }

    /**
     * Generate health report
     */
    generateHealthReport() {
        console.log('\n📋 Health Check Report');
        console.log('═'.repeat(50));

        // Summary
        const totalChecks = this.checks.length;
        const passedChecks = this.checks.filter(c => c.passed).length;
        const failedChecks = totalChecks - passedChecks;

        console.log(`Total Checks: ${totalChecks}`);
        console.log(`✅ Passed: ${passedChecks}`);
        if (failedChecks > 0) {
            console.log(`❌ Failed: ${failedChecks}`);
        }
        if (this.warnings.length > 0) {
            console.log(`⚠️  Warnings: ${this.warnings.length}`);
        }
        if (this.errors.length > 0) {
            console.log(`🚨 Errors: ${this.errors.length}`);
        }

        // Detailed results
        if (this.verbose || this.errors.length > 0 || this.warnings.length > 0) {
            console.log('\n📊 Detailed Results:');
            
            // Group checks by category
            const categories = {};
            for (const check of this.checks) {
                if (!categories[check.category]) {
                    categories[check.category] = [];
                }
                categories[check.category].push(check);
            }

            for (const [category, checks] of Object.entries(categories)) {
                console.log(`\n${category.toUpperCase()}:`);
                for (const check of checks) {
                    const status = check.passed ? '✅' : '❌';
                    console.log(`  ${status} ${check.name}: ${check.value}`);
                }
            }
        }

        // Warnings
        if (this.warnings.length > 0) {
            console.log('\n⚠️  Warnings:');
            for (const warning of this.warnings) {
                console.log(`  - ${warning.message}`);
            }
        }

        // Errors
        if (this.errors.length > 0) {
            console.log('\n🚨 Errors:');
            for (const error of this.errors) {
                console.log(`  - ${error.message}`);
            }
        }

        // Next steps
        console.log('\n💡 Next Steps:');
        if (this.errors.length > 0) {
            console.log('  1. Fix the errors listed above');
            console.log('  2. Run the health check again: node scripts/health-check-validator.js');
            console.log('  3. Consider running first-time setup: ./scripts/setup-first-time.sh');
        } else if (this.warnings.length > 0) {
            console.log('  1. Review warnings (optional but recommended)');
            console.log('  2. Run E2E tests: ./scripts/run-e2e-tests.sh');
        } else {
            console.log('  ✅ System is ready for E2E testing!');
            console.log('  🚀 Run tests: ./scripts/run-e2e-tests.sh');
        }

        console.log('═'.repeat(50));
    }

    /**
     * Get overall status
     */
    getOverallStatus() {
        return this.errors.length === 0;
    }
}

// CLI interface
async function main() {
    const args = process.argv.slice(2);
    const validator = new HealthCheckValidator();
    
    const options = {
        verbose: args.includes('--verbose') || args.includes('-v'),
        includeRuntime: args.includes('--include-runtime'),
        help: args.includes('--help') || args.includes('-h')
    };
    
    if (options.help) {
        console.log(`
AgentGateway E2E Testing Health Check Validator

Usage:
  node scripts/health-check-validator.js [options]

Options:
  --verbose, -v          Show detailed output
  --include-runtime      Check if services are running (optional)
  --help, -h             Show this help message

Examples:
  node scripts/health-check-validator.js
  node scripts/health-check-validator.js --verbose
  node scripts/health-check-validator.js --include-runtime --verbose

This tool validates:
  ✅ System requirements (memory, CPU, disk space)
  ✅ Dependencies (Rust, Node.js, npm, git)
  ✅ Project structure (directories, files, permissions)
  ✅ AgentGateway binary (build status)
  ✅ Test configuration (YAML files, settings)
  ✅ UI setup (dependencies, Cypress config, test files)
  ✅ System resources (load, memory usage)
  ✅ Runtime connectivity (optional, if --include-runtime)
`);
        process.exit(0);
    }
    
    try {
        const success = await validator.runHealthChecks(options);
        process.exit(success ? 0 : 1);
    } catch (error) {
        console.error('❌ Health check failed:', error.message);
        process.exit(1);
    }
}

// Run if called directly
if (require.main === module) {
    main().catch(error => {
        console.error('Fatal error:', error);
        process.exit(1);
    });
}

module.exports = HealthCheckValidator;
