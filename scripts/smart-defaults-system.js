#!/usr/bin/env node

/**
 * Smart Defaults System for AgentGateway E2E Testing
 * 
 * Provides intelligent environment detection with conservative fallbacks
 * for unknown environments, ensuring tests work out-of-the-box on all platforms.
 * 
 * Features:
 * - Advanced environment detection (CI, Docker, WSL, local)
 * - Conservative defaults for unknown/constrained environments
 * - Easy override mechanisms for advanced users
 * - Performance optimization recommendations
 * - Cross-platform compatibility validation
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync } = require('child_process');

class SmartDefaultsSystem {
    constructor() {
        this.configPath = path.join(__dirname, '..', 'smart-defaults.json');
        this.overridesPath = path.join(__dirname, '..', 'test-overrides.json');
        
        // Conservative base defaults - guaranteed to work on minimal systems
        this.conservativeDefaults = {
            workers: 1,
            memory_limit_mb: 1024,
            timeout_ms: 45000,
            retry_attempts: 3,
            parallel_mode: 'conservative',
            browser_settings: {
                headless: true,
                video: false,
                screenshots: true,
                viewport_width: 1280,
                viewport_height: 720
            },
            resource_limits: {
                max_cpu_percent: 50,
                max_memory_percent: 40,
                emergency_threshold_percent: 75
            }
        };
        
        // Environment-specific optimizations
        this.environmentProfiles = {
            'github-actions': {
                workers: 2,
                memory_limit_mb: 2048,
                timeout_ms: 60000,
                parallel_mode: 'balanced',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 60, max_memory_percent: 50 }
            },
            'gitlab-ci': {
                workers: 2,
                memory_limit_mb: 1536,
                timeout_ms: 50000,
                parallel_mode: 'conservative',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 55, max_memory_percent: 45 }
            },
            'docker-local': {
                workers: 2,
                memory_limit_mb: 2048,
                timeout_ms: 40000,
                parallel_mode: 'balanced',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 65, max_memory_percent: 55 }
            },
            'docker-ci': {
                workers: 1,
                memory_limit_mb: 1536,
                timeout_ms: 50000,
                parallel_mode: 'conservative',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 50, max_memory_percent: 40 }
            },
            'wsl': {
                workers: 2,
                memory_limit_mb: 2048,
                timeout_ms: 35000,
                parallel_mode: 'balanced',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 60, max_memory_percent: 50 }
            },
            'local-high-end': {
                workers: 6,
                memory_limit_mb: 6144,
                timeout_ms: 30000,
                parallel_mode: 'aggressive',
                browser_settings: { headless: false, video: true },
                resource_limits: { max_cpu_percent: 75, max_memory_percent: 65 }
            },
            'local-mid-range': {
                workers: 4,
                memory_limit_mb: 4096,
                timeout_ms: 30000,
                parallel_mode: 'balanced',
                browser_settings: { headless: false, video: true },
                resource_limits: { max_cpu_percent: 70, max_memory_percent: 60 }
            },
            'local-low-end': {
                workers: 2,
                memory_limit_mb: 2048,
                timeout_ms: 40000,
                parallel_mode: 'conservative',
                browser_settings: { headless: true, video: false },
                resource_limits: { max_cpu_percent: 60, max_memory_percent: 50 }
            }
        };
    }

    /**
     * Main entry point - detect environment and provide smart defaults
     */
    async generateSmartDefaults(options = {}) {
        console.log('🧠 Smart Defaults System - Analyzing environment...\n');

        try {
            // Detect comprehensive environment information
            const environment = await this.detectComprehensiveEnvironment();
            console.log(`🌍 Environment: ${environment.profile} (${environment.type})`);

            // Get system capabilities
            const systemInfo = await this.getSystemCapabilities();
            console.log(`💻 System: ${systemInfo.cpu_cores} cores, ${systemInfo.memory_gb}GB RAM, Load: ${systemInfo.load_1m.toFixed(2)}`);

            // Apply smart defaults based on environment and system
            const smartDefaults = this.calculateSmartDefaults(environment, systemInfo, options);
            console.log(`⚙️  Smart defaults: ${smartDefaults.workers} workers, ${smartDefaults.memory_limit_mb}MB limit`);

            // Validate defaults against system constraints
            const validatedDefaults = await this.validateAndAdjustDefaults(smartDefaults, systemInfo);

            // Apply user overrides if they exist
            const finalDefaults = this.applyUserOverrides(validatedDefaults);

            // Save configuration for future use
            await this.saveSmartDefaults(finalDefaults, environment, systemInfo);

            // Display comprehensive summary
            this.displaySmartDefaultsSummary(finalDefaults, environment, systemInfo);

            return {
                defaults: finalDefaults,
                environment: environment,
                systemInfo: systemInfo,
                recommendations: this.generateRecommendations(finalDefaults, environment, systemInfo)
            };

        } catch (error) {
            console.error('❌ Error in Smart Defaults System:', error.message);
            console.log('🔄 Falling back to ultra-conservative defaults...');
            
            const fallbackDefaults = { ...this.conservativeDefaults };
            await this.saveSmartDefaults(fallbackDefaults, { type: 'unknown', profile: 'fallback' }, {});
            return { defaults: fallbackDefaults, environment: { type: 'unknown' }, systemInfo: {} };
        }
    }

    /**
     * Comprehensive environment detection with detailed profiling
     */
    async detectComprehensiveEnvironment() {
        const env = {
            type: 'local',
            profile: 'unknown',
            ci: false,
            docker: false,
            wsl: false,
            details: {},
            confidence: 0
        };

        // CI Environment Detection
        if (process.env.CI) {
            env.ci = true;
            env.type = 'ci';
            env.confidence += 30;

            if (process.env.GITHUB_ACTIONS) {
                env.profile = 'github-actions';
                env.details.ci_provider = 'github-actions';
                env.details.runner_os = process.env.RUNNER_OS;
                env.details.runner_arch = process.env.RUNNER_ARCH;
                env.confidence += 40;
            } else if (process.env.GITLAB_CI) {
                env.profile = 'gitlab-ci';
                env.details.ci_provider = 'gitlab-ci';
                env.details.ci_runner_tags = process.env.CI_RUNNER_TAGS;
                env.confidence += 40;
            } else if (process.env.JENKINS_URL) {
                env.profile = 'jenkins';
                env.details.ci_provider = 'jenkins';
                env.confidence += 35;
            } else {
                env.profile = 'generic-ci';
                env.confidence += 20;
            }
        }

        // Docker Environment Detection
        if (fs.existsSync('/.dockerenv') || process.env.DOCKER_CONTAINER) {
            env.docker = true;
            env.confidence += 25;
            
            if (env.ci) {
                env.profile = 'docker-ci';
                env.type = 'docker-ci';
            } else {
                env.profile = 'docker-local';
                env.type = 'docker';
            }

            // Try to detect Docker resource limits
            try {
                if (fs.existsSync('/sys/fs/cgroup/memory/memory.limit_in_bytes')) {
                    const memLimit = fs.readFileSync('/sys/fs/cgroup/memory/memory.limit_in_bytes', 'utf8').trim();
                    env.details.docker_memory_limit = parseInt(memLimit);
                }
                if (fs.existsSync('/sys/fs/cgroup/cpu/cpu.cfs_quota_us')) {
                    const cpuQuota = fs.readFileSync('/sys/fs/cgroup/cpu/cpu.cfs_quota_us', 'utf8').trim();
                    env.details.docker_cpu_quota = parseInt(cpuQuota);
                }
            } catch (e) {
                // Ignore cgroup detection errors
            }
        }

        // WSL Detection
        if (process.platform === 'linux' && !env.docker) {
            try {
                if (fs.existsSync('/proc/version')) {
                    const version = fs.readFileSync('/proc/version', 'utf8').toLowerCase();
                    if (version.includes('microsoft') || version.includes('wsl')) {
                        env.wsl = true;
                        env.profile = 'wsl';
                        env.type = 'wsl';
                        env.confidence += 30;
                        
                        env.details.wsl_version = version.includes('wsl2') ? '2' : '1';
                        
                        // Check for WSL-specific limitations
                        if (version.includes('wsl1')) {
                            env.details.wsl_limitations = ['limited_filesystem_performance', 'no_docker_support'];
                        }
                    }
                }
            } catch (e) {
                // Ignore WSL detection errors
            }
        }

        // Local Environment Profiling (if not CI/Docker/WSL)
        if (!env.ci && !env.docker && !env.wsl) {
            const systemInfo = await this.getSystemCapabilities();
            
            if (systemInfo.memory_gb >= 16 && systemInfo.cpu_cores >= 8) {
                env.profile = 'local-high-end';
                env.confidence += 25;
            } else if (systemInfo.memory_gb >= 8 && systemInfo.cpu_cores >= 4) {
                env.profile = 'local-mid-range';
                env.confidence += 25;
            } else {
                env.profile = 'local-low-end';
                env.confidence += 25;
            }
            
            env.type = 'local';
        }

        // Platform-specific adjustments
        env.details.platform = process.platform;
        env.details.arch = process.arch;
        env.details.node_version = process.version;

        // Additional environment indicators
        if (process.env.TERM_PROGRAM) {
            env.details.terminal = process.env.TERM_PROGRAM;
        }
        if (process.env.SSH_CLIENT || process.env.SSH_TTY) {
            env.details.remote_session = true;
            env.confidence -= 10; // Slightly less confident about local optimizations
        }

        return env;
    }

    /**
     * Get comprehensive system capabilities
     */
    async getSystemCapabilities() {
        const systemInfo = {
            platform: process.platform,
            arch: process.arch,
            cpu_cores: os.cpus().length,
            memory_gb: Math.round(os.totalmem() / (1024 * 1024 * 1024) * 10) / 10,
            memory_free_gb: Math.round(os.freemem() / (1024 * 1024 * 1024) * 10) / 10,
            load_1m: os.loadavg()[0],
            load_5m: os.loadavg()[1],
            load_15m: os.loadavg()[2],
            uptime_hours: Math.round(os.uptime() / 3600 * 10) / 10
        };

        // Enhanced system information
        try {
            systemInfo.cpu_model = os.cpus()[0].model;
            systemInfo.cpu_speed_mhz = os.cpus()[0].speed;

            // Memory details (Linux)
            if (process.platform === 'linux' && fs.existsSync('/proc/meminfo')) {
                const meminfo = fs.readFileSync('/proc/meminfo', 'utf8');
                
                const available = meminfo.match(/MemAvailable:\s+(\d+)\s+kB/);
                if (available) {
                    systemInfo.memory_available_gb = Math.round(parseInt(available[1]) / 1024 / 1024 * 10) / 10;
                }
                
                const cached = meminfo.match(/Cached:\s+(\d+)\s+kB/);
                if (cached) {
                    systemInfo.memory_cached_gb = Math.round(parseInt(cached[1]) / 1024 / 1024 * 10) / 10;
                }
            }

            // Disk space
            try {
                const diskUsage = execSync('df -h . | tail -1', { encoding: 'utf8' });
                const diskMatch = diskUsage.match(/\s+(\d+)G\s+(\d+)G\s+(\d+)G\s+(\d+)%/);
                if (diskMatch) {
                    systemInfo.disk_total_gb = parseInt(diskMatch[1]);
                    systemInfo.disk_used_gb = parseInt(diskMatch[2]);
                    systemInfo.disk_available_gb = parseInt(diskMatch[3]);
                    systemInfo.disk_usage_percent = parseInt(diskMatch[4]);
                }
            } catch (e) {
                // Ignore disk space detection errors
            }

            // Node.js process memory
            const memUsage = process.memoryUsage();
            systemInfo.node_memory_mb = Math.round(memUsage.rss / 1024 / 1024);
            systemInfo.node_heap_mb = Math.round(memUsage.heapUsed / 1024 / 1024);

        } catch (error) {
            console.log('⚠️  Could not gather extended system information:', error.message);
        }

        return systemInfo;
    }

    /**
     * Calculate smart defaults based on environment and system capabilities
     */
    calculateSmartDefaults(environment, systemInfo, options = {}) {
        // Start with conservative base
        let defaults = { ...this.conservativeDefaults };

        // Apply environment profile if available
        if (this.environmentProfiles[environment.profile]) {
            const profile = this.environmentProfiles[environment.profile];
            defaults = { ...defaults, ...profile };
            console.log(`📋 Applied profile: ${environment.profile}`);
        }

        // System-based adjustments
        const availableMemoryGB = systemInfo.memory_available_gb || systemInfo.memory_free_gb;
        const totalMemoryGB = systemInfo.memory_gb;
        const cpuCores = systemInfo.cpu_cores;
        const currentLoad = systemInfo.load_1m;

        // CPU-based worker adjustment
        if (cpuCores >= 8 && currentLoad < 2.0) {
            defaults.workers = Math.min(defaults.workers * 1.5, 8);
        } else if (cpuCores <= 2 || currentLoad > 4.0) {
            defaults.workers = Math.max(1, Math.floor(defaults.workers * 0.7));
        }

        // Memory-based adjustments
        if (totalMemoryGB >= 16 && availableMemoryGB >= 8) {
            defaults.memory_limit_mb = Math.min(defaults.memory_limit_mb * 1.5, 8192);
        } else if (totalMemoryGB <= 4 || availableMemoryGB <= 2) {
            defaults.memory_limit_mb = Math.max(1024, Math.floor(defaults.memory_limit_mb * 0.7));
        }

        // Load-based timeout adjustments
        if (currentLoad > 2.0) {
            defaults.timeout_ms = Math.floor(defaults.timeout_ms * 1.3);
            defaults.retry_attempts = Math.min(defaults.retry_attempts + 1, 5);
        }

        // Platform-specific adjustments
        if (process.platform === 'win32') {
            defaults.timeout_ms = Math.floor(defaults.timeout_ms * 1.2); // Windows can be slower
            defaults.workers = Math.max(1, Math.floor(defaults.workers * 0.8));
        }

        // Apply user preferences from options
        if (options.prefer_speed && totalMemoryGB >= 8) {
            defaults.parallel_mode = 'aggressive';
            defaults.workers = Math.min(defaults.workers * 1.3, cpuCores);
        } else if (options.prefer_stability) {
            defaults.parallel_mode = 'conservative';
            defaults.workers = Math.max(1, Math.floor(defaults.workers * 0.8));
            defaults.timeout_ms = Math.floor(defaults.timeout_ms * 1.5);
        }

        return defaults;
    }

    /**
     * Validate and adjust defaults against system constraints
     */
    async validateAndAdjustDefaults(defaults, systemInfo) {
        const validated = { ...defaults };
        const warnings = [];

        // Memory validation
        const maxSafeMemoryMB = Math.floor(systemInfo.memory_gb * 1024 * 0.7); // Leave 30% free
        if (validated.memory_limit_mb > maxSafeMemoryMB) {
            validated.memory_limit_mb = maxSafeMemoryMB;
            warnings.push(`Reduced memory limit to ${maxSafeMemoryMB}MB (70% of total)`);
        }

        // Worker validation
        const maxSafeWorkers = Math.max(1, Math.floor(systemInfo.cpu_cores * 0.8));
        if (validated.workers > maxSafeWorkers) {
            validated.workers = maxSafeWorkers;
            warnings.push(`Reduced workers to ${maxSafeWorkers} (80% of CPU cores)`);
        }

        // Load-based adjustments
        if (systemInfo.load_1m > systemInfo.cpu_cores * 0.8) {
            validated.workers = Math.max(1, Math.floor(validated.workers * 0.6));
            validated.timeout_ms = Math.floor(validated.timeout_ms * 1.4);
            warnings.push('System under load - reduced workers and increased timeout');
        }

        // Disk space validation
        if (systemInfo.disk_available_gb && systemInfo.disk_available_gb < 2) {
            validated.browser_settings.video = false;
            validated.browser_settings.screenshots = false;
            warnings.push('Low disk space - disabled video and screenshots');
        }

        // Report warnings
        if (warnings.length > 0) {
            console.log('⚠️  Validation adjustments:');
            warnings.forEach(warning => console.log(`   - ${warning}`));
        }

        return validated;
    }

    /**
     * Apply user overrides from configuration file
     */
    applyUserOverrides(defaults) {
        if (!fs.existsSync(this.overridesPath)) {
            return defaults;
        }

        try {
            const overrides = JSON.parse(fs.readFileSync(this.overridesPath, 'utf8'));
            const finalDefaults = { ...defaults };

            // Apply overrides with validation
            if (overrides.workers && overrides.workers > 0) {
                finalDefaults.workers = Math.min(overrides.workers, os.cpus().length);
            }
            if (overrides.memory_limit_mb && overrides.memory_limit_mb > 0) {
                finalDefaults.memory_limit_mb = overrides.memory_limit_mb;
            }
            if (overrides.timeout_ms && overrides.timeout_ms > 0) {
                finalDefaults.timeout_ms = overrides.timeout_ms;
            }
            if (overrides.parallel_mode) {
                finalDefaults.parallel_mode = overrides.parallel_mode;
            }

            // Apply browser setting overrides
            if (overrides.browser_settings) {
                finalDefaults.browser_settings = { ...finalDefaults.browser_settings, ...overrides.browser_settings };
            }

            console.log('🔧 Applied user overrides from', path.basename(this.overridesPath));
            return finalDefaults;

        } catch (error) {
            console.log('⚠️  Could not apply user overrides:', error.message);
            return defaults;
        }
    }

    /**
     * Save smart defaults configuration
     */
    async saveSmartDefaults(defaults, environment, systemInfo) {
        const config = {
            defaults: defaults,
            environment: environment,
            system_info: systemInfo,
            generated_at: new Date().toISOString(),
            version: '2.0',
            generator: 'smart-defaults-system'
        };

        try {
            fs.writeFileSync(this.configPath, JSON.stringify(config, null, 2), 'utf8');
            console.log(`✅ Smart defaults saved to: ${path.relative(process.cwd(), this.configPath)}`);
        } catch (error) {
            console.log(`⚠️  Could not save smart defaults: ${error.message}`);
        }
    }

    /**
     * Generate performance and optimization recommendations
     */
    generateRecommendations(defaults, environment, systemInfo) {
        const recommendations = [];

        // Performance recommendations
        if (systemInfo.memory_gb >= 16 && defaults.workers < 4) {
            recommendations.push({
                type: 'performance',
                message: 'Your system can handle more parallel workers',
                suggestion: `Consider using --workers ${Math.min(6, systemInfo.cpu_cores)} for faster execution`
            });
        }

        if (systemInfo.load_1m > systemInfo.cpu_cores * 0.7) {
            recommendations.push({
                type: 'stability',
                message: 'System is under high load',
                suggestion: 'Consider running tests when system load is lower, or use --workers 1'
            });
        }

        // Environment-specific recommendations
        if (environment.wsl) {
            recommendations.push({
                type: 'compatibility',
                message: 'WSL detected - optimized for Windows Subsystem for Linux',
                suggestion: 'File system operations may be slower than native Linux'
            });
        }

        if (environment.docker) {
            recommendations.push({
                type: 'resource',
                message: 'Docker environment detected',
                suggestion: 'Ensure Docker has sufficient memory allocated (8GB+ recommended)'
            });
        }

        // Resource recommendations
        if (systemInfo.disk_available_gb && systemInfo.disk_available_gb < 5) {
            recommendations.push({
                type: 'resource',
                message: 'Low disk space detected',
                suggestion: 'Free up disk space or disable video recording to prevent test failures'
            });
        }

        return recommendations;
    }

    /**
     * Display comprehensive smart defaults summary
     */
    displaySmartDefaultsSummary(defaults, environment, systemInfo) {
        console.log('\n🧠 Smart Defaults Summary:');
        console.log('═'.repeat(60));
        
        // Environment information
        console.log(`Environment: ${environment.profile} (confidence: ${environment.confidence}%)`);
        console.log(`Platform: ${systemInfo.platform}/${systemInfo.arch}`);
        console.log(`System: ${systemInfo.cpu_cores} cores, ${systemInfo.memory_gb}GB RAM`);
        console.log(`Load: ${systemInfo.load_1m.toFixed(2)} (1m), Available: ${(systemInfo.memory_available_gb || systemInfo.memory_free_gb).toFixed(1)}GB`);
        
        console.log('\n📊 Optimized Settings:');
        console.log(`Workers: ${defaults.workers}`);
        console.log(`Memory Limit: ${defaults.memory_limit_mb}MB`);
        console.log(`Parallel Mode: ${defaults.parallel_mode}`);
        console.log(`Timeout: ${defaults.timeout_ms}ms`);
        console.log(`Retry Attempts: ${defaults.retry_attempts}`);
        
        console.log('\n🖥️  Browser Settings:');
        console.log(`Headless: ${defaults.browser_settings.headless ? 'Yes' : 'No'}`);
        console.log(`Video Recording: ${defaults.browser_settings.video ? 'Yes' : 'No'}`);
        console.log(`Screenshots: ${defaults.browser_settings.screenshots ? 'Yes' : 'No'}`);
        
        console.log('\n⚡ Resource Limits:');
        console.log(`Max CPU: ${defaults.resource_limits.max_cpu_percent}%`);
        console.log(`Max Memory: ${defaults.resource_limits.max_memory_percent}%`);
        console.log(`Emergency Threshold: ${defaults.resource_limits.emergency_threshold_percent}%`);
        
        if (environment.details && Object.keys(environment.details).length > 0) {
            console.log('\n🔍 Environment Details:');
            Object.entries(environment.details).forEach(([key, value]) => {
                console.log(`${key}: ${value}`);
            });
        }
        
        console.log('═'.repeat(60));
        console.log('✅ Smart defaults configuration complete!\n');
        
        console.log('💡 Usage:');
        console.log('   ./scripts/run-e2e-tests.sh  # Uses smart defaults automatically');
        console.log('   npm run test:e2e:smart      # If npm script is configured');
        console.log('\n🔧 Override Options:');
        console.log(`   Create ${path.basename(this.overridesPath)} to customize settings`);
        console.log('   Use CLI flags: --workers N --memory-limit N --timeout N');
        console.log('');
    }

    /**
     * Create user override template
     */
    createOverrideTemplate() {
        const template = {
            "_comment": "User overrides for smart defaults - customize as needed",
            "workers": null,
            "memory_limit_mb": null,
            "timeout_ms": null,
            "parallel_mode": null,
            "browser_settings": {
                "headless": null,
                "video": null,
                "screenshots": null
            },
            "resource_limits": {
                "max_cpu_percent": null,
                "max_memory_percent": null
            }
        };

        try {
            fs.writeFileSync(this.overridesPath, JSON.stringify(template, null, 2), 'utf8');
            console.log(`📝 Created override template: ${path.relative(process.cwd(), this.overridesPath)}`);
            console.log('   Edit this file to customize your test settings');
        } catch (error) {
            console.log(`⚠️  Could not create override template: ${error.message}`);
        }
    }

    /**
     * Load existing smart defaults
     */
    loadExistingDefaults() {
        if (!fs.existsSync(this.configPath)) {
            return null;
        }

        try {
            const config = JSON.parse(fs.readFileSync(this.configPath, 'utf8'));
            return config;
        } catch (error) {
            console.log('⚠️  Could not load existing smart defaults:', error.message);
            return null;
        }
    }

    /**
     * Check if smart defaults need updating
     */
    needsUpdate() {
        const existing = this.loadExistingDefaults();
        if (!existing) return true;

        // Check if configuration is older than 24 hours
        const generatedAt = new Date(existing.generated_at);
        const dayAgo = new Date(Date.now() - 24 * 60 * 60 * 1000);

        return generatedAt < dayAgo;
    }
}

// CLI interface
async function main() {
    const args = process.argv.slice(2);
    const smartDefaults = new SmartDefaultsSystem();

    if (args.includes('--help') || args.includes('-h')) {
        console.log(`
Smart Defaults System for AgentGateway E2E Testing

Usage:
  node scripts/smart-defaults-system.js [options]

Options:
  --force              Force regeneration even if config exists
  --check-only         Only check current configuration
  --create-template    Create user override template
  --prefer-speed       Optimize for speed (requires sufficient resources)
  --prefer-stability   Optimize for stability and reliability
  --help, -h           Show this help message

Examples:
  node scripts/smart-defaults-system.js
  node scripts/smart-defaults-system.js --force --prefer-speed
  node scripts/smart-defaults-system.js --create-template
  node scripts/smart-defaults-system.js --check-only
`);
        process.exit(0);
    }

    if (args.includes('--create-template')) {
        smartDefaults.createOverrideTemplate();
        process.exit(0);
    }

    if (args.includes('--check-only')) {
        const existing = smartDefaults.loadExistingDefaults();
        if (existing) {
            console.log('✅ Smart defaults configuration exists');
            console.log(`Generated: ${existing.generated_at}`);
            console.log(`Environment: ${existing.environment.profile}`);
            console.log(`Workers: ${existing.defaults.workers}, Memory: ${existing.defaults.memory_limit_mb}MB`);
        } else {
            console.log('❌ No smart defaults found - run without --check-only to generate');
        }
        process.exit(0);
    }

    const force = args.includes('--force');
    const options = {
        prefer_speed: args.includes('--prefer-speed'),
        prefer_stability: args.includes('--prefer-stability')
    };

    if (!force && !smartDefaults.needsUpdate()) {
        console.log('✅ Smart defaults are current - use --force to regenerate');
        const existing = smartDefaults.loadExistingDefaults();
        if (existing) {
            console.log(`Current profile: ${existing.environment.profile}`);
            console.log(`Workers: ${existing.defaults.workers}, Memory: ${existing.defaults.memory_limit_mb}MB`);
        }
        process.exit(0);
    }

    try {
        const result = await smartDefaults.generateSmartDefaults(options);
        
        if (result.recommendations.length > 0) {
            console.log('💡 Recommendations:');
            result.recommendations.forEach(rec => {
                console.log(`   ${rec.type.toUpperCase()}: ${rec.message}`);
                console.log(`   → ${rec.suggestion}`);
            });
        }
        
        console.log('🎉 Smart defaults system completed successfully!');
        process.exit(0);
    } catch (error) {
        console.error('❌ Failed to generate smart defaults:', error.message);
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

module.exports = SmartDefaultsSystem;
