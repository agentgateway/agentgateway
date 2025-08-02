const os = require('os');
const fs = require('fs').promises;
const { execSync } = require('child_process');

/**
 * ResourceMonitor - Monitors system resources and enforces safety limits
 * 
 * Key Features:
 * - Memory usage monitoring with 85% limit
 * - Disk space monitoring with 100MB buffer
 * - CPU utilization tracking
 * - Real-time resource alerting
 */
class ResourceMonitor {
  constructor(options = {}) {
    this.memoryLimitPercent = options.memoryLimit || 85;
    this.diskSpaceBuffer = options.diskSpaceBuffer || 100 * 1024 * 1024; // 100MB
    this.cpuThreshold = options.cpuThreshold || 90;
    
    this.totalMemory = os.totalmem();
    this.memoryLimit = this.totalMemory * (this.memoryLimitPercent / 100);
    
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
    console.log(`📊 Memory limit: ${this.formatBytes(this.memoryLimit)} (${this.memoryLimitPercent}%)`);
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
   * Check current memory usage
   */
  checkMemoryUsage() {
    const freeMemory = os.freemem();
    const usedMemory = this.totalMemory - freeMemory;
    const usagePercent = (usedMemory / this.totalMemory) * 100;
    
    return {
      total: this.totalMemory,
      used: usedMemory,
      free: freeMemory,
      percentage: usagePercent,
      safe: usedMemory < this.memoryLimit,
      limit: this.memoryLimit,
      limitPercent: this.memoryLimitPercent
    };
  }

  /**
   * Check current disk space
   */
  async checkDiskSpace(path = process.cwd()) {
    try {
      // Use statvfs for Unix-like systems
      if (process.platform !== 'win32') {
        const stats = await fs.stat(path);
        // For simplicity, we'll use a cross-platform approach
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
      const output = execSync(`dir /-c "${drive}"`, { encoding: 'utf8' });
      
      // Parse Windows dir output (this is a simplified version)
      const lines = output.split('\n');
      const lastLine = lines[lines.length - 2];
      const match = lastLine.match(/(\d+)\s+bytes free/);
      
      if (match) {
        const available = parseInt(match[1]);
        // For Windows, we'll estimate total and used
        return {
          total: available * 2, // Rough estimate
          used: available,
          available: available
        };
      }
      
      throw new Error('Could not parse Windows disk usage');
    } catch (error) {
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
   * Calculate optimal worker count based on resources
   */
  calculateOptimalWorkers() {
    const memory = this.checkMemoryUsage();
    const cpu = this.checkCPUUsage();
    
    // Base calculation on CPU cores
    const cpuWorkers = Math.max(2, cpu.cores - 1);
    
    // Calculate based on available memory (400MB per worker)
    const memoryPerWorker = 400 * 1024 * 1024; // 400MB
    const memoryWorkers = Math.floor(memory.free / memoryPerWorker);
    
    // Take the minimum to ensure safety
    const optimalWorkers = Math.min(cpuWorkers, memoryWorkers, 8); // Cap at 8
    
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
   * Check for resource warnings and emit events
   */
  checkForWarnings(snapshot) {
    // Memory warnings
    if (!snapshot.memory.safe) {
      this.emit('memoryWarning', snapshot.memory);
      if (snapshot.memory.percentage > 90) {
        this.emit('emergency', { type: 'memory', data: snapshot.memory });
      }
    }
    
    // CPU warnings
    if (!snapshot.cpu.safe) {
      this.emit('cpuWarning', snapshot.cpu);
      if (snapshot.cpu.percentage > 95) {
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
      safe: latest.safe
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
      memoryLimit: this.formatBytes(this.memoryLimit),
      diskSpaceBuffer: this.formatBytes(this.diskSpaceBuffer),
      nodeVersion: process.version,
      uptime: os.uptime()
    };
  }
}

module.exports = ResourceMonitor;
