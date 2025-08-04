# AgentGateway Docker-based Fortio Benchmarking

This directory contains a complete Docker-based benchmarking infrastructure for AgentGateway using Fortio load testing tools. This setup provides a consistent, reproducible environment for performance testing across different systems.

## 🏗️ Architecture

The Docker setup consists of four main services:

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│  Fortio Client  │───▶│  AgentGateway   │───▶│   Test Server   │
│   (Load Test)   │    │     (Proxy)     │    │   (Backend)     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                                │
                                ▼
                       ┌─────────────────┐
                       │ Report Generator│
                       │   (Analysis)    │
                       └─────────────────┘
```

### Services

1. **agentgateway**: The proxy being tested
2. **test-server**: Backend server that handles requests
3. **fortio-benchmark**: Fortio load testing tool
4. **report-generator**: Generates comparison reports and analysis

## 🚀 Quick Start

### Prerequisites

- Docker (20.10+)
- Docker Compose (2.0+)
- At least 4GB RAM available for containers
- 2+ CPU cores recommended

### Basic Usage

```bash
# Run quick HTTP tests
./run-docker-benchmarks.sh --protocols http --type quick

# Run comprehensive tests for all protocols
./run-docker-benchmarks.sh --protocols all --type comprehensive

# Run MCP protocol tests with custom duration
./run-docker-benchmarks.sh --protocols mcp --duration 60s --verbose
```

## 📋 Command Reference

### Main Runner Script

```bash
./run-docker-benchmarks.sh [OPTIONS]
```

#### Options

| Option | Description | Default | Example |
|--------|-------------|---------|---------|
| `--protocols` | Protocols to test: `all`, `http`, `mcp`, `a2a` | `all` | `--protocols http` |
| `--type` | Test type: `comprehensive`, `quick`, `latency`, `throughput` | `quick` | `--type comprehensive` |
| `--duration` | Test duration | `30s` | `--duration 60s` |
| `--no-build` | Skip building Docker images | false | `--no-build` |
| `--no-cleanup` | Skip cleanup after tests | false | `--no-cleanup` |
| `--verbose` | Enable verbose output | false | `--verbose` |
| `--help` | Show help message | - | `--help` |

#### Examples

```bash
# Quick start - run all protocols with quick tests
./run-docker-benchmarks.sh

# HTTP performance testing
./run-docker-benchmarks.sh --protocols http --type comprehensive --duration 120s

# MCP protocol focus
./run-docker-benchmarks.sh --protocols mcp --type latency --verbose

# Development mode (skip build, keep containers)
./run-docker-benchmarks.sh --no-build --no-cleanup --verbose
```

## 🔧 Manual Docker Operations

### Building Images

```bash
# Build Fortio benchmark image
docker build -f Dockerfile.fortio -t agentgateway-fortio ..

# Build test server image
docker build -f Dockerfile.test-server -t agentgateway-test-server ../../../..
```

### Running Services

```bash
# Start infrastructure services
docker-compose -f docker-compose.benchmark.yml up -d agentgateway test-server

# Run benchmarks
docker-compose -f docker-compose.benchmark.yml run --rm fortio-benchmark \
  ./fortio-tests.sh --protocols http --type quick

# Generate reports
docker-compose -f docker-compose.benchmark.yml run --rm report-generator

# Cleanup
docker-compose -f docker-compose.benchmark.yml down
```

## 📊 Test Types

### Quick Tests
- **Duration**: 30s per test
- **Concurrency**: 16, 64
- **Focus**: Basic functionality validation
- **Use Case**: CI/CD, development validation

### Comprehensive Tests
- **Duration**: 60s per test
- **Concurrency**: 16, 64, 256, 512
- **Payload Sizes**: 1KB, 10KB, 100KB
- **Focus**: Complete performance characterization
- **Use Case**: Release validation, performance analysis

### Latency Tests
- **Duration**: 60s per test
- **Concurrency**: 16, 32, 64
- **Focus**: Response time optimization
- **Use Case**: Latency-sensitive applications

### Throughput Tests
- **Duration**: 60s per test
- **Concurrency**: 64, 256, 512
- **Focus**: Maximum request rate
- **Use Case**: High-load scenarios

## 🌐 Protocol Testing

### HTTP Proxy
- **Endpoint**: `http://agentgateway:8080/test`
- **Tests**: Latency, throughput, payload sizes
- **Metrics**: p50, p95, p99 latency, QPS, error rate

### MCP Protocol
- **Endpoint**: `http://agentgateway:8080/mcp`
- **Message Types**: initialize, list_resources, call_tool, get_prompt
- **Tests**: Message-specific performance, concurrent sessions
- **Metrics**: Protocol overhead, session management

### A2A Protocol
- **Endpoint**: `http://agentgateway:8080/a2a`
- **Operations**: discovery, capability_exchange, message_routing
- **Tests**: Operation-specific performance, multi-hop communication
- **Metrics**: Protocol efficiency, routing performance

## 📈 Results and Reports

### Output Structure

```
traffic/results/
├── http-latency-c16.json           # HTTP latency test, 16 concurrent
├── http-throughput-c64.json        # HTTP throughput test, 64 concurrent
├── mcp-initialize.json             # MCP initialize message test
├── a2a-discovery.json              # A2A discovery operation test
├── benchmark_comparison_report.html # Visual HTML report
└── benchmark_summary.md            # Markdown summary
```

### Report Contents

#### HTML Report
- Executive summary with key metrics
- Interactive charts and graphs
- Industry baseline comparisons
- Performance recommendations

#### Markdown Summary
- Tabular results overview
- Performance categorization
- Quick reference metrics

### Industry Baselines

The reports include comparisons with:

| Proxy | p95 Latency | Throughput | Source |
|-------|-------------|------------|---------|
| nginx | 2.1ms | 125K QPS | TechEmpower Round 23 |
| HAProxy | 2.3ms | 118K QPS | TechEmpower Round 23 |
| Envoy | 3.1ms | 95K QPS | Envoy Benchmarks 2024 |
| Pingora | 1.8ms | 200K QPS | Cloudflare Blog 2024 |

## 🔍 Troubleshooting

### Common Issues

#### Docker Build Failures
```bash
# Check Docker daemon
docker info

# Clean build cache
docker system prune -f

# Rebuild with verbose output
./run-docker-benchmarks.sh --verbose
```

#### Service Health Check Failures
```bash
# Check service logs
docker-compose -f docker-compose.benchmark.yml logs agentgateway
docker-compose -f docker-compose.benchmark.yml logs test-server

# Manual health check
curl http://localhost:8080/health
curl http://localhost:3001/health
```

#### Performance Issues
```bash
# Check system resources
docker stats

# Increase test duration
./run-docker-benchmarks.sh --duration 120s

# Reduce concurrency
./run-docker-benchmarks.sh --type quick
```

### Debug Mode

```bash
# Run with maximum verbosity
./run-docker-benchmarks.sh --verbose --no-cleanup

# Access container for debugging
docker-compose -f docker-compose.benchmark.yml run --rm fortio-benchmark bash

# Check Fortio version
docker run --rm fortio/fortio version
```

## 🔧 Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AGENTGATEWAY_URL` | Proxy endpoint | `http://agentgateway:8080` |
| `BACKEND_URL` | Backend endpoint | `http://test-server:3001` |
| `FORTIO_DOCKER` | Docker mode flag | `true` |

### Custom Configurations

#### Proxy Configuration
Edit `../configs/http-proxy.yaml` to modify AgentGateway settings:

```yaml
listeners:
  - address: "0.0.0.0:8080"
    protocol: http
routes:
  - match:
      path: "/test"
    backend:
      address: "test-server:3001"
```

#### Test Payloads
Modify files in `../payloads/` to customize test data:

```json
{
  "jsonrpc": "2.0",
  "method": "initialize",
  "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {}
  }
}
```

## 🚀 CI/CD Integration

### GitHub Actions Example

```yaml
name: Performance Benchmarks

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Run Performance Tests
        run: |
          cd crates/agentgateway/benches/traffic/docker
          ./run-docker-benchmarks.sh --protocols all --type quick
      
      - name: Upload Results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: crates/agentgateway/benches/traffic/results/
```

### Jenkins Pipeline

```groovy
pipeline {
    agent any
    
    stages {
        stage('Benchmark') {
            steps {
                dir('crates/agentgateway/benches/traffic/docker') {
                    sh './run-docker-benchmarks.sh --protocols all --type comprehensive'
                }
            }
        }
        
        stage('Archive Results') {
            steps {
                archiveArtifacts artifacts: 'crates/agentgateway/benches/traffic/results/*'
                publishHTML([
                    allowMissing: false,
                    alwaysLinkToLastBuild: true,
                    keepAll: true,
                    reportDir: 'crates/agentgateway/benches/traffic/results',
                    reportFiles: 'benchmark_comparison_report.html',
                    reportName: 'Performance Report'
                ])
            }
        }
    }
}
```

## 📚 Advanced Usage

### Custom Test Scenarios

Create custom test scenarios by modifying the Docker Compose file:

```yaml
# Add custom test service
custom-test:
  build:
    context: ..
    dockerfile: docker/Dockerfile.fortio
  command: >
    sh -c "
      fortio load -c 100 -t 60s -qps 1000 
      -json /benchmarks/results/custom-test.json
      http://agentgateway:8080/custom-endpoint
    "
  depends_on:
    agentgateway:
      condition: service_healthy
```

### Performance Monitoring

```bash
# Monitor resource usage during tests
docker stats --format "table {{.Container}}\t{{.CPUPerc}}\t{{.MemUsage}}"

# Continuous monitoring
watch -n 1 'docker stats --no-stream'
```

### Scaling Tests

```bash
# Scale backend servers
docker-compose -f docker-compose.benchmark.yml up -d --scale test-server=3

# Run distributed tests
docker-compose -f docker-compose.benchmark.yml run --rm \
  -e BACKEND_URL=http://test-server:3001,http://test-server:3002 \
  fortio-benchmark ./fortio-tests.sh
```

## 🎯 Performance Targets

### HTTP Proxy Targets
- **p95 Latency**: < 5ms
- **Throughput**: > 50,000 QPS
- **Error Rate**: < 0.1%

### MCP Protocol Targets
- **p95 Latency**: < 10ms
- **Throughput**: > 10,000 QPS
- **Session Overhead**: < 5%

### A2A Protocol Targets
- **p95 Latency**: < 15ms
- **Throughput**: > 5,000 QPS
- **Routing Efficiency**: > 95%

## 🤝 Contributing

### Adding New Tests

1. Create test payload in `../payloads/`
2. Add test configuration in `../configs/`
3. Update `fortio-tests.sh` with new test logic
4. Update Docker Compose if needed
5. Document in this README

### Improving Performance

1. Profile using Docker stats
2. Analyze Fortio results
3. Compare with industry baselines
4. Submit performance improvements

## 📄 License

This benchmarking infrastructure is part of the AgentGateway project and follows the same license terms.
