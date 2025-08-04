# AgentGateway Baseline Comparison Report

## Executive Summary

AgentGateway shows best performance compared to Envoy Proxy (1.4227083333333335x improvement) and most challenging comparison against Basic HTTP Server (0.38303333333333334x relative performance).

## Detailed Comparisons

### vs Envoy Proxy

**Improvement Factor:** 1.4227083333333335x

**Analysis:** AgentGateway significantly outperforms Envoy Proxy (1.4227083333333335x improvement). 113% lower latency. 14% lower throughput. 25% higher memory usage

**Performance Metrics:**

| Metric | AgentGateway | Baseline | Ratio |
|--------|--------------|----------|-------|
| p95 Latency (ms) | 1.50 | 3.20 | 2.13x |
| Throughput (req/s) | 6850 | 8000 | 0.86x |
| Memory Usage (MB) | 60.0 | 45.0 | 0.75x |
| CPU Usage (%) | 15.0 | 20.0 | 1.33x |

### vs Nginx

**Improvement Factor:** 1.0504166666666668x

**Analysis:** AgentGateway performs better than Nginx (1.0504166666666668x improvement). 66% lower latency. 42% lower throughput. 58% higher memory usage

**Performance Metrics:**

| Metric | AgentGateway | Baseline | Ratio |
|--------|--------------|----------|-------|
| p95 Latency (ms) | 1.50 | 2.50 | 1.67x |
| Throughput (req/s) | 6850 | 12000 | 0.57x |
| Memory Usage (MB) | 60.0 | 25.0 | 0.42x |
| CPU Usage (%) | 15.0 | 15.0 | 1.00x |

### vs HAProxy

**Improvement Factor:** 0.8119999999999999x

**Analysis:** AgentGateway performs comparably to HAProxy (0.8119999999999999x relative performance). 19% lower latency. 54% lower throughput. 50% higher memory usage

**Performance Metrics:**

| Metric | AgentGateway | Baseline | Ratio |
|--------|--------------|----------|-------|
| p95 Latency (ms) | 1.50 | 1.80 | 1.20x |
| Throughput (req/s) | 6850 | 15000 | 0.46x |
| Memory Usage (MB) | 60.0 | 30.0 | 0.50x |
| CPU Usage (%) | 15.0 | 12.0 | 0.80x |

### vs Pingora (Cloudflare)

**Improvement Factor:** 0.5641666666666666x

**Analysis:** AgentGateway underperforms Pingora (Cloudflare) (0.5641666666666666x relative performance). 20% higher latency. 61% lower throughput. 66% higher memory usage

**Performance Metrics:**

| Metric | AgentGateway | Baseline | Ratio |
|--------|--------------|----------|-------|
| p95 Latency (ms) | 1.50 | 1.20 | 0.80x |
| Throughput (req/s) | 6850 | 18000 | 0.38x |
| Memory Usage (MB) | 60.0 | 20.0 | 0.33x |
| CPU Usage (%) | 15.0 | 8.0 | 0.53x |

### vs Basic HTTP Server

**Improvement Factor:** 0.38303333333333334x

**Analysis:** AgentGateway underperforms Basic HTTP Server (0.38303333333333334x relative performance). 46% higher latency. 72% lower throughput. 75% higher memory usage

**Performance Metrics:**

| Metric | AgentGateway | Baseline | Ratio |
|--------|--------------|----------|-------|
| p95 Latency (ms) | 1.50 | 0.80 | 0.53x |
| Throughput (req/s) | 6850 | 25000 | 0.27x |
| Memory Usage (MB) | 60.0 | 15.0 | 0.25x |
| CPU Usage (%) | 15.0 | 5.0 | 0.33x |

## Methodology

Baseline data is sourced from public benchmarks and industry reports. Improvement factors are calculated using weighted scoring: latency (40%), throughput (30%), memory efficiency (15%), CPU efficiency (15%).

## Recommendations

⚠️  AgentGateway performance lags behind some industry standards.
🔧 Prioritize performance optimizations before making strong claims.
📊 Consider additional benchmarking to identify bottlenecks.
