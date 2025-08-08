const os = require('os');
const fs = require('fs').promises;
const { execSync } = require('child_process');

/**
 * Fixed ResourceMonitor - Addresses memory calculation and threshold issues
 * 
 * Key Fixes:
 * - Proper memory calculation accounting for system overhead
 * - More realistic per-worker memory estimates
 * - Environment-specific defaults
 * - Better emergency thresholds
 */
class ResourceMonitor {
  constructor(options = {}) {
    // Apply environment-specific defaults
    const envDefaults = this.getEnvironmentDefaults();
    
    this.memoryLimitPercent = options.memoryLimit || envDefaults.memoryLimit;
    this.diskSpaceBuffer = options.diskSpaceBuffer || 200 * 1024 * 1024; // 200MB (increased)
    this.cpuThreshold = options.cpuThreshold || 80; // Reduced from 90
    
    this.totalMemory = os.totalmem();
    
    // Calculate system overhead (reserve memory for OS and other processes)
    this.systemOverhead = Math.min(2 * 1024 * 1024 * 1024, this.totalMemory * 0.15); // 2GB or 15%
    this.availableMemory = this.totalMemory - this.systemOverhead;
    this.memoryLimit = this.availableMemory * (this.memoryLimitPercent / 100);
    
    this.isMonitoring = false;
    this.monitoringInterval = null;
    this.resourceHistory = [];
    this.maxHistorySize = 100;
    
    this.listeners = {
      memoryWarning: [],
      diskWarning: [],
      cpuWarning: [],
      emergency: []
    };

    console.log(`🔧 ResourceMonitor initialized with fixed calculations:`);
    console.log(`   Total Memory: ${this.formatBytes(this.totalMemory)}`);
    console.log(`   System Overhead: ${this.formatBytes(this.systemOverhead)}`);
    console.log(`   Available for Tests: ${this.formatBytes(this.availableMemory)}`);
    console.log(`   Memory Limit: ${this.formatBytes(this.memoryLimit)} (${this.memoryLimitPercent}%)`);
  }

  /**
   * Get environment-specific defaults
   */
  getEnvironmentDefaults() {
    const totalMemoryGB = os.totalmem() / (1024 * 1024 * 1024);
    
    if (process.env.CI) {
      return { 
        memoryLimit: 60, // Very conservative for CI
        maxWorkers: 2 
      };
    } else if (totalMemoryGB < 8) {
      return { 
        memoryLimit: 50, // Very conservative for low-memory systems
        maxWorkers: 1 
      };
    } else if (totalMemoryGB < 16) {
      return { 
        memoryLimit: 60, // Conservative for medium systems
        maxWorkers: 2 
      };
    } else if (totalMemoryGB < 32) {
      return { 
        memoryLimit: 65, // Moderate for good systems
        maxWorkers: 4 
      };
    } else {
      return { 
        memoryLimit: 70, // Less conservative for high-memory systems
        maxWorkers: 6 
      };
    }
  }

  /**
   * Start continuous resource monitoring
   */
  startMonitoring(intervalMs = 5000) {
    if (this.isMonitoring) {
      return;
    }

    this.isMonitoring = true;
    this.monitoringInterval = setInterval(() => {
      this.checkResources();
    }, intervalMs);

    console.log(`🔍 Resource monitoring started (interval: ${intervalMs}ms)`);
    console.log(`📊 Memory limit: ${this.formatBytes(this.memoryLimit)} (${this.memoryLimitPercent}% of available)`);
    console.log(`💾 Disk buffer: ${this.formatBytes(this.diskSpaceBuffer)}`);
  }

  /**
   * Stop resource monitoring
   */
  stopMonitoring() {
    if (this.monitoringInterval) {
      clearInterval(this.monitoringInterval);
      this.monitoringInterval = null;
    }
    this.isMonitoring = false;
    console.log('🛑 Resource monitoring stopped');
  }

  /**
   * Check current memory usage (FIXED)
   */
  checkMemoryUsage() {
    const freeMemory = os.freemem();
    const usedMemory = this.totalMemory - freeMemory;
    
    // Calculate usage against available memory (not total)
    const usedOfAvailable = Math.max(0, usedMemory - this.systemOverhead);
    const usagePercent = (usedOfAvailable / this.availableMemory) * 100;
    
    // More realistic safety check
    const safe = usedOfAvailable < this.memoryLimit;
    
    return {
      total: this.totalMemory,
      used: usedMemory,
      free: freeMemory,
      available: this.availableMemory,
      usedOfAvailable: usedOfAvailable,
      percentage: Math.max(0, usagePercent), // Ensure non-negative
      safe: safe,
      limit: this.memoryLimit,
      limitPercent: this.memoryLimitPercent,
      systemOverhead: this.systemOverhead
    };
  }

  /**
   * Check current disk space
   */
  async checkDiskSpace(path = process.cwd()) {
    try {
      // Use statvfs for Unix-like systems
      if (process.platform !== 'win32') {
        const diskUsage = await this.getDiskUsageCrossPlatform(path);
        return {
          ...diskUsage,
          safe: diskUsage.available > this.diskSpaceBuffer
        };
      } else {
        // Windows implementation
        const diskUsage = await this.getDiskUsageWindows(path);
        return {
          ...diskUsage,
          safe: diskUsage.available > this.diskSpaceBuffer
        };
      }
    } catch (error) {
      console.warn('⚠️ Could not check disk space:', error.message);
      return {
        total: 0,
        used: 0,
        available: this.diskSpaceBuffer + 1, // Assume safe if we can't check
        safe: true,
        error: error.message
      };
    }
  }

  /**
   * Cross-platform disk usage check
   */
  async getDiskUsageCrossPlatform(path) {
    try {
      if (process.platform === 'win32') {
        return this.getDiskUsageWindows(path);
      } else {
        return this.getDiskUsageUnix(path);
      }
    } catch (error) {
      throw new Error(`Failed to get disk usage: ${error.message}`);
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
      
      const total = parseInt(data[1]) * 1024; // Convert from KB to bytes
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
        const output = execSync(`dir /-c "${drive}"`, { encoding: 'utf8' });
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
   * Check CPU usage
   */
  checkCPUUsage() {
    const cpus = os.cpus();
    const loadAvg = os.loadavg();
    
    // Calculate CPU usage percentage
    const cpuCount = cpus.length;
    const currentLoad = loadAvg[0]; // 1-minute average
    const usagePercent = (currentLoad / cpuCount) * 100;
    
    return {
      cores: cpuCount,
      loadAverage: loadAvg,
      currentLoad: currentLoad,
      percentage: Math.min(usagePercent, 100), // Cap at 100%
      safe: usagePercent < this.cpuThreshold
    };
  }

  /**
   * Calculate optimal worker count based on resources (IMPROVED)
   */
  calculateOptimalWorkers() {
    const memory = this.checkMemoryUsage();
    const cpu = this.checkCPUUsage();
    const envDefaults = this.getEnvironmentDefaults();
    
    // More realistic memory per worker estimates based on actual Cypress usage
    const memoryPerWorker = process.env.CI ? 300 * 1024 * 1024 : 500 * 1024 * 1024; // 300MB CI, 500MB local
    
    // Base calculation on CPU cores (leave at least 1 core for system)
    const cpuWorkers = Math.max(1, cpu.cores - 1);
    
    // Calculate based on available memory
    const availableForWorkers = Math.max(0, memory.available - memory.usedOfAvailable);
    const memoryWorkers = Math.floor(availableForWorkers / memoryPerWorker);
    
    // Take the minimum to ensure safety, but respect environment defaults
    let optimalWorkers = Math.min(cpuWorkers, memoryWorkers, envDefaults.maxWorkers);
    
    // Additional safety checks
    if (memory.percentage > 70) {
      optimalWorkers = Math.max(1, Math.floor(optimalWorkers / 2));
    }
    
    if (cpu.percentage > 60) {
      optimalWorkers = Math.max(1, Math.floor(optimalWorkers / 2));
    }
    
    // Ensure at least 1 worker
    return Math.max(1, optimalWorkers);
  }

  /**
   * Comprehensive resource check
   */
  async checkResources() {
    const timestamp = new Date();
    const memory = this.checkMemoryUsage();
    const cpu = this.checkCPUUsage();
    const disk = await this.checkDiskSpace();
    
    const resourceSnapshot = {
      timestamp,
      memory,
      cpu,
      disk,
      safe: memory.safe && cpu.safe && disk.safe
    };
    
    // Add to history
    this.resourceHistory.push(resourceSnapshot);
    if (this.resourceHistory.length > this.maxHistorySize) {
      this.resourceHistory.shift();
    }
    
    // Check for warnings
    this.checkForWarnings(resourceSnapshot);
    
    return resourceSnapshot;
  }

  /**
   * Check for resource warnings and emit events (IMPROVED)
   */
  checkForWarnings(snapshot) {
    // Memory warnings - more conservative thresholds
    if (!snapshot.memory.safe) {
      this.emit('memoryWarning', snapshot.memory);
      if (snapshot.memory.percentage > 80) { // Reduced from 90
        this.emit('emergency', { type: 'memory', data: snapshot.memory });
      }
    }
    
    // CPU warnings
    if (!snapshot.cpu.safe) {
      this.emit('cpuWarning', snapshot.cpu);
      if (snapshot.cpu.percentage > 90) { // Reduced from 95
        this.emit('emergency', { type: 'cpu', data: snapshot.cpu });
      }
    }
    
    // Disk warnings
    if (!snapshot.disk.safe) {
      this.emit('diskWarning', snapshot.disk);
      if (snapshot.disk.available < this.diskSpaceBuffer / 2) {
        this.emit('emergency', { type: 'disk', data: snapshot.disk });
      }
    }
  }

  /**
   * Event emitter functionality
   */
  on(event, callback) {
    if (this.listeners[event]) {
      this.listeners[event].push(callback);
    }
  }

  emit(event, data) {
    if (this.listeners[event]) {
      this.listeners[event].forEach(callback => {
        try {
          callback(data);
        } catch (error) {
          console.error(`Error in ${event} listener:`, error);
        }
      });
    }
  }

  /**
   * Get resource usage summary
   */
  getResourceSummary() {
    if (this.resourceHistory.length === 0) {
      return null;
    }
    
    const latest = this.resourceHistory[this.resourceHistory.length - 1];
    const avgMemory = this.resourceHistory.reduce((sum, r) => sum + r.memory.percentage, 0) / this.resourceHistory.length;
    const avgCPU = this.resourceHistory.reduce((sum, r) => sum + r.cpu.percentage, 0) / this.resourceHistory.length;
    
    return {
      current: latest,
      averages: {
        memory: avgMemory,
        cpu: avgCPU
      },
      optimalWorkers: this.calculateOptimalWorkers(),
      safe: latest.safe,
      configuration: {
        memoryLimit: this.memoryLimitPercent,
        systemOverhead: this.formatBytes(this.systemOverhead),
        availableMemory: this.formatBytes(this.availableMemory)
      }
    };
  }

  /**
   * Format bytes for human-readable output
   */
  formatBytes(bytes) {
    if (bytes === 0) return '0 Bytes';
    
    const k = 1024;
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  }

  /**
   * Get detailed system information
   */
  getSystemInfo() {
    return {
      platform: os.platform(),
      arch: os.arch(),
      cpus: os.cpus().length,
      totalMemory: this.formatBytes(this.totalMemory),
      availableMemory: this.formatBytes(this.availableMemory),
      systemOverhead: this.formatBytes(this.systemOverhead),
      memoryLimit: this.formatBytes(this.memoryLimit),
      diskSpaceBuffer: this.formatBytes(this.diskSpaceBuffer),
      nodeVersion: process.version,
      uptime: os.uptime(),
      environment: process.env.CI ? 'CI' : 'Development'
    };
  }
}

module.exports = ResourceMonitor;
