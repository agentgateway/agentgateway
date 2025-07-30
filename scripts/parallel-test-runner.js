#!/usr/bin/env node

const path = require('path');
const fs = require('fs').promises;
const { program } = require('commander');

const ResourceMonitor = require('./lib/resource-monitor-fixed');
const TestScheduler = require('./lib/test-scheduler');
const WorkerManager = require('./lib/worker-manager');
const { ensureDirectoryExists, REQUIRED_DIRECTORIES } = require('./setup-test-directories.js');

/**
 * Intelligent Parallel Test Runner
 * 
 * Main orchestrator for parallel E2E test execution with:
 * - Resource monitoring and safety
 * - Intelligent test scheduling
 * - Worker management
 * - Consolidated reporting
 */
class ParallelTestRunner {
  constructor(options = {}) {
    this.options = {
      baseDir: options.baseDir || '.',
      maxWorkers: options.maxWorkers || null, // Auto-detect if null
      memoryLimit: options.memoryLimit || 85, // Percentage
      diskSpaceBuffer: options.diskSpaceBuffer || 100 * 1024 * 1024, // 100MB
      strategy: options.strategy || 'balanced',
      browser: options.browser || 'electron',
      headless: options.headless !== false,
      video: options.video !== false,
      quiet: options.quiet !== false,
      debug: options.debug || false,
      ci: options.ci || false,
      dev: options.dev || false,
      ...options
    };
    
    this.resourceMonitor = null;
    this.testScheduler = null;
    this.workerManager = null;
    
    this.startTime = null;
    this.endTime = null;
    this.results = null;
    
    this.isRunning = false;
    this.shutdownHandlers = [];
    
    this.setupSignalHandlers();
  }

  /**
   * Initialize all components
   */
  async initialize() {
    console.log('🚀 Initializing Parallel Test Runner...');
    
    // Setup test directories first
    await this.setupTestDirectories();
    
    // Initialize resource monitor
    this.resourceMonitor = new ResourceMonitor({
      memoryLimit: this.options.memoryLimit,
      diskSpaceBuffer: this.options.diskSpaceBuffer
    });
    
    // Initialize test scheduler
    this.testScheduler = new TestScheduler({
      baseDir: 'cypress/e2e', // Relative to ui directory when script runs from ui
      strategy: this.options.strategy,
      smokeOnly: this.options.smoke,
      journeyFilter: this.options.journeyFilter || null,
      journeyPatterns: this.options.journeyPatterns || null
    });
    
    // Initialize worker manager
    this.workerManager = new WorkerManager({
      baseDir: this.options.baseDir,
      maxWorkers: this.options.maxWorkers || 8
    });
    
    // Set up resource monitoring
    this.setupResourceMonitoring();
    
    console.log('✅ Initialization complete');
  }

  /**
   * Setup test directories to prevent video recording errors
   */
  async setupTestDirectories() {
    console.log('📁 Setting up test directories...');
    
    try {
      // Ensure all required directories exist
      for (const dir of REQUIRED_DIRECTORIES) {
        ensureDirectoryExists(dir);
      }
      console.log('✅ Test directories ready');
    } catch (error) {
      console.error('❌ Failed to setup test directories:', error.message);
      throw error;
    }
  }

  /**
   * Set up resource monitoring and safety handlers
   */
  setupResourceMonitoring() {
    // Start monitoring
    this.resourceMonitor.startMonitoring(5000);
    
    // Set up emergency handlers
    this.resourceMonitor.on('emergency', async (emergency) => {
      console.error(`🚨 EMERGENCY: ${emergency.type} limit exceeded!`);
      await this.emergencyShutdown(emergency);
    });
    
    // Set up warning handlers
    this.resourceMonitor.on('memoryWarning', (memory) => {
      console.warn(`⚠️ Memory usage high: ${memory.percentage.toFixed(1)}%`);
    });
    
    this.resourceMonitor.on('diskWarning', (disk) => {
      console.warn(`⚠️ Disk space low: ${this.resourceMonitor.formatBytes(disk.available)} remaining`);
    });
    
    this.resourceMonitor.on('cpuWarning', (cpu) => {
      console.warn(`⚠️ CPU usage high: ${cpu.percentage.toFixed(1)}%`);
    });
  }

  /**
   * Set up signal handlers for graceful shutdown
   */
  setupSignalHandlers() {
    const signals = ['SIGINT', 'SIGTERM', 'SIGQUIT'];
    
    signals.forEach(signal => {
      process.on(signal, async () => {
        console.log(`\n🛑 Received ${signal}, shutting down gracefully...`);
        await this.shutdown('signal');
        process.exit(0);
      });
    });
    
    process.on('uncaughtException', async (error) => {
      console.error('💥 Uncaught exception:', error);
      await this.shutdown('error');
      process.exit(1);
    });
    
    process.on('unhandledRejection', async (reason, promise) => {
      console.error('💥 Unhandled rejection at:', promise, 'reason:', reason);
      await this.shutdown('error');
      process.exit(1);
    });
  }

  /**
   * Run the parallel tests
   */
  async run() {
    if (this.isRunning) {
      throw new Error('Test runner is already running');
    }
    
    this.isRunning = true;
    this.startTime = new Date();
    
    try {
      console.log('🎯 Starting parallel test execution...');
      this.logSystemInfo();
      
      // Check initial resources
      const initialResources = await this.resourceMonitor.checkResources();
      if (!initialResources.safe) {
        throw new Error('System resources are not safe for test execution');
      }
      
      // Calculate optimal workers
      const optimalWorkers = this.calculateOptimalWorkers();
      console.log(`🔧 Using ${optimalWorkers} workers`);
      
      // Schedule tests
      const schedule = await this.testScheduler.scheduleTests(optimalWorkers, {
        maxMemory: initialResources.memory.free * 0.8, // Use 80% of free memory
        maxWorkers: this.options.maxWorkers
      });
      
      if (schedule.length === 0) {
        throw new Error('No tests scheduled - check test discovery');
      }
      
      // Run workers
      const workerResults = await this.workerManager.runWorkers(schedule, {
        browser: this.options.browser,
        headless: this.options.headless,
        video: this.options.video,
        quiet: this.options.quiet
      });
      
      // Process results
      this.results = await this.processResults(workerResults, schedule);
      
      this.endTime = new Date();
      const duration = this.endTime - this.startTime;
      
      console.log(`🎉 Parallel test execution completed in ${(duration / 1000).toFixed(1)}s`);
      this.logResults();
      
      return this.results;
      
    } catch (error) {
      console.error('❌ Parallel test execution failed:', error.message);
      if (this.options.debug) {
        console.error(error.stack);
      }
      throw error;
    } finally {
      this.isRunning = false;
      await this.cleanup();
    }
  }

  /**
   * Calculate optimal number of workers
   */
  calculateOptimalWorkers() {
    if (this.options.maxWorkers) {
      return this.options.maxWorkers;
    }
    
    const resourceBasedWorkers = this.resourceMonitor.calculateOptimalWorkers();
    
    // Apply environment-specific adjustments
    if (this.options.ci) {
      // CI environments typically have limited resources
      return Math.min(resourceBasedWorkers, 4);
    }
    
    if (this.options.dev) {
      // Development mode - be more conservative
      return Math.min(resourceBasedWorkers, 6);
    }
    
    return resourceBasedWorkers;
  }

  /**
   * Process and consolidate results
   */
  async processResults(workerResults, schedule) {
    // Ensure we have valid timestamps
    const safeStartTime = this.startTime || new Date();
    const safeEndTime = this.endTime || new Date();
    const totalDuration = Math.max(0, safeEndTime - safeStartTime);
    
    // Calculate statistics
    const stats = {
      execution: {
        startTime: safeStartTime,
        endTime: safeEndTime,
        totalDuration,
        parallelEfficiency: this.calculateParallelEfficiency(workerResults, totalDuration)
      },
      workers: {
        total: workerResults.totalWorkers || 0,
        successful: workerResults.successfulWorkers || 0,
        failed: workerResults.failedWorkers || 0,
        errors: workerResults.errorWorkers || 0
      },
      tests: this.aggregateTestResults(workerResults),
      resources: this.resourceMonitor ? this.resourceMonitor.getResourceSummary() : null,
      schedule: this.testScheduler ? this.testScheduler.getScheduleStats(schedule) : null
    };
    
    // Generate reports
    await this.generateReports(stats, workerResults);
    
    return stats;
  }

  /**
   * Calculate parallel execution efficiency
   */
  calculateParallelEfficiency(workerResults, totalDuration) {
    // Safely calculate sequential time from worker results
    let sequentialTime = 0;
    let validResults = 0;
    
    if (workerResults && workerResults.results && Array.isArray(workerResults.results)) {
      for (const result of workerResults.results) {
        if (result && typeof result.duration === 'number' && result.duration > 0) {
          sequentialTime += result.duration;
          validResults++;
        }
      }
    }
    
    // Ensure totalDuration is valid
    const safeTotalDuration = Math.max(totalDuration || 0, 1); // Minimum 1ms to avoid division by zero
    
    // Calculate metrics safely
    const efficiency = sequentialTime > 0 && safeTotalDuration > 0 ? 
      Math.min(sequentialTime / safeTotalDuration, 1) : 0;
    
    const speedup = sequentialTime > 0 && safeTotalDuration > 0 ? 
      sequentialTime / safeTotalDuration : 1;
    
    const timeReduction = Math.max(0, sequentialTime - safeTotalDuration);
    
    const percentageImprovement = sequentialTime > 0 ? 
      Math.max(0, Math.min(100, ((sequentialTime - safeTotalDuration) / sequentialTime) * 100)) : 0;
    
    return {
      efficiency: Math.max(0, Math.min(1, efficiency)), // Ensure 0-1 range
      speedup: Math.max(1, speedup), // Speedup should be at least 1
      timeReduction,
      percentageImprovement: Math.max(0, Math.min(100, percentageImprovement)), // Ensure 0-100 range
      validResults,
      sequentialTime,
      totalDuration: safeTotalDuration
    };
  }

  /**
   * Aggregate test results from all workers
   */
  aggregateTestResults(workerResults) {
    let totalTests = 0;
    let passedTests = 0;
    let failedTests = 0;
    let skippedTests = 0;
    
    // Ensure we have valid worker results
    if (!workerResults || !workerResults.results || !Array.isArray(workerResults.results)) {
      return {
        total: 0,
        passed: 0,
        failed: 0,
        skipped: 0,
        passRate: 0
      };
    }
    
    console.log(`📊 Aggregating results from ${workerResults.results.length} workers...`);
    
    for (const result of workerResults.results) {
      if (result && result.results && result.results.stats) {
        const stats = result.results.stats;
        const workerTests = stats.tests || 0;
        const workerPasses = stats.passes || 0;
        const workerFailures = stats.failures || 0;
        const workerPending = stats.pending || 0;
        
        console.log(`  Worker ${result.workerId}: ${workerPasses}/${workerTests} passed, ${workerFailures} failed, ${workerPending} pending`);
        
        totalTests += workerTests;
        passedTests += workerPasses;
        failedTests += workerFailures;
        skippedTests += workerPending;
      } else {
        console.warn(`  Worker ${result?.workerId || 'unknown'}: No valid results found`);
      }
    }
    
    console.log(`📊 Total aggregated: ${passedTests}/${totalTests} passed, ${failedTests} failed, ${skippedTests} pending`);
    
    return {
      total: totalTests,
      passed: passedTests,
      failed: failedTests,
      skipped: skippedTests,
      passRate: totalTests > 0 ? (passedTests / totalTests) * 100 : 0
    };
  }

  /**
   * Generate consolidated reports
   */
  async generateReports(stats, workerResults) {
    const reportsDir = path.join(this.options.baseDir, 'cypress', 'reports');
    await fs.mkdir(reportsDir, { recursive: true });
    
    // Generate JSON report
    const jsonReport = {
      summary: stats,
      workerResults: workerResults.results,
      timestamp: new Date().toISOString(),
      version: '1.0.0'
    };
    
    await fs.writeFile(
      path.join(reportsDir, 'parallel-test-results.json'),
      JSON.stringify(jsonReport, null, 2)
    );
    
    // Generate simple text summary
    const textSummary = this.generateTextSummary(stats);
    await fs.writeFile(
      path.join(reportsDir, 'parallel-test-summary.txt'),
      textSummary
    );
    
    console.log(`📊 Reports generated in ${reportsDir}`);
  }

  /**
   * Generate text summary
   */
  generateTextSummary(stats) {
    const { execution, workers, tests, resources } = stats;
    
    return `
Parallel Test Execution Summary
===============================

Execution Details:
- Start Time: ${execution.startTime ? execution.startTime.toISOString() : 'N/A'}
- End Time: ${execution.endTime ? execution.endTime.toISOString() : 'N/A'}
- Total Duration: ${(execution.totalDuration / 1000).toFixed(1)}s
- Parallel Efficiency: ${(execution.parallelEfficiency.efficiency * 100).toFixed(1)}%
- Speed Improvement: ${execution.parallelEfficiency.percentageImprovement.toFixed(1)}%

Worker Statistics:
- Total Workers: ${workers.total}
- Successful: ${workers.successful}
- Failed: ${workers.failed}
- Errors: ${workers.errors}

Test Results:
- Total Tests: ${tests.total}
- Passed: ${tests.passed}
- Failed: ${tests.failed}
- Skipped: ${tests.skipped}
- Pass Rate: ${tests.passRate.toFixed(1)}%

Resource Usage:
- Peak Memory: ${resources ? resources.current.memory.percentage.toFixed(1) : 'N/A'}%
- Average CPU: ${resources ? resources.averages.cpu.toFixed(1) : 'N/A'}%
- Optimal Workers: ${resources ? resources.optimalWorkers : 'N/A'}

Generated: ${new Date().toISOString()}
`;
  }

  /**
   * Log system information
   */
  logSystemInfo() {
    const systemInfo = this.resourceMonitor.getSystemInfo();
    
    console.log('\n📋 System Information:');
    console.log('─'.repeat(50));
    console.log(`Platform: ${systemInfo.platform} (${systemInfo.arch})`);
    console.log(`CPUs: ${systemInfo.cpus}`);
    console.log(`Total Memory: ${systemInfo.totalMemory}`);
    console.log(`Memory Limit: ${systemInfo.memoryLimit}`);
    console.log(`Node Version: ${systemInfo.nodeVersion}`);
    console.log('─'.repeat(50));
  }

  /**
   * Log final results
   */
  logResults() {
    if (!this.results) return;
    
    const { execution, workers, tests } = this.results;
    
    console.log('\n🎯 Execution Results:');
    console.log('─'.repeat(60));
    console.log(`Duration: ${(execution.totalDuration / 1000).toFixed(1)}s`);
    console.log(`Workers: ${workers.successful}/${workers.total} successful`);
    console.log(`Tests: ${tests.passed}/${tests.total} passed (${tests.passRate.toFixed(1)}%)`);
    console.log(`Efficiency: ${(execution.parallelEfficiency.efficiency * 100).toFixed(1)}%`);
    console.log(`Speed Improvement: ${execution.parallelEfficiency.percentageImprovement.toFixed(1)}%`);
    console.log('─'.repeat(60));
    
    if (tests.failed > 0) {
      console.log(`❌ ${tests.failed} test(s) failed`);
    } else {
      console.log('✅ All tests passed!');
    }
  }

  /**
   * Emergency shutdown
   */
  async emergencyShutdown(emergency) {
    console.error(`🚨 Emergency shutdown triggered: ${emergency.type}`);
    
    try {
      // Immediately terminate all workers
      await this.workerManager.terminateAllWorkers('emergency');
      
      // Stop resource monitoring
      this.resourceMonitor.stopMonitoring();
      
      // Save partial results if possible
      if (this.workerManager.workerResults.size > 0) {
        console.log('💾 Saving partial results...');
        const partialResults = this.workerManager.consolidateResults();
        await this.savePartialResults(partialResults);
      }
      
    } catch (error) {
      console.error('❌ Error during emergency shutdown:', error.message);
    }
    
    process.exit(1);
  }

  /**
   * Save partial results during emergency
   */
  async savePartialResults(results) {
    try {
      const reportsDir = path.join(this.options.baseDir, 'cypress', 'reports');
      await fs.mkdir(reportsDir, { recursive: true });
      
      const partialReport = {
        type: 'partial',
        timestamp: new Date().toISOString(),
        results,
        reason: 'emergency_shutdown'
      };
      
      await fs.writeFile(
        path.join(reportsDir, 'partial-results.json'),
        JSON.stringify(partialReport, null, 2)
      );
      
      console.log('💾 Partial results saved');
    } catch (error) {
      console.error('❌ Could not save partial results:', error.message);
    }
  }

  /**
   * Graceful shutdown
   */
  async shutdown(reason = 'manual') {
    console.log(`🛑 Shutting down (reason: ${reason})...`);
    
    try {
      // Stop resource monitoring
      if (this.resourceMonitor) {
        this.resourceMonitor.stopMonitoring();
      }
      
      // Terminate workers
      if (this.workerManager) {
        await this.workerManager.terminateAllWorkers(reason);
      }
      
      // Cleanup
      await this.cleanup();
      
      console.log('✅ Shutdown complete');
    } catch (error) {
      console.error('❌ Error during shutdown:', error.message);
    }
  }

  /**
   * Cleanup resources
   */
  async cleanup() {
    try {
      if (this.workerManager) {
        await this.workerManager.cleanup();
      }
    } catch (error) {
      console.warn('⚠️ Cleanup warning:', error.message);
    }
  }
}

/**
 * CLI Interface
 */
async function main() {
  program
    .name('parallel-test-runner')
    .description('Intelligent parallel E2E test runner for AgentGateway')
    .version('1.0.0')
    .option('--workers <number>', 'Maximum number of workers', parseInt)
    .option('--strategy <type>', 'Scheduling strategy (balanced, fastest, priority)', 'balanced')
    .option('--browser <name>', 'Browser to use (electron, chrome, firefox, edge)', 'electron')
    .option('--headed', 'Run in headed mode (visible browser)')
    .option('--no-headless', 'Run in headed mode')
    .option('--no-video', 'Disable video recording')
    .option('--no-quiet', 'Enable verbose output')
    .option('--debug', 'Enable debug mode')
    .option('--ci', 'CI environment mode')
    .option('--dev', 'Development mode')
    .option('--smoke', 'Run only smoke tests')
    .option('--memory-limit <percent>', 'Memory usage limit percentage', parseFloat, 85)
    .option('--disk-buffer <mb>', 'Disk space buffer in MB', parseFloat, 100);

  program.parse();
  
  const options = program.opts();
  
  try {
    const runner = new ParallelTestRunner({
      maxWorkers: options.workers,
      strategy: options.strategy,
      browser: options.browser,
      headless: options.headless,
      video: options.video,
      quiet: options.quiet,
      debug: options.debug,
      ci: options.ci,
      dev: options.dev,
      smoke: options.smoke,
      memoryLimit: options.memoryLimit,
      diskSpaceBuffer: options.diskBuffer * 1024 * 1024
    });
    
    await runner.initialize();
    const results = await runner.run();
    
    // Exit with appropriate code
    const exitCode = results.tests.failed > 0 ? 1 : 0;
    process.exit(exitCode);
    
  } catch (error) {
    console.error('💥 Fatal error:', error.message);
    if (options.debug) {
      console.error(error.stack);
    }
    process.exit(1);
  }
}

// Run if called directly
if (require.main === module) {
  main().catch(error => {
    console.error('💥 Unhandled error:', error);
    process.exit(1);
  });
}

module.exports = ParallelTestRunner;
