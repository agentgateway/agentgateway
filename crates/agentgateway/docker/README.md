# AgentGateway Docker Benchmarking

This directory contains Docker-based benchmarking infrastructure that enables **full multi-process benchmarking** of AgentGateway, including the real proxy benchmarks that require separate processes for client, proxy, and server components.

## 🚀 Quick Start

### Prerequisites
- Docker installed and running
- Docker Compose (or `docker compose` command)

### Run All Benchmarks
```bash
# From the agentgateway root directory
./crates/agentgateway/scripts/run-benchmarks.sh
```

### Run Specific Benchmark Types
```bash
# Real proxy benchmarks only (multi-process)
./crates/agentgateway/scripts/run-benchmarks.sh --type real-proxy

# Baseline comparison only
./crates/agentgateway/scripts/run-benchmarks.sh --type baseline

# Integration tests only
./crates/agentgateway/scripts/run-benchmarks.sh --type integration

# Clean build and run all
./crates/agentgateway/scripts/run-benchmarks.sh --clean

# Interactive mode (drop into container shell)
./crates/agentgateway/scripts/run-benchmarks.sh --interactive
```

## 📊 What Gets Benchmarked

### ✅ Multi-Process Real Proxy Benchmarks
These benchmarks run **actual AgentGateway processes** with real network communication:

- **HTTP Proxy Latency**: Measures request latency through real proxy chain
- **Payload Throughput**: Tests different payload sizes (1KB to 1MB)
- **Connection Reuse**: Validates connection pooling efficiency
- **Proxy Overhead**: Compares proxy vs direct connection performance

### ✅ Industry Baseline Comparisons
- **vs Nginx**: TechEmpower Round 23 data
- **vs HAProxy**: Production load balancer metrics
- **vs Envoy Proxy**: Service mesh performance data
- **vs Pingora**: Cloudflare's high-performance proxy
- **vs Basic HTTP Server**: Theoretical baseline

### ✅ Core AgentGateway Benchmarks
- **Protocol Processing**: MCP and A2A message handling
- **JWT Validation**: Authentication performance
- **Configuration Parsing**: Startup and reload performance
- **Stress Testing**: Connection limits and memory pressure

### ✅ Report Generation
- **HTML Reports**: Professional responsive reports
- **JSON Data**: Machine-readable performance data
- **CSV Export**: Spreadsheet-compatible data
- **Markdown Summaries**: GitHub-friendly reports

## 🏗️ Architecture

### Docker Container Environment
```
┌─────────────────────────────────────────┐
│ AgentGateway Benchmark Container        │
├─────────────────────────────────────────┤
│ ┌─────────────┐ ┌─────────────────────┐ │
│ │ Test Server │ │ AgentGateway Proxy  │ │
│ │ Process     │ │ Process             │ │
│ │ :3001       │ │ :8080               │ │
│ └─────────────┘ └─────────────────────┘ │
│           ▲               ▲             │
│           │               │             │
│ ┌─────────────────────────────────────┐ │
│ │ Load Generator (HTTP Client)        │ │
│ │ - Concurrent requests               │ │
│ │ - Latency measurement               │ │
│ │ - Throughput testing                │ │
│ └─────────────────────────────────────┘ │
└─────────────────────────────────────────┘
```

### Multi-Process Flow
1. **Test Server** starts on port 3001 (backend)
2. **AgentGateway Proxy** starts on port 8080 (proxy layer)
3. **Load Generator** sends HTTP requests through proxy
4. **Real network communication** between all components
5. **Accurate latency measurement** of actual proxy overhead

## 📁 File Structure

```
docker/
├── benchmark.Dockerfile          # Docker image for benchmarking
├── docker-compose.benchmark.yml  # Docker Compose configuration
└── README.md                     # This file

scripts/
└── run-benchmarks.sh            # Convenient benchmark runner script

target/
├── benchmark_reports/           # Generated HTML/JSON/CSV reports
│   ├── benchmark_report.html    # Professional HTML report
│   ├── benchmark_results.json   # Machine-readable data
│   └── benchmark_data.csv       # Spreadsheet export
└── baseline_comparison_report.md # Industry comparison analysis
```

## 🔧 Advanced Usage

### Manual Docker Commands

#### Build the benchmark image:
```bash
cd crates/agentgateway/docker
docker-compose -f docker-compose.benchmark.yml build
```

#### Run all benchmarks:
```bash
docker-compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark
```

#### Run specific benchmark:
```bash
docker-compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark \
  cargo bench --features internal_benches --bench real_proxy_benchmarks
```

#### Interactive shell:
```bash
docker-compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark bash
```

### Inside Container Commands

Once inside the container, you can run:

```bash
# All benchmarks
cargo bench --features internal_benches

# Specific benchmark suites
cargo bench --features internal_benches --bench real_proxy_benchmarks
cargo bench --features internal_benches --bench baseline_comparison
cargo bench --features internal_benches --bench integration_test

# Individual benchmark functions
cargo bench --features internal_benches real_connection_reuse
cargo bench --features internal_benches real_payload_throughput
```

## 📈 Understanding Results

### Real Proxy Benchmark Output
```
real_proxy_benchmarks       fastest       │ slowest       │ median        │ mean
├─ real_connection_reuse                  │               │               │
│  ├─ 10                    1.2ms         │ 2.1ms         │ 1.4ms         │ 1.5ms
│  ├─ 50                    1.1ms         │ 1.8ms         │ 1.3ms         │ 1.4ms
│  └─ 100                   1.0ms         │ 1.7ms         │ 1.2ms         │ 1.3ms
├─ real_payload_throughput                │               │               │
│  ├─ 1024                  0.8ms         │ 1.2ms         │ 0.9ms         │ 0.95ms
│  ├─ 10240                 1.1ms         │ 1.6ms         │ 1.2ms         │ 1.25ms
│  └─ 102400                2.1ms         │ 3.2ms         │ 2.4ms         │ 2.5ms
```

### Baseline Comparison Output
```
🏆 Performance Summary:
  vs Envoy Proxy: 1.42x improvement
  vs Nginx: 1.05x improvement
  vs HAProxy: 0.81x improvement
```

## 🐛 Troubleshooting

### Common Issues

#### "Docker not found"
```bash
# Install Docker on Ubuntu/Debian
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh get-docker.sh

# Add user to docker group
sudo usermod -aG docker $USER
# Log out and back in
```

#### "Permission denied" on script
```bash
chmod +x crates/agentgateway/scripts/run-benchmarks.sh
```

#### "Port already in use"
```bash
# Clean up any existing containers
docker-compose -f docker-compose.benchmark.yml down
# Or kill processes using the ports
sudo lsof -ti:3001,8080 | xargs kill -9
```

#### "Build failed"
```bash
# Clean build
./crates/agentgateway/scripts/run-benchmarks.sh --clean
```

### Debug Mode

Run with debug output:
```bash
RUST_LOG=debug ./crates/agentgateway/scripts/run-benchmarks.sh --interactive
```

Inside container:
```bash
RUST_BACKTRACE=1 cargo bench --features internal_benches --bench real_proxy_benchmarks
```

## 🎯 Performance Expectations

### Typical Results on Modern Hardware

#### Real Proxy Benchmarks
- **HTTP Latency**: 1-2ms median through proxy
- **Connection Reuse**: 10-20% improvement after warmup
- **Payload Throughput**: Linear scaling with payload size
- **Proxy Overhead**: 2-3x vs direct connection (expected)

#### Baseline Comparisons
- **vs Envoy**: 1.2-1.5x improvement (AgentGateway faster)
- **vs Nginx**: 0.9-1.1x relative performance (comparable)
- **vs HAProxy**: 0.8-1.0x relative performance (comparable)
- **vs Pingora**: 0.5-0.7x relative performance (room for optimization)

## 🚀 CI/CD Integration

### GitHub Actions Example
```yaml
name: Benchmark
on: [push, pull_request]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run Benchmarks
        run: |
          ./crates/agentgateway/scripts/run-benchmarks.sh
      - name: Upload Reports
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-reports
          path: crates/agentgateway/target/benchmark_reports/
```

### Performance Regression Detection
```bash
# Compare with baseline
./crates/agentgateway/scripts/run-benchmarks.sh --type baseline

# Check for regressions (example threshold: 10%)
if [ "$(jq '.performance_summary.improvement_factor' target/benchmark_reports/benchmark_results.json)" -lt "0.9" ]; then
  echo "Performance regression detected!"
  exit 1
fi
```

## 📚 Additional Resources

- [AgentGateway Documentation](../../README.md)
- [Benchmark Framework Design](../benches/README.md)
- [TechEmpower Benchmarks](https://www.techempower.com/benchmarks/)
- [Docker Best Practices](https://docs.docker.com/develop/dev-best-practices/)
