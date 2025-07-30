# AgentGateway Fortio Traffic Testing Suite

A comprehensive performance testing infrastructure for AgentGateway using Fortio load testing with real-world traffic patterns and industry baseline comparisons.

## Overview

This testing suite provides:

- **Multi-protocol testing**: HTTP, MCP, and A2A protocols
- **Real traffic simulation**: Using Fortio for realistic load patterns
- **Industry comparisons**: Automated comparison with published baselines from nginx, HAProxy, Envoy, and Pingora
- **Comprehensive reporting**: HTML and Markdown reports with detailed analysis

## Quick Start

### Prerequisites

1. **Fortio**: Install the Fortio load testing tool
   ```bash
   # Linux/macOS
   curl -L https://github.com/fortio/fortio/releases/download/v1.60.3/fortio_linux_amd64.tar.gz | tar xz
   sudo mv fortio /usr/local/bin/
   
   # Or using Go
   go install fortio.org/fortio@latest
   ```

2. **Python 3**: For report generation (with `jq` for JSON processing)
   ```bash
   # Ubuntu/Debian
   sudo apt install python3 jq
   
   # macOS
   brew install python3 jq
   ```

3. **AgentGateway**: Build the project
   ```bash
   cd /path/to/agentgateway
   cargo build --release --bin agentgateway
   cargo build --release --bin test-server
   ```

### Running Tests

#### Basic Usage

```bash
# Run all protocols with comprehensive testing
./fortio-tests.sh

# Quick HTTP-only tests
./fortio-tests.sh --protocols http --type quick

# MCP protocol tests with custom duration
./fortio-tests.sh --protocols mcp --duration 30s

# Verbose output
./fortio-tests.sh --verbose
```

#### Available Options

- `--protocols`: `all`, `http`, `mcp`, `a2a`
- `--type`: `comprehensive`, `quick`, `latency`, `throughput`
- `--duration`: Test duration (e.g., `60s`, `2m`)
- `--verbose`: Enable detailed output
- `--help`: Show usage information

## Test Scenarios

### HTTP Proxy Tests

- **Latency tests**: Measure response times under various concurrency levels
- **Throughput tests**: Maximum QPS with unlimited rate
- **Payload tests**: Different payload sizes (1KB, 10KB, 100KB)
- **Concurrency scaling**: 16, 64, 256, 512 concurrent connections

### MCP Protocol Tests

- **Message types**: `initialize`, `list_resources`, `call_tool`, `get_prompt`
- **Session management**: Concurrent session handling
- **Protocol overhead**: MCP-specific performance characteristics

### A2A Protocol Tests

- **Operations**: `discovery`, `capability_exchange`, `message_routing`
- **Multi-hop communication**: Agent-to-agent routing performance
- **Protocol efficiency**: A2A-specific optimizations

## Directory Structure

```
traffic/
├── fortio-tests.sh              # Main test orchestrator
├── configs/                     # Proxy configurations
│   ├── http-proxy.yaml         # HTTP proxy config
│   ├── mcp-proxy.yaml          # MCP proxy config
│   └── a2a-proxy.yaml          # A2A proxy config
├── payloads/                    # Test payloads
│   ├── mcp-*.json              # MCP message payloads
│   └── a2a-*.json              # A2A message payloads
├── reports/                     # Report generation
│   └── generate-comparison.py   # Comparison report generator
├── results/                     # Test results (generated)
└── README.md                   # This file
```

## Configuration Files

### HTTP Proxy Configuration

Optimized for performance testing with:
- Disabled health checks
- Optimized HTTP/2 settings
- Minimal logging
- No tracing overhead

### MCP Proxy Configuration

Configured for MCP protocol testing with:
- Larger message size limits
- MCP-specific timeouts
- Optimized connection pooling

### A2A Proxy Configuration

Tuned for A2A protocol with:
- Multi-hop routing support
- Discovery timeouts
- Session management

## Payload Files

### MCP Payloads

- `mcp-initialize.json`: Protocol initialization
- `mcp-list_resources.json`: Resource listing
- `mcp-call_tool.json`: Tool invocation
- `mcp-get_prompt.json`: Prompt retrieval

### A2A Payloads

- `a2a-discovery.json`: Agent discovery
- `a2a-capability_exchange.json`: Capability negotiation
- `a2a-message_routing.json`: Message routing

## Industry Baselines

The comparison system includes published baselines from:

### TechEmpower Benchmarks (Round 23)
- **nginx**: 125,000 QPS, 2.1ms p95 latency
- **HAProxy**: 118,000 QPS, 2.3ms p95 latency
- **Hardware**: Intel Xeon Gold 6230R, 52 cores, 256GB RAM

### Vendor Benchmarks
- **Envoy Proxy**: 95,000 QPS, 3.1ms p95 latency (AWS c5.4xlarge)
- **Cloudflare Pingora**: 200,000 QPS, 1.8ms p95 latency (production)

## Report Generation

### Automated Reports

The test suite automatically generates:

1. **HTML Report**: Comprehensive visual report with charts and comparisons
2. **Markdown Summary**: Documentation-friendly summary
3. **JSON Results**: Raw Fortio results for further analysis

### Manual Report Generation

```bash
# Generate reports from existing results
cd reports/
python3 generate-comparison.py ../results/

# Custom output files
python3 generate-comparison.py ../results/ \
  --output-html custom-report.html \
  --output-md custom-summary.md
```

## Understanding Results

### Key Metrics

- **p50/p95/p99**: Latency percentiles (lower is better)
- **QPS**: Queries per second (higher is better)
- **Success Rate**: Percentage of successful requests
- **Throughput**: Data transfer rate

### Comparison Categories

- **Better**: >10% improvement over baseline
- **Similar**: Within 10% of baseline
- **Worse**: >10% degradation from baseline

### Performance Targets

Based on industry standards:

- **HTTP Proxy**: Target <5ms p95 latency, >50,000 QPS
- **MCP Protocol**: Target <10ms p95 latency, >10,000 QPS
- **A2A Protocol**: Target <15ms p95 latency, >5,000 QPS

## Troubleshooting

### Common Issues

1. **Fortio not found**
   ```bash
   # Install Fortio
   go install fortio.org/fortio@latest
   ```

2. **Port conflicts**
   ```bash
   # Check for running services
   netstat -tlnp | grep :8080
   # Kill conflicting processes
   sudo pkill -f agentgateway
   ```

3. **Permission errors**
   ```bash
   # Make scripts executable
   chmod +x fortio-tests.sh
   chmod +x reports/generate-comparison.py
   ```

4. **Python dependencies**
   ```bash
   # Install required packages
   pip3 install --user statistics pathlib
   ```

### Debug Mode

Enable verbose output for debugging:

```bash
./fortio-tests.sh --verbose --protocols http --type quick
```

### Manual Testing

Test individual components:

```bash
# Start backend server
./target/release/test-server --port 3001 &

# Start AgentGateway
./target/release/agentgateway --config traffic/configs/http-proxy.yaml &

# Run Fortio manually
fortio load -c 64 -t 30s http://localhost:8080/test

# Cleanup
pkill -f test-server
pkill -f agentgateway
```

## Integration with CI/CD

### GitHub Actions Example

```yaml
name: Performance Tests
on: [push, pull_request]

jobs:
  performance:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Install Fortio
        run: go install fortio.org/fortio@latest
      - name: Build AgentGateway
        run: cargo build --release
      - name: Run Performance Tests
        run: |
          cd crates/agentgateway/benches/traffic
          ./fortio-tests.sh --type quick
      - name: Upload Results
        uses: actions/upload-artifact@v3
        with:
          name: performance-results
          path: crates/agentgateway/benches/traffic/results/
```

### Performance Regression Detection

Set up automated alerts for performance regressions:

```bash
# Example threshold check
python3 -c "
import json
with open('results/http-latency-c64.json') as f:
    data = json.load(f)
    p95 = next(p['Value'] for p in data['DurationHistogram']['Percentiles'] if p['Percentile'] == 95)
    if p95 > 0.005:  # 5ms threshold
        exit(1)
"
```

## Contributing

### Adding New Test Scenarios

1. **Create payload files** in `payloads/` directory
2. **Update configuration** in `configs/` if needed
3. **Modify test script** to include new scenarios
4. **Update baselines** in `generate-comparison.py`

### Extending Baseline Comparisons

Add new baselines to `PUBLISHED_BASELINES` in `generate-comparison.py`:

```python
"new_proxy": {
    "source": "Benchmark Source",
    "source_url": "https://example.com/benchmarks",
    "test_date": "2024-XX-XX",
    "hardware": "Hardware specification",
    "scenarios": {
        "test_name": {
            "p50_ms": 1.0,
            "p95_ms": 2.5,
            "p99_ms": 5.0,
            "qps": 100000,
            "notes": "Test description"
        }
    }
}
```

## Performance Optimization Tips

### System Tuning

```bash
# Increase file descriptor limits
ulimit -n 65536

# Optimize network settings
echo 'net.core.somaxconn = 65536' | sudo tee -a /etc/sysctl.conf
echo 'net.ipv4.tcp_max_syn_backlog = 65536' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

### AgentGateway Tuning

- **Worker threads**: Set `WORKER_THREADS` environment variable
- **Connection pooling**: Tune HTTP/2 settings in configuration
- **Memory allocation**: Monitor and optimize connection state management

### Test Environment

- **Dedicated hardware**: Run tests on dedicated machines
- **Network isolation**: Minimize network interference
- **Resource monitoring**: Monitor CPU, memory, and network during tests

## References

- [Fortio Documentation](https://fortio.org/)
- [TechEmpower Benchmarks](https://www.techempower.com/benchmarks/)
- [Envoy Proxy Performance](https://www.envoyproxy.io/docs/envoy/latest/faq/performance/)
- [Cloudflare Pingora](https://blog.cloudflare.com/how-we-built-pingora-the-proxy-that-connects-cloudflare-to-the-internet/)
