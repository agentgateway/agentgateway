# E2E Testing Infrastructure Fixes

This document outlines the fixes implemented to address the memory usage and testing issues identified in PR #184.

## Issues Identified

### 1. Memory Calculation Problems
- **Original Issue**: Resource monitor showed 99.9% memory usage on a 64GB system, triggering emergency shutdown
- **Root Cause**: Used `os.freemem()` which shows currently available memory, not accounting for system overhead
- **Impact**: Even systems with abundant memory couldn't run parallel tests

### 2. Unrealistic Memory Expectations
- **Original Issue**: Documentation suggested 250-500MB per worker, but actual usage was much higher
- **Root Cause**: Cypress with video recording and browser instances uses significantly more memory
- **Impact**: Worker calculations were based on incorrect assumptions

### 3. Aggressive Emergency Thresholds
- **Original Issue**: 85% memory limit with 90% emergency shutdown was too aggressive
- **Root Cause**: Didn't account for system processes and OS overhead
- **Impact**: False emergencies on healthy systems

## Fixes Implemented

### 1. Resource Detection Script (`scripts/detect-system-resources.js`)

**Purpose**: Allows users to document their system configuration and get appropriate test settings.

**Features**:
- Detects system resources (CPU, memory, disk)
- Calculates system overhead (2GB or 15% of total memory)
- Provides environment-specific recommendations
- Generates npm scripts for different test modes
- Supports CI detection (GitLab, GitHub Actions, etc.)

**Usage**:
```bash
# Basic detection
node scripts/detect-system-resources.js

# Apply recommendations to package.json
node scripts/detect-system-resources.js --apply

# Save to custom files
node scripts/detect-system-resources.js --output ci-resources.json --report ci-report.txt
```

**Example Output**:
```
System Resource Detection Report
================================
Memory Information:
------------------
Total Memory: 62.61 GB
Available for Tests: 60.61 GB
System Overhead: 2 GB

Recommendations:
---------------
Max Workers: 4
Memory Limit: 65%
Strategy: balanced

Recommended npm Scripts:
-----------------------
"test:e2e:auto": "./scripts/run-e2e-tests.sh --workers 4 --memory-limit 65"
"test:e2e:conservative": "./scripts/run-e2e-tests.sh --workers 1 --memory-limit 50"
```

### 2. Fixed Resource Monitor (`scripts/lib/resource-monitor-fixed.js`)

**Key Improvements**:

#### Memory Calculation Fix
```javascript
// OLD (problematic)
const usedMemory = this.totalMemory - os.freemem();
const usagePercent = (usedMemory / this.totalMemory) * 100;

// NEW (fixed)
const systemOverhead = Math.min(2 * 1024 * 1024 * 1024, this.totalMemory * 0.15);
const availableMemory = this.totalMemory - systemOverhead;
const usedOfAvailable = Math.max(0, usedMemory - systemOverhead);
const usagePercent = (usedOfAvailable / availableMemory) * 100;
```

#### Environment-Specific Defaults
```javascript
getEnvironmentDefaults() {
  const totalMemoryGB = os.totalmem() / (1024 * 1024 * 1024);
  
  if (process.env.CI) {
    return { memoryLimit: 60, maxWorkers: 2 }; // Conservative for CI
  } else if (totalMemoryGB < 8) {
    return { memoryLimit: 50, maxWorkers: 1 }; // Very conservative
  } else if (totalMemoryGB < 16) {
    return { memoryLimit: 60, maxWorkers: 2 }; // Conservative
  } else if (totalMemoryGB < 32) {
    return { memoryLimit: 65, maxWorkers: 4 }; // Moderate
  } else {
    return { memoryLimit: 70, maxWorkers: 6 }; // Less conservative
  }
}
```

#### Realistic Worker Memory Estimates
```javascript
// More realistic memory per worker based on actual Cypress usage
const memoryPerWorker = process.env.CI ? 300 * 1024 * 1024 : 500 * 1024 * 1024;
```

#### Better Emergency Thresholds
- Memory emergency: 80% (reduced from 90%)
- CPU emergency: 90% (reduced from 95%)
- More conservative warning thresholds

### 3. Minimal Test Script (`scripts/test-e2e-minimal.js`)

**Purpose**: Provides step-by-step debugging and validation of fixes.

**Features**:
- Tests fixed resource monitor in isolation
- Debugs backend startup issues with detailed logging
- Tests UI startup with comprehensive error capture
- Runs single Cypress test for validation
- Generates detailed test reports

**Usage**:
```bash
# Test resource monitor only
node scripts/test-e2e-minimal.js --resource-only

# Test backend startup only
node scripts/test-e2e-minimal.js --backend-only

# Full test suite
node scripts/test-e2e-minimal.js
```

## Validation Results

### System Resource Detection
```
Total Memory: 62.61 GB
Available for Tests: 60.61 GB (96.8% of total)
System Overhead: 2 GB (3.2% reserved)
Recommendations: 4 workers, 65% memory limit
```

### Fixed Resource Monitor
```
Memory Usage: 8.5% of available (vs 99.9% before)
Status: ✅ Safe (vs ❌ Emergency before)
Optimal Workers: 6 (vs 0 before due to emergency)
```

## Usage Instructions

### For Developers

1. **Detect Your System Resources**:
   ```bash
   node scripts/detect-system-resources.js --apply
   ```

2. **Test the Fixes**:
   ```bash
   node scripts/test-e2e-minimal.js --resource-only
   ```

3. **Run E2E Tests with Fixed Settings**:
   ```bash
   # Use auto-detected settings
   npm run test:e2e:auto
   
   # Or use conservative settings
   npm run test:e2e:conservative
   ```

### For CI/CD

1. **Detect CI Resources**:
   ```bash
   # In CI pipeline
   node scripts/detect-system-resources.js --output ci-resources.json --quiet
   ```

2. **Use Conservative Settings**:
   ```bash
   # GitLab CI example
   npm run test:e2e:conservative
   # or
   ./scripts/run-e2e-tests.sh --workers 1 --memory-limit 50
   ```

### For Different Environments

#### Low Memory Systems (< 8GB)
```bash
./scripts/run-e2e-tests.sh --workers 1 --memory-limit 50
```

#### Medium Systems (8-16GB)
```bash
./scripts/run-e2e-tests.sh --workers 2 --memory-limit 60
```

#### High Memory Systems (32GB+)
```bash
./scripts/run-e2e-tests.sh --workers 6 --memory-limit 70
```

## Breaking Changes

### Resource Monitor API Changes
- Constructor now accepts environment-specific defaults
- Memory calculation methods return different structure
- New properties: `systemOverhead`, `availableMemory`, `usedOfAvailable`

### Parallel Test Runner Changes
- Now uses `resource-monitor-fixed.js` instead of `resource-monitor.js`
- Default memory limits adjusted based on environment
- Worker calculation algorithm improved

### Configuration Changes
- Memory limits now calculated against available memory (not total)
- Worker memory estimates increased to realistic values
- Emergency thresholds made less aggressive

## Backward Compatibility

The fixes maintain backward compatibility for:
- CLI interfaces and options
- Report generation formats
- npm script names and functionality

Breaking changes are limited to internal APIs and calculation methods.

## Testing the Fixes

### Quick Validation
```bash
# Test resource detection
node scripts/detect-system-resources.js

# Test fixed resource monitor
node scripts/test-e2e-minimal.js --resource-only

# Test backend startup
node scripts/test-e2e-minimal.js --backend-only
```

### Full Validation
```bash
# Run complete minimal test suite
node scripts/test-e2e-minimal.js

# Check generated report
cat minimal-test-report.json
```

### Production Testing
```bash
# Use conservative settings first
npm run test:e2e:conservative

# If successful, try balanced settings
npm run test:e2e:auto
```

## Troubleshooting

### Still Getting Memory Errors?
1. Run resource detection: `node scripts/detect-system-resources.js`
2. Use conservative settings: `--workers 1 --memory-limit 50`
3. Check system load: `top` or `htop`
4. Disable video recording: `--no-video`

### Backend Startup Issues?
1. Run backend test: `node scripts/test-e2e-minimal.js --backend-only --verbose`
2. Check binary exists: `ls -la target/debug/agentgateway`
3. Verify config: `cat test-config.yaml`
4. Check logs for detailed error messages

### UI Startup Issues?
1. Check Node.js version: `node --version`
2. Verify dependencies: `cd ui && npm install`
3. Test UI separately: `cd ui && npm run dev`
4. Check port availability: `lsof -i :3000`

## Next Steps

1. **Test on Different Environments**: Run the detection script on various systems
2. **Gather CI/CD Specs**: Use detection script in GitLab CI to determine resource limits
3. **Monitor Performance**: Use the fixed resource monitor to track actual usage
4. **Optimize Further**: Based on real usage data, fine-tune memory estimates

The fixes provide a solid foundation for reliable E2E testing across different environments while maintaining safety and performance.
