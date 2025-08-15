#!/usr/bin/env node

const os = require('os');
const fs = require('fs').promises;
const { execSync } = require('child_process');
const path = require('path');

/**
 * System Resource Detection Script
 * 
 * This script detects and documents system resources for E2E test configuration.
 * Run this on any deployment environment to generate appropriate test settings.
 */

class SystemResourceDetector {
  constructor() {
    this.results = {
      timestamp: new Date().toISOString(),
      environment: this.detectEnvironment(),
      system: {},
      memory: {},
      cpu: {},
      disk: {},
      recommendations: {}
    };
  }

  /**
   * Detect the current environment
   */
  detectEnvironment() {
    if (process.env.CI) {
      return {
        type: 'ci',
        provider: this.detectCIProvider(),
        runner: process.env.RUNNER_OS || 'unknown'
      };
    } else if (process.env.NODE_ENV === 'development') {
      return {
        type: 'development',
        provider: 'local',
        runner: os.platform()
      };
    } else {
      return {
        type: 'production',
        provider: 'unknown',
        runner: os.platform()
      };
    }
  }

  /**
   * Detect CI provider
   */
  detectCIProvider() {
    if (process.env.GITLAB_CI) return 'gitlab';
    if (process.env.GITHUB_ACTIONS) return 'github';
    if (process.env.JENKINS_URL) return 'jenkins';
    if (process.env.CIRCLECI) return 'circleci';
    if (process.env.TRAVIS) return 'travis';
    return 'unknown';
  }

  /**
   * Detect system information
   */
  detectSystemInfo() {
    this.results.system = {
      platform: os.platform(),
      arch: os.arch(),
      nodeVersion: process.version,
      hostname: os.hostname(),
      uptime: os.uptime(),
      loadAverage: os.loadavg(),
      networkInterfaces: Object.keys(os.networkInterfaces()),
      tmpdir: os.tmpdir()
    };
  }

  /**
   * Detect memory information
   */
  detectMemoryInfo() {
    const totalMemory = os.totalmem();
    const freeMemory = os.freemem();
    const usedMemory = totalMemory - freeMemory;

    this.results.memory = {
      total: totalMemory,
      totalGB: Math.round(totalMemory / (1024 * 1024 * 1024) * 100) / 100,
      free: freeMemory,
      freeGB: Math.round(freeMemory / (1024 * 1024 * 1024) * 100) / 100,
      used: usedMemory,
      usedGB: Math.round(usedMemory / (1024 * 1024 * 1024) * 100) / 100,
      usagePercent: Math.round((usedMemory / totalMemory) * 100 * 100) / 100,
      
      // Calculate available memory for tests (reserve system overhead)
      systemOverhead: Math.min(2 * 1024 * 1024 * 1024, totalMemory * 0.15), // 2GB or 15%
      availableForTests: 0,
      availableForTestsGB: 0
    };

    this.results.memory.availableForTests = totalMemory - this.results.memory.systemOverhead;
    this.results.memory.availableForTestsGB = Math.round(this.results.memory.availableForTests / (1024 * 1024 * 1024) * 100) / 100;
  }

  /**
   * Detect CPU information
   */
  detectCPUInfo() {
    const cpus = os.cpus();
    const loadAvg = os.loadavg();

    this.results.cpu = {
      count: cpus.length,
      model: cpus[0]?.model || 'unknown',
      speed: cpus[0]?.speed || 0,
      loadAverage: {
        '1min': loadAvg[0],
        '5min': loadAvg[1],
        '15min': loadAvg[2]
      },
      currentUsage: Math.min((loadAvg[0] / cpus.length) * 100, 100)
    };
  }

  /**
   * Detect disk space information
   */
  async detectDiskInfo() {
    try {
      const diskUsage = await this.getDiskUsage(process.cwd());
      
      this.results.disk = {
        ...diskUsage,
        availableGB: Math.round(diskUsage.available / (1024 * 1024 * 1024) * 100) / 100,
        totalGB: Math.round(diskUsage.total / (1024 * 1024 * 1024) * 100) / 100,
        usedGB: Math.round(diskUsage.used / (1024 * 1024 * 1024) * 100) / 100,
        usagePercent: Math.round((diskUsage.used / diskUsage.total) * 100 * 100) / 100
      };
    } catch (error) {
      this.results.disk = {
        error: error.message,
        available: 0,
        total: 0,
        used: 0
      };
    }
  }

  /**
   * Get disk usage cross-platform
   */
  async getDiskUsage(path) {
    if (process.platform === 'win32') {
      return this.getDiskUsageWindows(path);
    } else {
      return this.getDiskUsageUnix(path);
    }
  }

  /**
   * Unix/Linux/macOS disk usage
   */
  getDiskUsageUnix(path) {
    try {
      const output = execSync(`df -k "${path}"`, { encoding: 'utf8' });
      const lines = output.trim().split('\n');
      const data = lines[1].split(/\s+/);
      
      const total = parseInt(data[1]) * 1024;
      const used = parseInt(data[2]) * 1024;
      const available = parseInt(data[3]) * 1024;
      
      return { total, used, available };
    } catch (error) {
      throw new Error(`Unix disk usage check failed: ${error.message}`);
    }
  }

  /**
   * Windows disk usage
   */
  getDiskUsageWindows(path) {
    try {
      const drive = path.charAt(0) + ':';
      const output = execSync(`fsutil volume diskfree "${drive}"`, { encoding: 'utf8' });
      
      const lines = output.split('\n');
      const freeBytes = parseInt(lines[0].match(/\d+/)[0]);
      const totalBytes = parseInt(lines[1].match(/\d+/)[0]);
      const usedBytes = totalBytes - freeBytes;
      
      return {
        total: totalBytes,
        used: usedBytes,
        available: freeBytes
      };
    } catch (error) {
      // Fallback method for Windows
      try {
        const output = execSync(`dir /-c "${path}"`, { encoding: 'utf8' });
        const match = output.match(/(\d+)\s+bytes free/);
        if (match) {
          const available = parseInt(match[1]);
          return {
            total: available * 2, // Rough estimate
            used: available,
            available: available
          };
        }
      } catch (fallbackError) {
        // Ignore fallback error
      }
      throw new Error(`Windows disk usage check failed: ${error.message}`);
    }
  }

  /**
   * Generate recommendations based on detected resources
   */
  generateRecommendations() {
    const { memory, cpu, environment } = this.results;
    
    // Base recommendations on available memory and CPU
    let maxWorkers = 1;
    let memoryLimit = 60; // Conservative default
    
    if (memory.availableForTestsGB >= 8) {
      maxWorkers = Math.min(cpu.count - 1, 4);
      memoryLimit = 65;
    } else if (memory.availableForTestsGB >= 4) {
      maxWorkers = Math.min(cpu.count - 1, 2);
      memoryLimit = 60;
    } else {
      maxWorkers = 1;
      memoryLimit = 50;
    }

    // Adjust for environment
    if (environment.type === 'ci') {
      maxWorkers = Math.min(maxWorkers, 2); // Conservative for CI
      memoryLimit = Math.min(memoryLimit, 60);
    }

    // Adjust for high CPU usage
    if (cpu.currentUsage > 50) {
      maxWorkers = Math.max(1, Math.floor(maxWorkers / 2));
    }

    this.results.recommendations = {
      maxWorkers,
      memoryLimit,
      diskBuffer: 200, // 200MB buffer
      strategy: maxWorkers > 2 ? 'balanced' : 'sequential',
      videoRecording: memory.availableForTestsGB > 4,
      headless: environment.type === 'ci',
      
      // Configuration for different test modes
      configurations: {
        conservative: {
          maxWorkers: 1,
          memoryLimit: 50,
          videoRecording: false
        },
        balanced: {
          maxWorkers: Math.max(1, Math.floor(maxWorkers / 2)),
          memoryLimit: memoryLimit - 10,
          videoRecording: memory.availableForTestsGB > 2
        },
        aggressive: {
          maxWorkers,
          memoryLimit,
          videoRecording: memory.availableForTestsGB > 4
        }
      },

      // Environment-specific npm scripts
      npmScripts: this.generateNpmScripts(maxWorkers, memoryLimit)
    };
  }

  /**
   * Generate recommended npm scripts
   */
  generateNpmScripts(maxWorkers, memoryLimit) {
    return {
      "test:e2e:auto": `./scripts/run-e2e-tests.sh --workers ${maxWorkers} --memory-limit ${memoryLimit}`,
      "test:e2e:conservative": `./scripts/run-e2e-tests.sh --workers 1 --memory-limit 50`,
      "test:e2e:smoke": `./scripts/run-e2e-tests.sh --smoke --workers 2`,
      "test:e2e:debug": `./scripts/run-e2e-tests.sh --workers 1 --headed --verbose`
    };
  }

  /**
   * Run all detection methods
   */
  async detect() {
    console.log('🔍 Detecting system resources...');
    
    this.detectSystemInfo();
    this.detectMemoryInfo();
    this.detectCPUInfo();
    await this.detectDiskInfo();
    this.generateRecommendations();
    
    console.log('✅ Resource detection complete');
    return this.results;
  }

  /**
   * Format results for display
   */
  formatResults() {
    const { system, memory, cpu, disk, recommendations, environment } = this.results;
    
    return `
System Resource Detection Report
================================
Generated: ${this.results.timestamp}
Environment: ${environment.type} (${environment.provider})

System Information:
------------------
Platform: ${system.platform} (${system.arch})
Node.js: ${system.nodeVersion}
Hostname: ${system.hostname}
Uptime: ${Math.floor(system.uptime / 3600)}h ${Math.floor((system.uptime % 3600) / 60)}m

Memory Information:
------------------
Total Memory: ${memory.totalGB} GB
Used Memory: ${memory.usedGB} GB (${memory.usagePercent}%)
Free Memory: ${memory.freeGB} GB
Available for Tests: ${memory.availableForTestsGB} GB

CPU Information:
---------------
CPU Cores: ${cpu.count}
CPU Model: ${cpu.model}
Current Load: ${cpu.currentUsage.toFixed(1)}%
Load Average: ${cpu.loadAverage['1min'].toFixed(2)} (1min)

Disk Information:
----------------
Total Disk: ${disk.totalGB} GB
Available: ${disk.availableGB} GB
Used: ${disk.usedGB} GB (${disk.usagePercent}%)

Recommendations:
---------------
Max Workers: ${recommendations.maxWorkers}
Memory Limit: ${recommendations.memoryLimit}%
Strategy: ${recommendations.strategy}
Video Recording: ${recommendations.videoRecording ? 'Enabled' : 'Disabled'}
Headless Mode: ${recommendations.headless ? 'Enabled' : 'Disabled'}

Configuration Options:
---------------------
Conservative: ${recommendations.configurations.conservative.maxWorkers} workers, ${recommendations.configurations.conservative.memoryLimit}% memory
Balanced: ${recommendations.configurations.balanced.maxWorkers} workers, ${recommendations.configurations.balanced.memoryLimit}% memory  
Aggressive: ${recommendations.configurations.aggressive.maxWorkers} workers, ${recommendations.configurations.aggressive.memoryLimit}% memory

Recommended npm Scripts:
-----------------------
${Object.entries(recommendations.npmScripts).map(([key, value]) => `"${key}": "${value}"`).join('\n')}

To use these recommendations, add the npm scripts to your package.json
or run the detection script with --apply to automatically configure.
`;
  }

  /**
   * Save results to file
   */
  async saveResults(outputPath = 'system-resources.json') {
    await fs.writeFile(outputPath, JSON.stringify(this.results, null, 2));
    console.log(`📊 Results saved to ${outputPath}`);
  }

  /**
   * Save formatted report
   */
  async saveReport(outputPath = 'system-resources-report.txt') {
    const report = this.formatResults();
    await fs.writeFile(outputPath, report);
    console.log(`📋 Report saved to ${outputPath}`);
  }
}

/**
 * CLI Interface
 */
async function main() {
  const args = process.argv.slice(2);
  const options = {
    output: args.includes('--output') ? args[args.indexOf('--output') + 1] : null,
    report: args.includes('--report') ? args[args.indexOf('--report') + 1] : null,
    apply: args.includes('--apply'),
    quiet: args.includes('--quiet')
  };

  try {
    const detector = new SystemResourceDetector();
    const results = await detector.detect();

    if (!options.quiet) {
      console.log(detector.formatResults());
    }

    // Save JSON results
    if (options.output) {
      await detector.saveResults(options.output);
    } else {
      await detector.saveResults();
    }

    // Save formatted report
    if (options.report) {
      await detector.saveReport(options.report);
    } else {
      await detector.saveReport();
    }

    // Apply recommendations to package.json
    if (options.apply) {
      await applyRecommendations(results.recommendations);
    }

    console.log('\n✅ Resource detection completed successfully');
    console.log('📁 Files generated:');
    console.log('  - system-resources.json (machine-readable data)');
    console.log('  - system-resources-report.txt (human-readable report)');
    
    if (options.apply) {
      console.log('  - package.json updated with recommended scripts');
    }

  } catch (error) {
    console.error('❌ Resource detection failed:', error.message);
    process.exit(1);
  }
}

/**
 * Apply recommendations to package.json
 */
async function applyRecommendations(recommendations) {
  try {
    const packageJsonPath = path.join(process.cwd(), 'ui', 'package.json');
    const packageJson = JSON.parse(await fs.readFile(packageJsonPath, 'utf8'));
    
    // Add recommended scripts
    packageJson.scripts = {
      ...packageJson.scripts,
      ...recommendations.npmScripts
    };
    
    await fs.writeFile(packageJsonPath, JSON.stringify(packageJson, null, 2));
    console.log('📦 Updated ui/package.json with recommended scripts');
  } catch (error) {
    console.warn('⚠️ Could not update package.json:', error.message);
  }
}

// Show help
if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log(`
System Resource Detection Script
================================

Usage: node detect-system-resources.js [options]

Options:
  --output <file>    Save JSON results to specified file (default: system-resources.json)
  --report <file>    Save formatted report to specified file (default: system-resources-report.txt)
  --apply           Apply recommendations to package.json
  --quiet           Suppress console output
  --help, -h        Show this help message

Examples:
  node detect-system-resources.js
  node detect-system-resources.js --apply
  node detect-system-resources.js --output ci-resources.json --report ci-report.txt
  node detect-system-resources.js --quiet --apply

This script detects system resources and generates recommendations for E2E test configuration.
Run this on any deployment environment to get optimal settings for that environment.
`);
  process.exit(0);
}

// Run if called directly
if (require.main === module) {
  main().catch(error => {
    console.error('💥 Unhandled error:', error);
    process.exit(1);
  });
}

module.exports = SystemResourceDetector;
