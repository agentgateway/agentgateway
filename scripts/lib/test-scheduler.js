const path = require('path');
const fs = require('fs').promises;
const glob = require('glob');

/**
 * TestScheduler - Intelligent test distribution and load balancing
 * 
 * Key Features:
 * - Test grouping by execution characteristics
 * - Load balancing across workers
 * - Priority-based scheduling
 * - Bin-packing optimization
 */
class TestScheduler {
  constructor(options = {}) {
    this.testGroups = options.testGroups || this.getDefaultTestGroups();
    this.baseDir = options.baseDir || 'ui/cypress/e2e';
    this.testHistory = options.testHistory || {};
    this.smokeOnly = options.smokeOnly || false;
    this.journeyFilter = options.journeyFilter || null;
    this.journeyPatterns = options.journeyPatterns || null;
    
    this.schedulingStrategy = options.strategy || 'balanced'; // balanced, fastest, priority
    this.maxWorkersPerGroup = options.maxWorkersPerGroup || 8;
  }

  /**
   * Default test group configuration
   */
  getDefaultTestGroups() {
    return {
      smoke: {
        patterns: [
          'smoke/*.cy.ts'
        ],
        maxWorkers: 4,
        estimatedTimePerTest: 1, // seconds
        priority: 'critical',
        memoryPerWorker: 250 * 1024 * 1024 // 250MB
      },
      fast: {
        patterns: [
          'foundation/*.cy.ts',
          'navigation/*.cy.ts'
        ],
        maxWorkers: 6,
        estimatedTimePerTest: 2, // seconds
        priority: 'high',
        memoryPerWorker: 300 * 1024 * 1024 // 300MB
      },
      medium: {
        patterns: [
          'setup-wizard/*.cy.ts',
          'configuration/*.cy.ts'
        ],
        maxWorkers: 3,
        estimatedTimePerTest: 8, // seconds
        priority: 'medium',
        memoryPerWorker: 400 * 1024 * 1024 // 400MB
      },
      slow: {
        patterns: [
          'integration/*.cy.ts',
          'playground/*.cy.ts',
          'error-handling/*.cy.ts'
        ],
        maxWorkers: 2,
        estimatedTimePerTest: 20, // seconds
        priority: 'low',
        memoryPerWorker: 500 * 1024 * 1024 // 500MB
      }
    };
  }

  /**
   * Discover all test files
   */
  async discoverTestFiles() {
    const testFiles = [];
    
    console.log(`🔍 Searching for tests in: ${this.baseDir}`);
    if (this.smokeOnly) {
      console.log('🚀 Smoke test mode - only running smoke tests');
    }
    if (this.journeyFilter) {
      console.log(`🎯 Journey filter active: ${this.journeyFilter}`);
    }
    
    // Use journey patterns if provided, otherwise use default test groups
    if (this.journeyPatterns && this.journeyPatterns.length > 0) {
      return await this.discoverJourneyTests();
    }
    
    for (const [groupName, group] of Object.entries(this.testGroups)) {
      // Skip non-smoke groups if smokeOnly is enabled
      if (this.smokeOnly && groupName !== 'smoke') {
        console.log(`📂 Group ${groupName}: Skipped (smoke mode)`);
        continue;
      }
      
      console.log(`📂 Group ${groupName}:`);
      for (const pattern of group.patterns) {
        const fullPattern = path.join(this.baseDir, pattern);
        console.log(`  Pattern: ${fullPattern}`);
        const files = glob.sync(fullPattern);
        console.log(`  Found ${files.length} files: ${files.join(', ')}`);
        
        for (const file of files) {
          const relativePath = path.relative(process.cwd(), file);
          const testInfo = await this.analyzeTestFile(relativePath, groupName, group);
          testFiles.push(testInfo);
        }
      }
    }
    
    console.log(`📁 Total discovered: ${testFiles.length} test files`);
    return testFiles;
  }

  /**
   * Discover test files based on journey patterns
   */
  async discoverJourneyTests() {
    const testFiles = [];
    
    console.log(`🎯 Discovering tests for journey patterns: ${this.journeyPatterns.join(', ')}`);
    
    for (const pattern of this.journeyPatterns) {
      const fullPattern = path.join(this.baseDir, pattern);
      console.log(`  Journey pattern: ${fullPattern}`);
      const files = glob.sync(fullPattern);
      console.log(`  Found ${files.length} files: ${files.join(', ')}`);
      
      for (const file of files) {
        const relativePath = path.relative(process.cwd(), file);
        
        // Determine group based on file path
        const groupName = this.determineGroupFromPath(relativePath);
        const groupConfig = this.testGroups[groupName] || this.testGroups.medium; // Default to medium
        
        const testInfo = await this.analyzeTestFile(relativePath, groupName, groupConfig);
        testFiles.push(testInfo);
      }
    }
    
    console.log(`📁 Total journey tests discovered: ${testFiles.length} test files`);
    return testFiles;
  }

  /**
   * Determine test group from file path
   */
  determineGroupFromPath(filePath) {
    const pathLower = filePath.toLowerCase();
    
    if (pathLower.includes('smoke')) {
      return 'smoke';
    } else if (pathLower.includes('foundation') || pathLower.includes('navigation')) {
      return 'fast';
    } else if (pathLower.includes('setup-wizard') || pathLower.includes('configuration')) {
      return 'medium';
    } else if (pathLower.includes('integration') || pathLower.includes('playground') || pathLower.includes('error-handling')) {
      return 'slow';
    }
    
    // Default to medium for unknown paths
    return 'medium';
  }

  /**
   * Analyze individual test file
   */
  async analyzeTestFile(filePath, groupName, groupConfig) {
    try {
      const content = await fs.readFile(filePath, 'utf8');
      const testCount = this.countTests(content);
      const estimatedTime = testCount * groupConfig.estimatedTimePerTest;
      
      // Get historical data if available
      const historicalTime = this.testHistory[filePath]?.averageTime || estimatedTime;
      
      return {
        path: filePath,
        relativePath: path.relative(this.baseDir, filePath),
        group: groupName,
        groupConfig,
        testCount,
        estimatedTime,
        historicalTime,
        actualTime: historicalTime, // Use historical if available
        priority: this.getPriorityScore(groupConfig.priority),
        memoryRequirement: groupConfig.memoryPerWorker,
        dependencies: this.analyzeDependencies(content)
      };
    } catch (error) {
      console.warn(`⚠️ Could not analyze test file ${filePath}:`, error.message);
      return {
        path: filePath,
        relativePath: path.relative(this.baseDir, filePath),
        group: groupName,
        groupConfig,
        testCount: 1,
        estimatedTime: groupConfig.estimatedTimePerTest,
        historicalTime: groupConfig.estimatedTimePerTest,
        actualTime: groupConfig.estimatedTimePerTest,
        priority: this.getPriorityScore(groupConfig.priority),
        memoryRequirement: groupConfig.memoryPerWorker,
        dependencies: []
      };
    }
  }

  /**
   * Count tests in a file
   */
  countTests(content) {
    // Count 'it(' and 'test(' occurrences
    const itMatches = content.match(/\bit\s*\(/g) || [];
    const testMatches = content.match(/\btest\s*\(/g) || [];
    return itMatches.length + testMatches.length;
  }

  /**
   * Analyze test dependencies
   */
  analyzeDependencies(content) {
    const dependencies = [];
    
    // Look for beforeEach, before, setup patterns that might indicate dependencies
    if (content.includes('beforeEach')) {
      dependencies.push('setup');
    }
    
    if (content.includes('cy.visit')) {
      dependencies.push('navigation');
    }
    
    if (content.includes('wizard')) {
      dependencies.push('wizard-state');
    }
    
    return dependencies;
  }

  /**
   * Convert priority string to numeric score
   */
  getPriorityScore(priority) {
    const scores = {
      'critical': 4,
      'high': 3,
      'medium': 2,
      'low': 1
    };
    return scores[priority] || 2;
  }

  /**
   * Schedule tests across available workers
   */
  async scheduleTests(availableWorkers, resourceConstraints = {}) {
    console.log(`📋 Scheduling tests across ${availableWorkers} workers...`);
    
    const testFiles = await this.discoverTestFiles();
    console.log(`📁 Discovered ${testFiles.length} test files`);
    
    if (testFiles.length === 0) {
      throw new Error('No test files found to schedule');
    }
    
    // Apply resource constraints
    const constrainedWorkers = this.applyResourceConstraints(availableWorkers, testFiles, resourceConstraints);
    
    // Choose scheduling strategy
    let schedule;
    switch (this.schedulingStrategy) {
      case 'fastest':
        schedule = this.scheduleFastest(testFiles, constrainedWorkers);
        break;
      case 'priority':
        schedule = this.schedulePriority(testFiles, constrainedWorkers);
        break;
      case 'balanced':
      default:
        schedule = this.scheduleBalanced(testFiles, constrainedWorkers);
        break;
    }
    
    // Validate and optimize schedule
    schedule = this.optimizeSchedule(schedule);
    
    console.log(`✅ Scheduled ${testFiles.length} tests across ${schedule.length} workers`);
    this.logScheduleSummary(schedule);
    
    return schedule;
  }

  /**
   * Apply resource constraints to worker allocation
   */
  applyResourceConstraints(availableWorkers, testFiles, constraints) {
    const { maxMemory, maxWorkers } = constraints;
    
    let constrainedWorkers = availableWorkers;
    
    // Apply memory constraints
    if (maxMemory) {
      const avgMemoryPerWorker = testFiles.reduce((sum, test) => sum + test.memoryRequirement, 0) / testFiles.length;
      const memoryBasedWorkers = Math.floor(maxMemory / avgMemoryPerWorker);
      constrainedWorkers = Math.min(constrainedWorkers, memoryBasedWorkers);
    }
    
    // Apply explicit worker limit
    if (maxWorkers) {
      constrainedWorkers = Math.min(constrainedWorkers, maxWorkers);
    }
    
    return Math.max(1, constrainedWorkers);
  }

  /**
   * Balanced scheduling strategy (default)
   */
  scheduleBalanced(testFiles, workerCount) {
    // Sort tests by execution time (longest first) for better bin packing
    const sortedTests = [...testFiles].sort((a, b) => b.actualTime - a.actualTime);
    
    // Initialize workers
    const workers = Array.from({ length: workerCount }, (_, index) => ({
      id: index + 1,
      tests: [],
      totalTime: 0,
      totalMemory: 0,
      groups: new Set()
    }));
    
    // Distribute tests using bin-packing algorithm
    for (const test of sortedTests) {
      const bestWorker = this.findBestWorkerForTest(workers, test);
      this.assignTestToWorker(bestWorker, test);
    }
    
    return workers;
  }

  /**
   * Priority-based scheduling
   */
  schedulePriority(testFiles, workerCount) {
    // Sort by priority first, then by execution time
    const sortedTests = [...testFiles].sort((a, b) => {
      if (a.priority !== b.priority) {
        return b.priority - a.priority; // Higher priority first
      }
      return b.actualTime - a.actualTime; // Longer tests first within same priority
    });
    
    return this.scheduleBalanced(sortedTests, workerCount);
  }

  /**
   * Fastest completion scheduling
   */
  scheduleFastest(testFiles, workerCount) {
    // Sort by execution time only (longest first)
    const sortedTests = [...testFiles].sort((a, b) => b.actualTime - a.actualTime);
    
    return this.scheduleBalanced(sortedTests, workerCount);
  }

  /**
   * Find the best worker for a specific test
   */
  findBestWorkerForTest(workers, test) {
    // Consider group constraints
    const groupConstraint = test.groupConfig.maxWorkers;
    const workersInGroup = workers.filter(w => w.groups.has(test.group)).length;
    
    // Filter workers that can accept this test
    let candidateWorkers = workers.filter(worker => {
      // Check group constraints
      if (workersInGroup >= groupConstraint && !worker.groups.has(test.group)) {
        return false;
      }
      
      // Check memory constraints (simple check)
      const projectedMemory = worker.totalMemory + test.memoryRequirement;
      if (projectedMemory > 800 * 1024 * 1024) { // 800MB limit per worker
        return false;
      }
      
      return true;
    });
    
    // If no candidates due to constraints, use all workers
    if (candidateWorkers.length === 0) {
      candidateWorkers = workers;
    }
    
    // Find worker with least total time (load balancing)
    return candidateWorkers.reduce((best, current) => 
      current.totalTime < best.totalTime ? current : best
    );
  }

  /**
   * Assign test to worker
   */
  assignTestToWorker(worker, test) {
    worker.tests.push(test);
    worker.totalTime += test.actualTime;
    worker.totalMemory += test.memoryRequirement;
    worker.groups.add(test.group);
  }

  /**
   * Optimize the schedule
   */
  optimizeSchedule(schedule) {
    // Remove empty workers
    const optimizedSchedule = schedule.filter(worker => worker.tests.length > 0);
    
    // Try to balance load better
    this.balanceWorkload(optimizedSchedule);
    
    return optimizedSchedule;
  }

  /**
   * Balance workload across workers
   */
  balanceWorkload(workers) {
    const maxIterations = 10;
    let iteration = 0;
    
    while (iteration < maxIterations) {
      const avgTime = workers.reduce((sum, w) => sum + w.totalTime, 0) / workers.length;
      const threshold = avgTime * 0.2; // 20% threshold
      
      let improved = false;
      
      // Find overloaded and underloaded workers
      const overloaded = workers.filter(w => w.totalTime > avgTime + threshold);
      const underloaded = workers.filter(w => w.totalTime < avgTime - threshold);
      
      if (overloaded.length === 0 || underloaded.length === 0) {
        break; // Already balanced
      }
      
      // Try to move tests from overloaded to underloaded workers
      for (const overWorker of overloaded) {
        for (const underWorker of underloaded) {
          const moved = this.moveTestBetweenWorkers(overWorker, underWorker, threshold);
          if (moved) {
            improved = true;
            break;
          }
        }
        if (improved) break;
      }
      
      if (!improved) break;
      iteration++;
    }
  }

  /**
   * Move a test between workers if beneficial
   */
  moveTestBetweenWorkers(fromWorker, toWorker, threshold) {
    // Find a test that would improve balance
    for (let i = 0; i < fromWorker.tests.length; i++) {
      const test = fromWorker.tests[i];
      
      // Check if moving this test would improve balance
      const newFromTime = fromWorker.totalTime - test.actualTime;
      const newToTime = toWorker.totalTime + test.actualTime;
      
      if (newFromTime > newToTime && (fromWorker.totalTime - newFromTime) > threshold / 2) {
        // Move the test
        fromWorker.tests.splice(i, 1);
        fromWorker.totalTime -= test.actualTime;
        fromWorker.totalMemory -= test.memoryRequirement;
        
        toWorker.tests.push(test);
        toWorker.totalTime += test.actualTime;
        toWorker.totalMemory += test.memoryRequirement;
        toWorker.groups.add(test.group);
        
        return true;
      }
    }
    
    return false;
  }

  /**
   * Log schedule summary
   */
  logScheduleSummary(schedule) {
    console.log('\n📊 Test Schedule Summary:');
    console.log('─'.repeat(80));
    
    const totalTests = schedule.reduce((sum, w) => sum + w.tests.length, 0);
    const totalTime = Math.max(...schedule.map(w => w.totalTime));
    const avgTime = schedule.reduce((sum, w) => sum + w.totalTime, 0) / schedule.length;
    
    console.log(`Total Tests: ${totalTests}`);
    console.log(`Workers: ${schedule.length}`);
    console.log(`Estimated Total Time: ${totalTime.toFixed(1)}s`);
    console.log(`Average Worker Time: ${avgTime.toFixed(1)}s`);
    console.log(`Load Balance: ${((1 - (totalTime - avgTime) / totalTime) * 100).toFixed(1)}%`);
    
    console.log('\nWorker Distribution:');
    schedule.forEach(worker => {
      const groups = Array.from(worker.groups).join(', ');
      console.log(`  Worker ${worker.id}: ${worker.tests.length} tests, ${worker.totalTime.toFixed(1)}s, Groups: ${groups}`);
    });
    
    console.log('─'.repeat(80));
  }

  /**
   * Get scheduling statistics
   */
  getScheduleStats(schedule) {
    const totalTests = schedule.reduce((sum, w) => sum + w.tests.length, 0);
    const totalTime = Math.max(...schedule.map(w => w.totalTime));
    const avgTime = schedule.reduce((sum, w) => sum + w.totalTime, 0) / schedule.length;
    const loadBalance = (1 - (totalTime - avgTime) / totalTime) * 100;
    
    return {
      totalTests,
      workerCount: schedule.length,
      estimatedTotalTime: totalTime,
      averageWorkerTime: avgTime,
      loadBalance,
      workers: schedule.map(w => ({
        id: w.id,
        testCount: w.tests.length,
        totalTime: w.totalTime,
        groups: Array.from(w.groups),
        tests: w.tests.map(t => t.relativePath)
      }))
    };
  }

  /**
   * Update test history with actual execution times
   */
  updateTestHistory(testResults) {
    for (const result of testResults) {
      const { testFile, executionTime, success } = result;
      
      if (!this.testHistory[testFile]) {
        this.testHistory[testFile] = {
          executions: [],
          averageTime: executionTime,
          successRate: success ? 1 : 0
        };
      } else {
        const history = this.testHistory[testFile];
        history.executions.push({ time: executionTime, success, timestamp: new Date() });
        
        // Keep only last 10 executions
        if (history.executions.length > 10) {
          history.executions = history.executions.slice(-10);
        }
        
        // Update averages
        const successfulRuns = history.executions.filter(e => e.success);
        if (successfulRuns.length > 0) {
          history.averageTime = successfulRuns.reduce((sum, e) => sum + e.time, 0) / successfulRuns.length;
        }
        history.successRate = successfulRuns.length / history.executions.length;
      }
    }
  }
}

module.exports = TestScheduler;
