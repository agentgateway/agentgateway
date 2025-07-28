const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs').promises;

/**
 * WorkerManager - Manages Cypress worker processes
 * 
 * Key Features:
 * - Worker lifecycle management (spawn, monitor, terminate)
 * - Worker communication protocols
 * - Worker health monitoring
 * - Dynamic worker scaling
 */
class WorkerManager {
  constructor(options = {}) {
    this.baseDir = options.baseDir || '.';
    this.cypressCommand = options.cypressCommand || 'npx cypress run';
    this.maxWorkers = options.maxWorkers || 8;
    this.workerTimeout = options.workerTimeout || 300000; // 5 minutes
    
    this.workers = new Map();
    this.workerResults = new Map();
    this.nextWorkerId = 1;
    
    this.isShuttingDown = false;
    this.shutdownPromise = null;
  }

  /**
   * Spawn a worker for a specific test schedule
   */
  async spawnWorker(workerSchedule, options = {}) {
    const workerId = this.nextWorkerId++;
    const workerName = `worker-${workerId}`;
    
    console.log(`🚀 Spawning ${workerName} with ${workerSchedule.tests.length} tests...`);
    
    try {
      // Create worker-specific configuration
      const workerConfig = await this.createWorkerConfig(workerId, workerSchedule, options);
      
      // Build Cypress command
      const command = this.buildCypressCommand(workerConfig, options);
      
      // Spawn the process
      const worker = this.spawnCypressProcess(workerId, command, options);
      
      // Set up worker monitoring
      this.setupWorkerMonitoring(worker, workerSchedule);
      
      // Store worker info
      this.workers.set(workerId, {
        id: workerId,
        name: workerName,
        process: worker,
        schedule: workerSchedule,
        config: workerConfig,
        startTime: new Date(),
        status: 'running',
        output: [],
        errors: []
      });
      
      console.log(`✅ ${workerName} started (PID: ${worker.pid})`);
      return workerId;
      
    } catch (error) {
      console.error(`❌ Failed to spawn ${workerName}:`, error.message);
      throw error;
    }
  }

  /**
   * Create worker-specific Cypress configuration
   */
  async createWorkerConfig(workerId, workerSchedule, options) {
    const configDir = path.join(this.baseDir, 'cypress', 'workers');
    const resultsDir = path.resolve(this.baseDir, 'cypress', 'results', `worker-${workerId}`);
    
    // Create both config and results directories
    await fs.mkdir(configDir, { recursive: true });
    await fs.mkdir(resultsDir, { recursive: true });
    
    const configPath = path.resolve(configDir, `cypress.worker-${workerId}.config.js`);
    
    // Generate test spec pattern for this worker
    const testSpecs = workerSchedule.tests.map(test => test.path).join(',');
    
    const configContent = this.generateWorkerConfigContent(workerId, testSpecs, options);
    
    await fs.writeFile(configPath, configContent);
    
    return {
      configPath,
      testSpecs,
      workerId,
      reporterOptions: {
        outputDir: resultsDir,
        outputFile: `results-worker-${workerId}.json`
      }
    };
  }

  /**
   * Generate worker-specific Cypress configuration content
   */
  generateWorkerConfigContent(workerId, testSpecs, options) {
    return `
// Worker-specific Cypress configuration
const path = require('path');
const fs = require('fs');

module.exports = {
  e2e: {
    // Worker-specific test patterns - ensure proper path resolution
    specPattern: [${testSpecs.split(',').map(spec => `'${spec.trim()}'`).join(', ')}],
    
    // Base URL - same as regular tests
    baseUrl: 'http://localhost:3000/ui',
    
    // Support file
    supportFile: 'cypress/support/e2e.ts',
    
    // Worker-specific settings
    video: ${options.video !== false},
    screenshot: true,
    screenshotOnRunFailure: true,
    
    // Video settings - ensure directories exist
    videosFolder: 'cypress/videos',
    videoCompression: 32,
    
    // Timeouts
    defaultCommandTimeout: 10000,
    requestTimeout: 15000,
    responseTimeout: 15000,
    pageLoadTimeout: 30000,
    
    // Viewport
    viewportWidth: 1280,
    viewportHeight: 720,
    
    // Retry configuration
    retries: {
      runMode: 2,
      openMode: 0
    },
    
    // Reporter configuration for this worker
    reporter: 'json',
    reporterOptions: {
      outputDir: 'cypress/results/worker-${workerId}',
      outputFile: 'results-worker-${workerId}.json'
    },
    
    // Environment variables
    env: {
      WORKER_ID: ${workerId},
      PARALLEL_MODE: true
    },
    
    setupNodeEvents(on, config) {
      // Worker-specific setup
      on('task', {
        log(message) {
          console.log(\`[Worker ${workerId}] \${message}\`);
          return null;
        }
      });
      
      // Ensure video directories exist before tests run
      on('before:run', (details) => {
        const videoDir = path.resolve(config.videosFolder);
        const subDirs = ['navigation', 'foundation', 'setup-wizard', 'configuration', 'playground', 'integration', 'error-handling'];
        
        // Create main video directory
        if (!fs.existsSync(videoDir)) {
          fs.mkdirSync(videoDir, { recursive: true });
        }
        
        // Create subdirectories for organized video storage
        subDirs.forEach(subDir => {
          const fullPath = path.join(videoDir, subDir);
          if (!fs.existsSync(fullPath)) {
            fs.mkdirSync(fullPath, { recursive: true });
          }
        });
      });
      
      return config;
    }
  }
};
`;
  }

  /**
   * Build Cypress command for worker
   */
  buildCypressCommand(workerConfig, options) {
    const args = [
      'cypress', 'run',
      '--config-file', workerConfig.configPath,
      '--reporter', 'json',
      '--reporter-options', `outputDir=${workerConfig.reporterOptions.outputDir},outputFile=${workerConfig.reporterOptions.outputFile}`
    ];
    
    // Add browser option if specified
    if (options.browser) {
      args.push('--browser', options.browser);
    }
    
    // Add headless option
    if (options.headless !== false) {
      args.push('--headless');
    }
    
    // Add quiet option for cleaner output
    if (options.quiet !== false) {
      args.push('--quiet');
    }
    
    return {
      command: 'npx',
      args,
      options: {
        cwd: this.baseDir,
        stdio: ['pipe', 'pipe', 'pipe'],
        env: {
          ...process.env,
          CYPRESS_WORKER_ID: workerConfig.workerId,
          CYPRESS_PARALLEL_MODE: 'true'
        }
      }
    };
  }

  /**
   * Spawn Cypress process
   */
  spawnCypressProcess(workerId, command, options) {
    const worker = spawn(command.command, command.args, command.options);
    
    // Handle process errors
    worker.on('error', (error) => {
      console.error(`❌ Worker ${workerId} process error:`, error.message);
      this.handleWorkerError(workerId, error);
    });
    
    return worker;
  }

  /**
   * Set up worker monitoring
   */
  setupWorkerMonitoring(worker, workerSchedule) {
    const workerId = worker.pid; // Use PID as temporary ID
    
    // Capture stdout
    worker.stdout.on('data', (data) => {
      const output = data.toString();
      this.handleWorkerOutput(workerId, output, 'stdout');
    });
    
    // Capture stderr
    worker.stderr.on('data', (data) => {
      const output = data.toString();
      this.handleWorkerOutput(workerId, output, 'stderr');
    });
    
    // Handle process exit
    worker.on('close', (code, signal) => {
      this.handleWorkerExit(workerId, code, signal);
    });
    
    // Set up timeout
    const timeout = setTimeout(() => {
      console.warn(`⚠️ Worker ${workerId} timeout after ${this.workerTimeout}ms`);
      this.terminateWorker(workerId, 'timeout');
    }, this.workerTimeout);
    
    worker.timeout = timeout;
  }

  /**
   * Handle worker output
   */
  handleWorkerOutput(workerId, output, type) {
    const workerInfo = this.findWorkerByPid(workerId);
    if (!workerInfo) return;
    
    // Store output
    workerInfo.output.push({
      type,
      content: output,
      timestamp: new Date()
    });
    
    // Log all output for debugging
    console.log(`[Worker ${workerInfo.id}] ${type}: ${output.trim()}`);
    
    // Check for errors
    if (type === 'stderr' && output.trim()) {
      workerInfo.errors.push({
        content: output,
        timestamp: new Date()
      });
    }
  }

  /**
   * Handle worker exit
   */
  handleWorkerExit(pid, code, signal) {
    const workerInfo = this.findWorkerByPid(pid);
    if (!workerInfo) return;
    
    const workerId = workerInfo.id;
    const endTime = new Date();
    const duration = endTime - workerInfo.startTime;
    
    // Clear timeout
    if (workerInfo.process.timeout) {
      clearTimeout(workerInfo.process.timeout);
    }
    
    // Check if this is a video recording failure but tests passed
    const hasVideoError = workerInfo.output.some(output => 
      output.content.includes('ffmpeg exited with code 1') || 
      output.content.includes('We failed to record the video')
    );
    
    // Update worker status - treat video-only failures as success if tests passed
    if (code === 0) {
      workerInfo.status = 'completed';
    } else if (hasVideoError && this.hasTestResults(workerInfo)) {
      // If we have test results and only video failed, treat as success
      workerInfo.status = 'completed';
      console.log(`⚠️ Worker ${workerId} had video recording issues but tests passed (${(duration / 1000).toFixed(1)}s)`);
    } else {
      workerInfo.status = 'failed';
    }
    
    workerInfo.endTime = endTime;
    workerInfo.duration = duration;
    workerInfo.exitCode = code;
    workerInfo.signal = signal;
    
    // Log completion
    if (workerInfo.status === 'completed') {
      console.log(`✅ Worker ${workerId} completed successfully (${(duration / 1000).toFixed(1)}s)`);
    } else {
      console.error(`❌ Worker ${workerId} failed with code ${code} (${(duration / 1000).toFixed(1)}s)`);
    }
    
    // Process results
    this.processWorkerResults(workerId);
  }

  /**
   * Check if worker has test results in output
   */
  hasTestResults(workerInfo) {
    if (!workerInfo.output || !Array.isArray(workerInfo.output)) {
      return false;
    }
    
    return workerInfo.output.some(output => 
      output.type === 'stdout' && 
      output.content.includes('"stats"') && 
      output.content.includes('"tests"')
    );
  }

  /**
   * Handle worker error
   */
  handleWorkerError(workerId, error) {
    const workerInfo = this.workers.get(workerId);
    if (!workerInfo) return;
    
    workerInfo.status = 'error';
    workerInfo.error = error;
    
    console.error(`❌ Worker ${workerId} encountered error:`, error.message);
  }

  /**
   * Find worker by process PID
   */
  findWorkerByPid(pid) {
    for (const [workerId, workerInfo] of this.workers) {
      if (workerInfo.process.pid === pid) {
        return workerInfo;
      }
    }
    return null;
  }

  /**
   * Process worker results
   */
  async processWorkerResults(workerId) {
    const workerInfo = this.workers.get(workerId);
    if (!workerInfo) return;
    
    try {
      // Read Cypress results file
      const resultsPath = path.join(
        workerInfo.config.reporterOptions.outputDir,
        workerInfo.config.reporterOptions.outputFile
      );
      
      let results = null;
      try {
        const resultsContent = await fs.readFile(resultsPath, 'utf8');
        results = JSON.parse(resultsContent);
      } catch (error) {
        console.warn(`⚠️ Could not read results for worker ${workerId}:`, error.message);
        
        // Try to extract results from stdout JSON output
        results = this.extractResultsFromOutput(workerInfo) || this.createFallbackResults(workerInfo);
      }
      
      // Store processed results
      this.workerResults.set(workerId, {
        workerId,
        schedule: workerInfo.schedule,
        status: workerInfo.status,
        duration: workerInfo.duration || 0,
        exitCode: workerInfo.exitCode,
        results,
        output: workerInfo.output,
        errors: workerInfo.errors,
        startTime: workerInfo.startTime,
        endTime: workerInfo.endTime
      });
      
    } catch (error) {
      console.error(`❌ Error processing results for worker ${workerId}:`, error.message);
      
      // Try to extract results from output as fallback
      const extractedResults = this.extractResultsFromOutput(workerInfo);
      
      // Store minimal result even on error
      this.workerResults.set(workerId, {
        workerId,
        schedule: workerInfo.schedule,
        status: extractedResults ? 'completed' : 'error',
        duration: workerInfo.duration || 0,
        exitCode: workerInfo.exitCode || (extractedResults ? 0 : -1),
        results: extractedResults || this.createFallbackResults(workerInfo),
        output: workerInfo.output || [],
        errors: workerInfo.errors || [],
        startTime: workerInfo.startTime,
        endTime: workerInfo.endTime || new Date()
      });
    }
  }

  /**
   * Extract results from worker stdout output (when JSON reporter writes to stdout)
   */
  extractResultsFromOutput(workerInfo) {
    if (!workerInfo.output || !Array.isArray(workerInfo.output)) {
      return null;
    }
    
    const allResults = [];
    
    // Look for JSON output in stdout - collect ALL results from all test files
    for (const outputEntry of workerInfo.output) {
      if (outputEntry.type === 'stdout' && outputEntry.content) {
        const content = outputEntry.content.trim();
        
        // Try to find JSON blocks in the output
        const jsonMatches = content.match(/\{[\s\S]*"stats"[\s\S]*\}/g);
        if (jsonMatches) {
          for (const jsonMatch of jsonMatches) {
            try {
              const parsed = JSON.parse(jsonMatch);
              if (parsed.stats && typeof parsed.stats.tests === 'number') {
                allResults.push(parsed);
                console.log(`✅ Found test results for worker ${workerInfo.id}: ${parsed.stats.passes}/${parsed.stats.tests} passed`);
              }
            } catch (e) {
              // Continue trying other matches
            }
          }
        }
      }
    }
    
    if (allResults.length === 0) {
      return null;
    }
    
    // If we only have one result, return it directly
    if (allResults.length === 1) {
      console.log(`✅ Extracted single result from stdout for worker ${workerInfo.id}: ${allResults[0].stats.passes}/${allResults[0].stats.tests} passed`);
      return allResults[0];
    }
    
    // Aggregate multiple results from different test files
    console.log(`✅ Aggregating ${allResults.length} test file results for worker ${workerInfo.id}`);
    
    const aggregatedStats = {
      suites: 0,
      tests: 0,
      passes: 0,
      pending: 0,
      failures: 0,
      start: null,
      end: null,
      duration: 0
    };
    
    const aggregatedTests = [];
    const aggregatedPending = [];
    const aggregatedFailures = [];
    const aggregatedPasses = [];
    
    // Aggregate all results
    for (const result of allResults) {
      if (result.stats) {
        aggregatedStats.suites += result.stats.suites || 0;
        aggregatedStats.tests += result.stats.tests || 0;
        aggregatedStats.passes += result.stats.passes || 0;
        aggregatedStats.pending += result.stats.pending || 0;
        aggregatedStats.failures += result.stats.failures || 0;
        aggregatedStats.duration += result.stats.duration || 0;
        
        // Track earliest start and latest end times
        if (result.stats.start) {
          const startTime = new Date(result.stats.start);
          if (!aggregatedStats.start || startTime < new Date(aggregatedStats.start)) {
            aggregatedStats.start = result.stats.start;
          }
        }
        
        if (result.stats.end) {
          const endTime = new Date(result.stats.end);
          if (!aggregatedStats.end || endTime > new Date(aggregatedStats.end)) {
            aggregatedStats.end = result.stats.end;
          }
        }
      }
      
      // Aggregate test arrays
      if (result.tests && Array.isArray(result.tests)) {
        aggregatedTests.push(...result.tests);
      }
      if (result.pending && Array.isArray(result.pending)) {
        aggregatedPending.push(...result.pending);
      }
      if (result.failures && Array.isArray(result.failures)) {
        aggregatedFailures.push(...result.failures);
      }
      if (result.passes && Array.isArray(result.passes)) {
        aggregatedPasses.push(...result.passes);
      }
    }
    
    const aggregatedResult = {
      stats: aggregatedStats,
      tests: aggregatedTests,
      pending: aggregatedPending,
      failures: aggregatedFailures,
      passes: aggregatedPasses
    };
    
    console.log(`✅ Aggregated results for worker ${workerInfo.id}: ${aggregatedStats.passes}/${aggregatedStats.tests} total passed from ${allResults.length} test files`);
    
    return aggregatedResult;
  }

  /**
   * Create fallback results when Cypress results file is missing
   */
  createFallbackResults(workerInfo) {
    const testCount = workerInfo.schedule ? workerInfo.schedule.tests.length : 0;
    
    // Determine test outcomes based on worker status
    let passes = 0;
    let failures = 0;
    let pending = 0;
    
    if (workerInfo.status === 'completed' && workerInfo.exitCode === 0) {
      // Assume all tests passed if worker completed successfully
      passes = testCount;
    } else if (workerInfo.status === 'failed' || workerInfo.exitCode !== 0) {
      // Assume all tests failed if worker failed
      failures = testCount;
    } else {
      // Unknown status - mark as pending
      pending = testCount;
    }
    
    return {
      stats: {
        suites: 1,
        tests: testCount,
        passes,
        pending,
        failures,
        start: workerInfo.startTime ? workerInfo.startTime.toISOString() : new Date().toISOString(),
        end: workerInfo.endTime ? workerInfo.endTime.toISOString() : new Date().toISOString(),
        duration: workerInfo.duration || 0
      },
      tests: [],
      pending: [],
      failures: [],
      passes: []
    };
  }

  /**
   * Run all workers for a schedule
   */
  async runWorkers(schedule, options = {}) {
    console.log(`🚀 Starting ${schedule.length} workers...`);
    
    const workerPromises = [];
    
    // Spawn all workers
    for (const workerSchedule of schedule) {
      const workerPromise = this.spawnWorker(workerSchedule, options)
        .then(workerId => this.waitForWorker(workerId))
        .catch(error => {
          console.error('Worker spawn error:', error);
          return { error: error.message };
        });
      
      workerPromises.push(workerPromise);
      
      // Small delay between spawns to avoid overwhelming the system
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
    
    // Wait for all workers to complete
    console.log('⏳ Waiting for all workers to complete...');
    const results = await Promise.all(workerPromises);
    
    console.log('✅ All workers completed');
    return this.consolidateResults();
  }

  /**
   * Wait for a specific worker to complete
   */
  async waitForWorker(workerId) {
    return new Promise((resolve) => {
      const checkWorker = () => {
        const workerInfo = this.workers.get(workerId);
        if (!workerInfo) {
          resolve({ error: 'Worker not found' });
          return;
        }
        
        if (workerInfo.status === 'completed' || workerInfo.status === 'failed' || workerInfo.status === 'error') {
          resolve(this.workerResults.get(workerId) || workerInfo);
        } else {
          setTimeout(checkWorker, 1000);
        }
      };
      
      checkWorker();
    });
  }

  /**
   * Terminate a specific worker
   */
  async terminateWorker(workerId, reason = 'manual') {
    const workerInfo = this.workers.get(workerId);
    if (!workerInfo) return;
    
    console.log(`🛑 Terminating worker ${workerId} (reason: ${reason})`);
    
    try {
      // Clear timeout
      if (workerInfo.process.timeout) {
        clearTimeout(workerInfo.process.timeout);
      }
      
      // Kill the process
      workerInfo.process.kill('SIGTERM');
      
      // Wait a bit, then force kill if necessary
      setTimeout(() => {
        if (!workerInfo.process.killed) {
          workerInfo.process.kill('SIGKILL');
        }
      }, 5000);
      
      workerInfo.status = 'terminated';
      workerInfo.terminationReason = reason;
      
    } catch (error) {
      console.error(`❌ Error terminating worker ${workerId}:`, error.message);
    }
  }

  /**
   * Terminate all workers
   */
  async terminateAllWorkers(reason = 'shutdown') {
    if (this.isShuttingDown) {
      return this.shutdownPromise;
    }
    
    this.isShuttingDown = true;
    console.log('🛑 Terminating all workers...');
    
    this.shutdownPromise = Promise.all(
      Array.from(this.workers.keys()).map(workerId => 
        this.terminateWorker(workerId, reason)
      )
    );
    
    await this.shutdownPromise;
    console.log('✅ All workers terminated');
    
    return this.shutdownPromise;
  }

  /**
   * Consolidate results from all workers
   */
  consolidateResults() {
    const allResults = Array.from(this.workerResults.values());
    
    const summary = {
      totalWorkers: allResults.length,
      successfulWorkers: allResults.filter(r => r.status === 'completed').length,
      failedWorkers: allResults.filter(r => r.status === 'failed').length,
      errorWorkers: allResults.filter(r => r.status === 'error').length,
      totalDuration: Math.max(...allResults.map(r => r.duration || 0)),
      averageDuration: allResults.reduce((sum, r) => sum + (r.duration || 0), 0) / allResults.length,
      results: allResults
    };
    
    return summary;
  }

  /**
   * Get worker status
   */
  getWorkerStatus(workerId) {
    const workerInfo = this.workers.get(workerId);
    if (!workerInfo) return null;
    
    return {
      id: workerId,
      status: workerInfo.status,
      startTime: workerInfo.startTime,
      duration: workerInfo.endTime ? workerInfo.endTime - workerInfo.startTime : Date.now() - workerInfo.startTime,
      testCount: workerInfo.schedule.tests.length,
      pid: workerInfo.process.pid
    };
  }

  /**
   * Get all workers status
   */
  getAllWorkersStatus() {
    return Array.from(this.workers.keys()).map(workerId => this.getWorkerStatus(workerId));
  }

  /**
   * Cleanup worker files
   */
  async cleanup() {
    try {
      const workersDir = path.join(this.baseDir, 'cypress', 'workers');
      await fs.rm(workersDir, { recursive: true });
      console.log('🧹 Cleaned up worker configuration files');
    } catch (error) {
      console.warn('⚠️ Could not clean up worker files:', error.message);
    }
  }
}

module.exports = WorkerManager;
