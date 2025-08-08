# AgentGateway E2E Test Scripts

This directory contains sophisticated testing infrastructure for AgentGateway's end-to-end tests, featuring intelligent parallel execution, resource monitoring, and comprehensive reporting.

## Overview

The testing system consists of several components working together to provide efficient, reliable, and scalable E2E testing:

- **Parallel Test Runner**: Orchestrates parallel test execution with resource management
- **Test Scheduler**: Intelligently distributes tests across workers using bin-packing algorithms
- **Worker Manager**: Manages Cypress worker processes with lifecycle control
- **Resource Monitor**: Monitors system resources and prevents resource exhaustion
- **Shell Script Runner**: Provides simple interface for common testing scenarios

## Quick Start

### Basic Usage

```bash
# Run all tests in parallel (recommended)
npm run test:e2e:parallel

# Run smoke tests only (fast feedback)
npm run e2e:smoke

# Run tests with visible browser (debugging)
npm run test:e2e:parallel:ci:headed

# Run tests in specific browser
npm run test:e2e:parallel:chrome
```

### Shell Script Interface

```bash
# Simple parallel execution
./scripts/run-e2e-tests.sh

# Run with specific options
./scripts/run-e2e-tests.sh --mode parallel --workers 6 --headed

# Interactive mode (opens Cypress UI)
./scripts/run-e2e-tests.sh --mode interactive
```

## Test Organization

### Test Groups

Tests are organized into groups with different characteristics:

#### 🚀 Smoke Tests (`smoke/`)
- **Purpose**: Critical path validation for rapid feedback
- **Execution Time**: ~1 second per test
- **Priority**: Critical
- **Memory**: 250MB per worker
- **Max Workers**: 4

**Files:**
- `critical-path.cy.ts` - Core application functionality
- `api-health.cy.ts` - Backend connectivity and health checks

#### ⚡ Fast Tests (`foundation/`, `navigation/`)
- **Purpose**: Basic functionality and navigation
- **Execution Time**: ~2 seconds per test
- **Priority**: High
- **Memory**: 300MB per worker
- **Max Workers**: 6

#### 🔧 Medium Tests (`setup-wizard/`, `configuration/`)
- **Purpose**: Configuration and setup workflows
- **Execution Time**: ~8 seconds per test
- **Priority**: Medium
- **Memory**: 400MB per worker
- **Max Workers**: 3

#### 🐌 Slow Tests (`integration/`, `playground/`, `error-handling/`)
- **Purpose**: Complex integration scenarios
- **Execution Time**: ~20 seconds per test
- **Priority**: Low
- **Memory**: 500MB per worker
- **Max Workers**: 2

## Parallel Test Runner

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Parallel Test Runner                     │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ Resource Monitor│  │  Test Scheduler │  │Worker Manager│ │
│  │                 │  │                 │  │              │ │
│  │ • CPU Monitoring│  │ • Test Discovery│  │ • Process    │ │
│  │ • Memory Limits │  │ • Load Balancing│  │   Management │ │
│  │ • Disk Space    │  │ • Bin Packing   │  │ • Result     │ │
│  │ • Emergency     │  │ • Priority      │  │   Collection │ │
│  │   Shutdown      │  │   Scheduling    │  │ • Cleanup    │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Features

#### 🔍 Intelligent Test Discovery
- Automatic test file discovery using glob patterns
- Test analysis for execution time estimation
- Dependency detection and grouping

#### ⚖️ Load Balancing
- Bin-packing algorithm for optimal test distribution
- Historical execution time tracking
- Dynamic worker allocation based on system resources

#### 🛡️ Resource Safety
- Real-time CPU, memory, and disk monitoring
- Automatic worker scaling based on available resources
- Emergency shutdown on resource exhaustion
- Configurable resource limits and thresholds

#### 📊 Comprehensive Reporting
- Parallel execution efficiency metrics
- Worker utilization statistics
- Test result aggregation
- JSON and text report generation

### Command Line Options

```bash
node scripts/parallel-test-runner.js [options]

Options:
  --workers <number>        Maximum number of workers (auto-detect if not set)
  --strategy <type>         Scheduling strategy: balanced, fastest, priority (default: balanced)
  --browser <name>          Browser: electron, chrome, firefox, edge (default: electron)
  --headed                  Run in headed mode (visible browser)
  --no-headless            Run in headed mode (alternative syntax)
  --no-video               Disable video recording
  --no-quiet               Enable verbose output
  --debug                  Enable debug mode with detailed logging
  --ci                     CI environment mode (resource conservative)
  --dev                    Development mode (resource conservative)
  --smoke                  Run only smoke tests
  --memory-limit <percent> Memory usage limit percentage (default: 85)
  --disk-buffer <mb>       Disk space buffer in MB (default: 100)
```

### Scheduling Strategies

#### Balanced (Default)
- Distributes tests evenly across workers
- Uses bin-packing algorithm for optimal load distribution
- Considers test execution time and resource requirements

#### Fastest
- Prioritizes fastest overall completion time
- Runs longest tests first for better parallelization
- Ignores test priorities

#### Priority
- Runs high-priority tests first
- Ensures critical tests complete early
- Falls back to execution time for same-priority tests

### Resource Monitoring

#### Memory Management
- Monitors system memory usage in real-time
- Prevents memory exhaustion by limiting worker count
- Configurable memory limits (default: 85% of system memory)

#### CPU Monitoring
- Tracks CPU usage across all cores
- Warns when CPU usage exceeds thresholds
- Factors CPU availability into worker calculations

#### Disk Space Management
- Monitors available disk space for test artifacts
- Reserves buffer space for video recordings and reports
- Prevents disk full scenarios

#### Emergency Procedures
- Automatic worker termination on resource emergencies
- Partial result saving during emergency shutdown
- Graceful cleanup of system resources

## Test Scheduler

### Bin-Packing Algorithm

The scheduler uses a sophisticated bin-packing algorithm to distribute tests:

1. **Sort Tests**: Orders tests by execution time (longest first)
2. **Worker Selection**: Finds the best worker for each test considering:
   - Current worker load
   - Group constraints (max workers per test group)
   - Memory requirements
   - Resource availability
3. **Load Balancing**: Iteratively moves tests between workers to optimize balance
4. **Validation**: Ensures schedule meets all constraints

### Group Constraints

Each test group has constraints to prevent resource conflicts:

- **Max Workers**: Limits concurrent workers per group
- **Memory Per Worker**: Expected memory usage
- **Estimated Time**: Average execution time per test
- **Priority**: Scheduling priority (critical > high > medium > low)

## Worker Manager

### Process Management

- **Isolation**: Each worker runs in a separate Node.js process
- **Communication**: IPC-based communication for result collection
- **Lifecycle**: Complete process lifecycle management
- **Cleanup**: Automatic cleanup of worker processes and resources

### Error Handling

- **Process Crashes**: Automatic detection and reporting
- **Timeout Handling**: Configurable timeouts for worker processes
- **Resource Leaks**: Prevention of zombie processes
- **Graceful Shutdown**: Coordinated shutdown of all workers

## Available NPM Scripts

### Parallel Execution
```bash
npm run test:e2e:parallel              # Default parallel execution
npm run test:e2e:parallel:dev          # Development mode (conservative resources)
npm run test:e2e:parallel:ci           # CI mode (resource optimized)
npm run test:e2e:parallel:ci:headed    # CI mode with visible browser
npm run test:e2e:parallel:debug        # Debug mode (single worker, verbose)
npm run test:e2e:parallel:chrome       # Run with Chrome browser
npm run test:e2e:parallel:firefox      # Run with Firefox browser
npm run test:e2e:parallel:smoke        # Run only smoke tests
```

### Sequential Execution
```bash
npm run test:e2e:sequential            # Sequential execution (headless)
npm run test:e2e:sequential:headed     # Sequential execution (headed)
```

### Cypress Direct
```bash
npm run cypress:run                    # Standard Cypress run
npm run cypress:run:headed             # Cypress run with visible browser
npm run cypress:run:chrome             # Cypress run with Chrome
npm run cypress:run:firefox            # Cypress run with Firefox
npm run cypress:run:edge               # Cypress run with Edge
npm run cypress:open                   # Open Cypress Test Runner
```

### Smoke Tests
```bash
npm run e2e:smoke                      # Run smoke tests (headless)
npm run e2e:smoke:headed               # Run smoke tests (headed)
npm run e2e:smoke:with-backend         # Run smoke tests and start backend
```

## Shell Script Runner

The `run-e2e-tests.sh` script provides a complete testing environment:

### Features

- **Automatic Setup**: Builds AgentGateway and starts required services
- **Service Management**: Manages backend and UI development servers
- **Health Checks**: Waits for services to be ready before running tests
- **Cleanup**: Automatic cleanup of processes and resources
- **Error Handling**: Comprehensive error handling and reporting

### Usage Examples

```bash
# Basic usage (parallel mode)
./scripts/run-e2e-tests.sh

# Sequential mode
./scripts/run-e2e-tests.sh --mode sequential

# Interactive mode (opens Cypress UI)
./scripts/run-e2e-tests.sh --mode interactive

# Custom worker count
./scripts/run-e2e-tests.sh --workers 8

# Headed mode with no cleanup
./scripts/run-e2e-tests.sh --headed --no-cleanup

# Verbose logging
./scripts/run-e2e-tests.sh --verbose
```

### Environment Variables

```bash
AGENTGATEWAY_BINARY=/path/to/binary    # Custom binary path
SKIP_BUILD=true                        # Skip building AgentGateway
SKIP_BACKEND=true                      # Skip starting backend
SKIP_UI=true                          # Skip starting UI
CI=true                               # Enable CI mode
```

## Reports and Artifacts

### Generated Reports

#### JSON Report (`cypress/reports/parallel-test-results.json`)
```json
{
  "summary": {
    "execution": {
      "startTime": "2024-01-01T00:00:00.000Z",
      "endTime": "2024-01-01T00:01:30.000Z",
      "totalDuration": 90000,
      "parallelEfficiency": {
        "efficiency": 0.85,
        "speedup": 3.2,
        "percentageImprovement": 68.75
      }
    },
    "workers": {
      "total": 4,
      "successful": 4,
      "failed": 0
    },
    "tests": {
      "total": 24,
      "passed": 24,
      "failed": 0,
      "passRate": 100
    }
  }
}
```

#### Text Summary (`cypress/reports/parallel-test-summary.txt`)
```
Parallel Test Execution Summary
===============================

Execution Details:
- Start Time: 2024-01-01T00:00:00.000Z
- End Time: 2024-01-01T00:01:30.000Z
- Total Duration: 90.0s
- Parallel Efficiency: 85.0%
- Speed Improvement: 68.8%

Worker Statistics:
- Total Workers: 4
- Successful: 4
- Failed: 0

Test Results:
- Total Tests: 24
- Passed: 24
- Failed: 0
- Pass Rate: 100.0%
```

### Video Recordings

- **Location**: `cypress/videos/`
- **Format**: MP4
- **Scope**: Failed tests (configurable)
- **Retention**: Automatic cleanup after successful runs

### Screenshots

- **Location**: `cypress/screenshots/`
- **Format**: PNG
- **Trigger**: Test failures
- **Context**: Full page screenshots with timestamp

## Best Practices

### Development Workflow

1. **Smoke Tests First**: Always run smoke tests for rapid feedback
   ```bash
   npm run e2e:smoke
   ```

2. **Parallel Development**: Use parallel execution for comprehensive testing
   ```bash
   npm run test:e2e:parallel:dev
   ```

3. **Debug Mode**: Use debug mode for investigating failures
   ```bash
   npm run test:e2e:parallel:debug
   ```

4. **Browser Testing**: Test across browsers before release
   ```bash
   npm run test:e2e:parallel:chrome
   npm run test:e2e:parallel:firefox
   ```

### CI/CD Integration

1. **Resource Optimization**: Use CI mode for resource-constrained environments
   ```bash
   npm run test:e2e:parallel:ci
   ```

2. **Headed Debugging**: Use headed CI mode for debugging CI failures
   ```bash
   npm run test:e2e:parallel:ci:headed
   ```

3. **Artifact Collection**: Collect videos and screenshots for failed tests
4. **Report Integration**: Parse JSON reports for CI/CD metrics

### Performance Optimization

1. **Worker Tuning**: Adjust worker count based on system capabilities
2. **Memory Limits**: Configure memory limits for your environment
3. **Test Grouping**: Organize tests by execution characteristics
4. **Resource Monitoring**: Monitor resource usage and adjust accordingly

## Troubleshooting

### Common Issues

#### High Memory Usage
```bash
# Reduce worker count
npm run test:e2e:parallel -- --workers 2

# Lower memory limit
npm run test:e2e:parallel -- --memory-limit 70
```

#### Slow Test Discovery
```bash
# Enable debug mode to see discovery process
npm run test:e2e:parallel:debug
```

#### Worker Failures
```bash
# Check system resources
npm run test:e2e:parallel -- --debug

# Run sequentially to isolate issues
npm run test:e2e:sequential
```

#### CI Timeouts
```bash
# Use CI mode with conservative settings
npm run test:e2e:parallel:ci

# Reduce worker count for CI
npm run test:e2e:parallel:ci -- --workers 2
```

### Debug Information

Enable debug mode for detailed logging:
```bash
npm run test:e2e:parallel:debug
```

This provides:
- Test discovery details
- Worker allocation decisions
- Resource usage metrics
- Scheduling algorithm decisions
- Error stack traces

### Log Analysis

Key log patterns to look for:

- `🔍 Searching for tests`: Test discovery phase
- `📋 Scheduling tests`: Test distribution phase
- `🔧 Using X workers`: Worker allocation
- `📊 Aggregating results`: Result collection
- `⚠️ Memory usage high`: Resource warnings
- `🚨 EMERGENCY`: Resource emergencies

## Contributing

### Adding New Test Groups

1. **Define Group**: Add to `getDefaultTestGroups()` in `test-scheduler.js`
2. **Set Constraints**: Configure memory, workers, and timing
3. **Update Documentation**: Document the new group characteristics

### Extending Resource Monitoring

1. **Add Metrics**: Extend `ResourceMonitor` class
2. **Set Thresholds**: Configure warning and emergency thresholds
3. **Add Handlers**: Implement response to new metrics

### Improving Scheduling

1. **Algorithm Enhancement**: Modify scheduling algorithms
2. **Constraint Addition**: Add new scheduling constraints
3. **Optimization**: Improve load balancing algorithms

## Architecture Decisions

### Why Parallel Testing?
- **Speed**: Reduces test execution time by 60-80%
- **Resource Utilization**: Better use of multi-core systems
- **Scalability**: Scales with available system resources

### Why Bin-Packing?
- **Optimal Distribution**: Minimizes total execution time
- **Load Balancing**: Prevents worker idle time
- **Resource Efficiency**: Maximizes system utilization

### Why Resource Monitoring?
- **Stability**: Prevents system crashes from resource exhaustion
- **Reliability**: Ensures consistent test execution
- **Safety**: Protects development environments

### Why Multiple Browsers?
- **Compatibility**: Ensures cross-browser functionality
- **Coverage**: Catches browser-specific issues
- **Confidence**: Increases deployment confidence

## Future Enhancements

### Planned Features

1. **Test Sharding**: Distribute tests across multiple machines
2. **Historical Analytics**: Track test performance over time
3. **Flaky Test Detection**: Identify and handle unreliable tests
4. **Dynamic Scaling**: Auto-scale workers based on test queue
5. **Cloud Integration**: Support for cloud testing services

### Performance Improvements

1. **Caching**: Cache test analysis results
2. **Predictive Scheduling**: Use ML for better test time prediction
3. **Resource Prediction**: Predict resource needs before execution
4. **Parallel Setup**: Parallelize test environment setup

