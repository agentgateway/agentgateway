#!/usr/bin/env node

const { spawn } = require('child_process');
const fs = require('fs').promises;
const path = require('path');
const ResourceMonitor = require('./lib/resource-monitor-fixed');

/**
 * Minimal E2E Test Script
 * 
 * This script provides a minimal approach to test the fixes:
 * 1. Test the fixed resource monitor
 * 2. Debug backend startup issues
 * 3. Validate basic E2E test execution
 */

class MinimalE2ETest {
  constructor() {
    this.resourceMonitor = null;
    this.processes = {
      backend: null,
      ui: null
    };
    this.startTime = Date.now();
  }

  /**
   * Test the fixed resource monitor
   */
  async testResourceMonitor() {
    console.log('🔧 Testing Fixed Resource Monitor...');
    
    try {
      this.resourceMonitor = new ResourceMonitor();
      
      // Test memory calculation
      const memory = this.resourceMonitor.checkMemoryUsage();
      console.log('📊 Memory Status:');
      console.log(`   Total: ${this.resourceMonitor.formatBytes(memory.total)}`);
      console.log(`   Used: ${this.resourceMonitor.formatBytes(memory.used)} (${memory.percentage.toFixed(1)}% of available)`);
      console.log(`   Available for Tests: ${this.resourceMonitor.formatBytes(memory.available)}`);
      console.log(`   System Overhead: ${this.resourceMonitor.formatBytes(memory.systemOverhead)}`);
      console.log(`   Safe: ${memory.safe ? '✅' : '❌'}`);
      
      // Test optimal worker calculation
      const optimalWorkers = this.resourceMonitor.calculateOptimalWorkers();
      console.log(`🔧 Optimal Workers: ${optimalWorkers}`);
      
      // Test CPU usage
      const cpu = this.resourceMonitor.checkCPUUsage();
      console.log(`💻 CPU: ${cpu.cores} cores, ${cpu.percentage.toFixed(1)}% usage, Safe: ${cpu.safe ? '✅' : '❌'}`);
      
      // Test disk space
      const disk = await this.resourceMonitor.checkDiskSpace();
      console.log(`💾 Disk: ${this.resourceMonitor.formatBytes(disk.available)} available, Safe: ${disk.safe ? '✅' : '❌'}`);
      
      console.log('✅ Resource Monitor test completed');
      return { memory, cpu, disk, optimalWorkers };
      
    } catch (error) {
      console.error('❌ Resource Monitor test failed:', error.message);
      throw error;
    }
  }

  /**
   * Test backend startup with detailed logging
   */
  async testBackendStartup() {
    console.log('🚀 Testing Backend Startup...');
    
    try {
      // Check if binary exists
      const binaryPath = './target/debug/agentgateway';
      try {
        await fs.access(binaryPath);
        console.log(`✅ Binary found: ${binaryPath}`);
      } catch (error) {
        console.log(`⚠️ Binary not found at ${binaryPath}, trying to build...`);
        await this.buildBackend();
      }
      
      // Check config file
      const configPath = './test-config.yaml';
      try {
        await fs.access(configPath);
        const config = await fs.readFile(configPath, 'utf8');
        console.log(`✅ Config found: ${configPath}`);
        console.log('📄 Config content:');
        console.log(config.split('\n').map(line => `   ${line}`).join('\n'));
      } catch (error) {
        console.log(`⚠️ Config not found: ${configPath}`);
        await this.createTestConfig();
      }
      
      // Start backend with detailed logging
      console.log('🔄 Starting AgentGateway backend...');
      const backend = spawn(binaryPath, ['--file', configPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, RUST_LOG: 'debug' }
      });
      
      this.processes.backend = backend;
      
      // Capture output
      let backendOutput = '';
      let backendErrors = '';
      
      backend.stdout.on('data', (data) => {
        const output = data.toString();
        backendOutput += output;
        console.log(`[BACKEND] ${output.trim()}`);
      });
      
      backend.stderr.on('data', (data) => {
        const error = data.toString();
        backendErrors += error;
        console.log(`[BACKEND ERROR] ${error.trim()}`);
      });
      
      backend.on('error', (error) => {
        console.error('❌ Backend process error:', error.message);
      });
      
      backend.on('exit', (code, signal) => {
        console.log(`🛑 Backend exited with code ${code}, signal ${signal}`);
      });
      
      // Wait for backend to start
      console.log('⏳ Waiting for backend to start...');
      const backendReady = await this.waitForBackend('http://localhost:15021/healthz/ready', 30000);
      
      if (backendReady) {
        console.log('✅ Backend started successfully');
        return { success: true, output: backendOutput, errors: backendErrors };
      } else {
        console.log('❌ Backend failed to start');
        return { success: false, output: backendOutput, errors: backendErrors };
      }
      
    } catch (error) {
      console.error('❌ Backend startup test failed:', error.message);
      throw error;
    }
  }

  /**
   * Build the backend if needed
   */
  async buildBackend() {
    console.log('🔨 Building AgentGateway backend...');
    
    return new Promise((resolve, reject) => {
      const build = spawn('cargo', ['build'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: process.cwd()
      });
      
      build.stdout.on('data', (data) => {
        console.log(`[BUILD] ${data.toString().trim()}`);
      });
      
      build.stderr.on('data', (data) => {
        console.log(`[BUILD ERROR] ${data.toString().trim()}`);
      });
      
      build.on('close', (code) => {
        if (code === 0) {
          console.log('✅ Backend build completed');
          resolve();
        } else {
          reject(new Error(`Build failed with code ${code}`));
        }
      });
    });
  }

  /**
   * Create a minimal test config
   */
  async createTestConfig() {
    console.log('📝 Creating test configuration...');
    
    const config = `# Minimal test configuration for AgentGateway
binds:
- port: 15021
  listeners:
  - routes:
    - backends:
      - host: httpbin.org:80
`;
    
    await fs.writeFile('./test-config.yaml', config);
    console.log('✅ Test configuration created');
  }

  /**
   * Wait for backend to be ready
   */
  async waitForBackend(url, timeout = 30000) {
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeout) {
      try {
        const response = await fetch(url);
        if (response.ok) {
          return true;
        }
      } catch (error) {
        // Backend not ready yet, continue waiting
      }
      
      await new Promise(resolve => setTimeout(resolve, 1000));
      process.stdout.write('.');
    }
    
    console.log('');
    return false;
  }

  /**
   * Test UI startup
   */
  async testUIStartup() {
    console.log('🌐 Testing UI Startup...');
    
    try {
      // Change to UI directory
      const uiDir = path.join(process.cwd(), 'ui');
      
      // Check if package.json exists
      try {
        await fs.access(path.join(uiDir, 'package.json'));
        console.log('✅ UI package.json found');
      } catch (error) {
        throw new Error('UI package.json not found');
      }
      
      // Start UI development server
      console.log('🔄 Starting UI development server...');
      const ui = spawn('npm', ['run', 'dev'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: uiDir,
        env: { ...process.env, PORT: '3000' }
      });
      
      this.processes.ui = ui;
      
      // Capture output
      let uiOutput = '';
      let uiErrors = '';
      
      ui.stdout.on('data', (data) => {
        const output = data.toString();
        uiOutput += output;
        console.log(`[UI] ${output.trim()}`);
      });
      
      ui.stderr.on('data', (data) => {
        const error = data.toString();
        uiErrors += error;
        console.log(`[UI ERROR] ${error.trim()}`);
      });
      
      ui.on('error', (error) => {
        console.error('❌ UI process error:', error.message);
      });
      
      ui.on('exit', (code, signal) => {
        console.log(`🛑 UI exited with code ${code}, signal ${signal}`);
      });
      
      // Wait for UI to start
      console.log('⏳ Waiting for UI to start...');
      const uiReady = await this.waitForBackend('http://localhost:3000/ui', 60000);
      
      if (uiReady) {
        console.log('✅ UI started successfully');
        return { success: true, output: uiOutput, errors: uiErrors };
      } else {
        console.log('❌ UI failed to start');
        return { success: false, output: uiOutput, errors: uiErrors };
      }
      
    } catch (error) {
      console.error('❌ UI startup test failed:', error.message);
      throw error;
    }
  }

  /**
   * Run a single Cypress test
   */
  async runSingleTest() {
    console.log('🧪 Running Single Cypress Test...');
    
    try {
      const uiDir = path.join(process.cwd(), 'ui');
      
      // Run a simple smoke test
      const cypress = spawn('npx', ['cypress', 'run', '--spec', 'cypress/e2e/smoke/*.cy.ts', '--headless'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: uiDir
      });
      
      let cypressOutput = '';
      let cypressErrors = '';
      
      cypress.stdout.on('data', (data) => {
        const output = data.toString();
        cypressOutput += output;
        console.log(`[CYPRESS] ${output.trim()}`);
      });
      
      cypress.stderr.on('data', (data) => {
        const error = data.toString();
        cypressErrors += error;
        console.log(`[CYPRESS ERROR] ${error.trim()}`);
      });
      
      return new Promise((resolve) => {
        cypress.on('close', (code) => {
          const success = code === 0;
          console.log(`${success ? '✅' : '❌'} Cypress test ${success ? 'passed' : 'failed'} (exit code: ${code})`);
          resolve({ success, output: cypressOutput, errors: cypressErrors, exitCode: code });
        });
      });
      
    } catch (error) {
      console.error('❌ Cypress test failed:', error.message);
      throw error;
    }
  }

  /**
   * Cleanup processes
   */
  async cleanup() {
    console.log('🧹 Cleaning up processes...');
    
    if (this.processes.backend) {
      console.log('🛑 Stopping backend...');
      this.processes.backend.kill('SIGTERM');
    }
    
    if (this.processes.ui) {
      console.log('🛑 Stopping UI...');
      this.processes.ui.kill('SIGTERM');
    }
    
    if (this.resourceMonitor) {
      this.resourceMonitor.stopMonitoring();
    }
    
    // Wait a bit for processes to terminate
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    console.log('✅ Cleanup completed');
  }

  /**
   * Generate test report
   */
  generateReport(results) {
    const duration = Date.now() - this.startTime;
    
    const report = {
      timestamp: new Date().toISOString(),
      duration: duration,
      results: results,
      summary: {
        resourceMonitor: results.resourceMonitor ? '✅ PASS' : '❌ FAIL',
        backendStartup: results.backendStartup?.success ? '✅ PASS' : '❌ FAIL',
        uiStartup: results.uiStartup?.success ? '✅ PASS' : '❌ FAIL',
        cypressTest: results.cypressTest?.success ? '✅ PASS' : '❌ FAIL'
      }
    };
    
    console.log('\n📋 Test Report Summary:');
    console.log('═'.repeat(50));
    console.log(`Duration: ${(duration / 1000).toFixed(1)}s`);
    console.log(`Resource Monitor: ${report.summary.resourceMonitor}`);
    console.log(`Backend Startup: ${report.summary.backendStartup}`);
    console.log(`UI Startup: ${report.summary.uiStartup}`);
    console.log(`Cypress Test: ${report.summary.cypressTest}`);
    console.log('═'.repeat(50));
    
    return report;
  }

  /**
   * Run all tests
   */
  async run() {
    console.log('🚀 Starting Minimal E2E Test Suite...');
    console.log('═'.repeat(60));
    
    const results = {};
    
    try {
      // Test 1: Resource Monitor
      try {
        results.resourceMonitor = await this.testResourceMonitor();
      } catch (error) {
        results.resourceMonitor = { error: error.message };
      }
      
      // Test 2: Backend Startup
      try {
        results.backendStartup = await this.testBackendStartup();
      } catch (error) {
        results.backendStartup = { success: false, error: error.message };
      }
      
      // Test 3: UI Startup (only if backend started)
      if (results.backendStartup?.success) {
        try {
          results.uiStartup = await this.testUIStartup();
        } catch (error) {
          results.uiStartup = { success: false, error: error.message };
        }
        
        // Test 4: Single Cypress Test (only if UI started)
        if (results.uiStartup?.success) {
          try {
            results.cypressTest = await this.runSingleTest();
          } catch (error) {
            results.cypressTest = { success: false, error: error.message };
          }
        }
      }
      
      // Generate report
      const report = this.generateReport(results);
      
      // Save report
      await fs.writeFile('minimal-test-report.json', JSON.stringify(report, null, 2));
      console.log('📊 Report saved to minimal-test-report.json');
      
      return report;
      
    } finally {
      await this.cleanup();
    }
  }
}

/**
 * CLI Interface
 */
async function main() {
  const args = process.argv.slice(2);
  const options = {
    resourceOnly: args.includes('--resource-only'),
    backendOnly: args.includes('--backend-only'),
    verbose: args.includes('--verbose')
  };

  try {
    const tester = new MinimalE2ETest();
    
    if (options.resourceOnly) {
      console.log('🔧 Running resource monitor test only...');
      await tester.testResourceMonitor();
    } else if (options.backendOnly) {
      console.log('🚀 Running backend startup test only...');
      await tester.testBackendStartup();
      await tester.cleanup();
    } else {
      await tester.run();
    }
    
    console.log('\n✅ Minimal E2E test completed successfully');
    
  } catch (error) {
    console.error('\n❌ Minimal E2E test failed:', error.message);
    if (options.verbose) {
      console.error(error.stack);
    }
    process.exit(1);
  }
}

// Show help
if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log(`
Minimal E2E Test Script
=======================

Usage: node test-e2e-minimal.js [options]

Options:
  --resource-only    Test only the resource monitor
  --backend-only     Test only backend startup
  --verbose          Show detailed error information
  --help, -h         Show this help message

Examples:
  node test-e2e-minimal.js
  node test-e2e-minimal.js --resource-only
  node test-e2e-minimal.js --backend-only --verbose

This script provides minimal testing to validate fixes and debug issues.
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

module.exports = MinimalE2ETest;
