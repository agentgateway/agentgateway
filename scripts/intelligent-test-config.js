#!/usr/bin/env node

/**
 * Intelligent Test Configuration System
 * 
 * Automatically detects optimal test settings based on system capabilities
 * and creates persistent configuration for future test runs.
 * 
 * Features:
 * - Auto-detection of system resources
 * - Environment-specific optimization (CI vs local vs Docker)
 * - Configuration persistence
 * - Validation and health checks
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync } = require('child_process');

// Import existing resource monitoring
const ResourceMonitor = require('./lib/resource-monitor-fixed.js');

class IntelligentTestConfig {
    constructor() {
        this.configPath = path.join(__dirname, '..', 'test-config-optimized.yaml');
        this.settingsPath = path.join(__dirname, '..', 'ui', '.test-settings.json');
        this.resourceMonitor = new ResourceMonitor();
        
        this.defaultConfig = {
            test_settings: {
                workers: 2,
                memory_limit_mb: 2048,
                timeout_ms: 30000,
                retry_attempts: 2,
                parallel_mode: 'conservative'
            },
            browser_settings: {
                headless: true,
                viewport_width: 1280,
                viewport_height: 720,
                video: false,
                screenshots: true
            },
            resource_limits: {
                max_cpu_percent: 70,
                max_memory_percent: 60,
                emergency_threshold_percent: 85
            },
            environment: {
                type: 'unknown',
                detected_at: new Date().toISOString(),
                system_info: {}
            }
        };
    }

    /**
     * Main entry point - detect environment and create optimal configuration
     */
    async generateOptimalConfig() {
        console.log('🔍 Analyzing system capabilities for optimal test configuration...\n');

        try {
            // Detect environment type
            const environment = this.detectEnvironment();
            console.log(`📊 Environment detected: ${environment.type}`);

            // Get system resources
            const systemInfo = await this.getSystemInfo();
            console.log(`💻 System: ${systemInfo.cpu_cores} cores, ${systemInfo.memory_gb}GB RAM`);

            // Calculate optimal settings
            const optimalSettings = this.calculateOptimalSettings(environment, systemInfo);
            console.log(`⚙️  Optimal workers: ${optimalSettings.workers}, Memory limit: ${optimalSettings.memory_limit_mb}MB`);

            // Create configuration
            const config = this.createConfiguration(environment, systemInfo, optimalSettings);

            // Validate configuration
            const validation = await this.validateConfiguration(config);
            if (!validation.valid) {
                console.log('⚠️  Configuration validation failed, using conservative defaults');
                config.test_settings = { ...this.defaultConfig.test_settings };
            }

            // Save configuration
            await this.saveConfiguration(config);
            await this.saveSettings(optimalSettings);

            // Display summary
            this.displayConfigurationSummary(config);

            return config;

        } catch (error) {
            console.error('❌ Error generating optimal configuration:', error.message);
            console.log('🔄 Falling back to conservative defaults...');
            
            const fallbackConfig = { ...this.defaultConfig };
            await this.saveConfiguration(fallbackConfig);
            return fallbackConfig;
        }
    }

    /**
     * Detect the current environment type
     */
    detectEnvironment() {
        const env = {
            type: 'local',
            ci: false,
            docker: false,
            wsl: false,
            details: {}
        };

        // Check for CI environment
        if (process.env.CI || process.env.GITHUB_ACTIONS || process.env.GITLAB_CI || process.env.JENKINS_URL) {
            env.type = 'ci';
            env.ci = true;
            env.details.ci_provider = process.env.GITHUB_ACTIONS ? 'github' : 
                                    process.env.GITLAB_CI ? 'gitlab' : 
                                    process.env.JENKINS_URL ? 'jenkins' : 'unknown';
        }

        // Check for Docker environment
        if (fs.existsSync('/.dockerenv') || process.env.DOCKER_CONTAINER) {
            env.docker = true;
            if (env.type === 'local') env.type = 'docker';
        }

        // Check for WSL
        if (process.platform === 'linux' && fs.existsSync('/proc/version')) {
            try {
                const version = fs.readFileSync('/proc/version', 'utf8');
                if (version.toLowerCase().includes('microsoft') || version.toLowerCase().includes('wsl')) {
                    env.wsl = true;
                    env.details.wsl_version = version.includes('WSL2') ? '2' : '1';
                }
            } catch (e) {
                // Ignore errors reading /proc/version
            }
        }

        return env;
    }

    /**
     * Get comprehensive system information
     */
    async getSystemInfo() {
        const systemInfo = {
            platform: process.platform,
            arch: process.arch,
            cpu_cores: os.cpus().length,
            memory_gb: Math.round(os.totalmem() / (1024 * 1024 * 1024) * 10) / 10,
            memory_free_gb: Math.round(os.freemem() / (1024 * 1024 * 1024) * 10) / 10,
            load_average: os.loadavg(),
            uptime_hours: Math.round(os.uptime() / 3600 * 10) / 10
        };

        // Get additional system details
        try {
            if (process.platform === 'linux') {
                systemInfo.cpu_model = os.cpus()[0].model;
                
                // Try to get more detailed memory info
                if (fs.existsSync('/proc/meminfo')) {
                    const meminfo = fs.readFileSync('/proc/meminfo', 'utf8');
                    const available = meminfo.match(/MemAvailable:\s+(\d+)\s+kB/);
                    if (available) {
                        systemInfo.memory_available_gb = Math.round(parseInt(available[1]) / 1024 / 1024 * 10) / 10;
                    }
                }
            }

            // Check Node.js memory usage
            const memUsage = process.memoryUsage();
            systemInfo.node_memory_mb = Math.round(memUsage.rss / 1024 / 1024);

        } catch (error) {
            console.log('⚠️  Could not gather extended system information:', error.message);
        }

        return systemInfo;
    }

    /**
     * Calculate optimal settings based on environment and system info
     */
    calculateOptimalSettings(environment, systemInfo) {
        let settings = { ...this.defaultConfig.test_settings };

        // Base calculations on available resources
        const availableMemoryGB = systemInfo.memory_available_gb || systemInfo.memory_free_gb;
        const totalMemoryGB = systemInfo.memory_gb;
        const cpuCores = systemInfo.cpu_cores;

        // Environment-specific adjustments
        switch (environment.type) {
            case 'ci':
                // Conservative settings for CI
                settings.workers = Math.min(2, Math.max(1, Math.floor(cpuCores / 2)));
                settings.memory_limit_mb = Math.min(2048, Math.floor(totalMemoryGB * 1024 * 0.4));
                settings.parallel_mode = 'conservative';
                settings.timeout_ms = 45000; // Longer timeout for CI
                break;

            case 'docker':
                // Docker-optimized settings
                settings.workers = Math.min(3, Math.max(1, Math.floor(cpuCores * 0.6)));
                settings.memory_limit_mb = Math.min(3072, Math.floor(totalMemoryGB * 1024 * 0.5));
                settings.parallel_mode = 'balanced';
                break;

            case 'local':
            default:
                // Optimal settings for local development
                if (totalMemoryGB >= 16) {
                    // High-end system
                    settings.workers = Math.min(6, Math.max(2, Math.floor(cpuCores * 0.75)));
                    settings.memory_limit_mb = Math.min(6144, Math.floor(totalMemoryGB * 1024 * 0.6));
                    settings.parallel_mode = 'aggressive';
                } else if (totalMemoryGB >= 8) {
                    // Mid-range system
                    settings.workers = Math.min(4, Math.max(2, Math.floor(cpuCores * 0.6)));
                    settings.memory_limit_mb = Math.min(4096, Math.floor(totalMemoryGB * 1024 * 0.5));
                    settings.parallel_mode = 'balanced';
                } else {
                    // Low-end system
                    settings.workers = Math.min(2, Math.max(1, Math.floor(cpuCores / 2)));
                    settings.memory_limit_mb = Math.min(2048, Math.floor(totalMemoryGB * 1024 * 0.4));
                    settings.parallel_mode = 'conservative';
                }
                break;
        }

        // Apply resource constraints
        const maxMemoryMB = Math.floor(availableMemoryGB * 1024 * 0.8); // Leave 20% free
        if (settings.memory_limit_mb > maxMemoryMB) {
            settings.memory_limit_mb = maxMemoryMB;
            console.log(`⚠️  Reduced memory limit to ${maxMemoryMB}MB based on available memory`);
        }

        // Ensure minimum viable settings
        settings.workers = Math.max(1, settings.workers);
        settings.memory_limit_mb = Math.max(1024, settings.memory_limit_mb);

        // WSL-specific adjustments
        if (environment.wsl) {
            settings.workers = Math.max(1, Math.floor(settings.workers * 0.8));
            settings.memory_limit_mb = Math.floor(settings.memory_limit_mb * 0.9);
            console.log('🔧 Applied WSL-specific optimizations');
        }

        return settings;
    }

    /**
     * Create complete configuration object
     */
    createConfiguration(environment, systemInfo, optimalSettings) {
        const config = {
            test_settings: optimalSettings,
            browser_settings: {
                ...this.defaultConfig.browser_settings,
                headless: environment.ci || environment.docker, // GUI in local dev
                video: !environment.ci, // No video in CI to save space
            },
            resource_limits: {
                ...this.defaultConfig.resource_limits,
                max_cpu_percent: environment.ci ? 60 : 70,
                max_memory_percent: environment.ci ? 50 : 60,
            },
            environment: {
                type: environment.type,
                ci: environment.ci,
                docker: environment.docker,
                wsl: environment.wsl,
                detected_at: new Date().toISOString(),
                system_info: systemInfo,
                details: environment.details
            }
        };

        return config;
    }

    /**
     * Validate the generated configuration
     */
    async validateConfiguration(config) {
        const validation = {
            valid: true,
            warnings: [],
            errors: []
        };

        try {
            // Check if workers setting is reasonable
            if (config.test_settings.workers > os.cpus().length) {
                validation.warnings.push(`Workers (${config.test_settings.workers}) exceeds CPU cores (${os.cpus().length})`);
            }

            // Check memory limits
            const totalMemoryMB = Math.round(os.totalmem() / 1024 / 1024);
            if (config.test_settings.memory_limit_mb > totalMemoryMB * 0.8) {
                validation.warnings.push(`Memory limit (${config.test_settings.memory_limit_mb}MB) may be too high for system (${totalMemoryMB}MB total)`);
            }

            // Check if required directories exist
            const requiredDirs = ['ui', 'scripts'];
            for (const dir of requiredDirs) {
                if (!fs.existsSync(path.join(__dirname, '..', dir))) {
                    validation.errors.push(`Required directory not found: ${dir}`);
                    validation.valid = false;
                }
            }

            // Check if Node.js and npm are available
            try {
                execSync('node --version', { stdio: 'ignore' });
                execSync('npm --version', { stdio: 'ignore' });
            } catch (error) {
                validation.errors.push('Node.js or npm not found in PATH');
                validation.valid = false;
            }

        } catch (error) {
            validation.errors.push(`Validation error: ${error.message}`);
            validation.valid = false;
        }

        return validation;
    }

    /**
     * Save configuration to YAML file
     */
    async saveConfiguration(config) {
        try {
            // Convert to YAML format (simple implementation)
            const yamlContent = this.objectToYaml(config);
            fs.writeFileSync(this.configPath, yamlContent, 'utf8');
            console.log(`✅ Configuration saved to: ${path.relative(process.cwd(), this.configPath)}`);
        } catch (error) {
            throw new Error(`Failed to save configuration: ${error.message}`);
        }
    }

    /**
     * Save settings to JSON file for quick access
     */
    async saveSettings(settings) {
        try {
            const settingsData = {
                ...settings,
                generated_at: new Date().toISOString(),
                version: '1.0'
            };
            
            // Ensure UI directory exists
            const uiDir = path.dirname(this.settingsPath);
            if (!fs.existsSync(uiDir)) {
                fs.mkdirSync(uiDir, { recursive: true });
            }
            
            fs.writeFileSync(this.settingsPath, JSON.stringify(settingsData, null, 2), 'utf8');
            console.log(`✅ Settings saved to: ${path.relative(process.cwd(), this.settingsPath)}`);
        } catch (error) {
            console.log(`⚠️  Could not save settings file: ${error.message}`);
        }
    }

    /**
     * Simple object to YAML converter
     */
    objectToYaml(obj, indent = 0) {
        let yaml = '';
        const spaces = '  '.repeat(indent);
        
        for (const [key, value] of Object.entries(obj)) {
            if (value === null || value === undefined) {
                yaml += `${spaces}${key}: null\n`;
            } else if (typeof value === 'object' && !Array.isArray(value)) {
                yaml += `${spaces}${key}:\n`;
                yaml += this.objectToYaml(value, indent + 1);
            } else if (Array.isArray(value)) {
                yaml += `${spaces}${key}:\n`;
                for (const item of value) {
                    if (typeof item === 'object') {
                        yaml += `${spaces}  -\n`;
                        yaml += this.objectToYaml(item, indent + 2);
                    } else {
                        yaml += `${spaces}  - ${item}\n`;
                    }
                }
            } else if (typeof value === 'string') {
                yaml += `${spaces}${key}: "${value}"\n`;
            } else {
                yaml += `${spaces}${key}: ${value}\n`;
            }
        }
        
        return yaml;
    }

    /**
     * Display configuration summary
     */
    displayConfigurationSummary(config) {
        console.log('\n📋 Configuration Summary:');
        console.log('═'.repeat(50));
        console.log(`Environment: ${config.environment.type.toUpperCase()}`);
        console.log(`Workers: ${config.test_settings.workers}`);
        console.log(`Memory Limit: ${config.test_settings.memory_limit_mb}MB`);
        console.log(`Parallel Mode: ${config.test_settings.parallel_mode}`);
        console.log(`Timeout: ${config.test_settings.timeout_ms}ms`);
        console.log(`Headless: ${config.browser_settings.headless ? 'Yes' : 'No'}`);
        console.log(`Video Recording: ${config.browser_settings.video ? 'Yes' : 'No'}`);
        
        if (config.environment.ci) {
            console.log(`CI Provider: ${config.environment.details.ci_provider || 'unknown'}`);
        }
        
        if (config.environment.docker) {
            console.log('Docker: Yes');
        }
        
        if (config.environment.wsl) {
            console.log(`WSL: Yes (v${config.environment.details.wsl_version || 'unknown'})`);
        }
        
        console.log('═'.repeat(50));
        console.log('✅ Intelligent test configuration complete!\n');
        
        console.log('💡 Usage:');
        console.log(`   ./scripts/run-e2e-tests.sh --config ${path.basename(this.configPath)}`);
        console.log('   npm run test:e2e:optimized');
        console.log('');
    }

    /**
     * Load existing configuration if available
     */
    loadExistingConfig() {
        try {
            if (fs.existsSync(this.settingsPath)) {
                const settings = JSON.parse(fs.readFileSync(this.settingsPath, 'utf8'));
                console.log('📁 Found existing configuration from', settings.generated_at);
                return settings;
            }
        } catch (error) {
            console.log('⚠️  Could not load existing configuration:', error.message);
        }
        return null;
    }

    /**
     * Check if configuration needs updating
     */
    needsUpdate() {
        const existing = this.loadExistingConfig();
        if (!existing) return true;
        
        // Check if configuration is older than 7 days
        const generatedAt = new Date(existing.generated_at);
        const weekAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);
        
        return generatedAt < weekAgo;
    }
}

// CLI interface
async function main() {
    const args = process.argv.slice(2);
    const config = new IntelligentTestConfig();
    
    if (args.includes('--help') || args.includes('-h')) {
        console.log(`
Intelligent Test Configuration Generator

Usage:
  node scripts/intelligent-test-config.js [options]

Options:
  --force          Force regeneration even if config exists
  --check-only     Only check current configuration
  --help, -h       Show this help message

Examples:
  node scripts/intelligent-test-config.js
  node scripts/intelligent-test-config.js --force
  node scripts/intelligent-test-config.js --check-only
`);
        process.exit(0);
    }
    
    if (args.includes('--check-only')) {
        const existing = config.loadExistingConfig();
        if (existing) {
            console.log('✅ Configuration exists and is current');
            console.log(`Generated: ${existing.generated_at}`);
            console.log(`Workers: ${existing.workers}, Memory: ${existing.memory_limit_mb}MB`);
        } else {
            console.log('❌ No configuration found - run without --check-only to generate');
        }
        process.exit(0);
    }
    
    const force = args.includes('--force');
    
    if (!force && !config.needsUpdate()) {
        console.log('✅ Configuration is current - use --force to regenerate');
        process.exit(0);
    }
    
    try {
        await config.generateOptimalConfig();
        console.log('🎉 Intelligent test configuration completed successfully!');
        process.exit(0);
    } catch (error) {
        console.error('❌ Failed to generate configuration:', error.message);
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

module.exports = IntelligentTestConfig;
