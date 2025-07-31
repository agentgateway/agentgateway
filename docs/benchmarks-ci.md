# GitHub CI Benchmarks

AgentGateway provides comprehensive performance benchmarking capabilities through GitHub Actions workflows. These benchmarks can be triggered manually by maintainers to validate performance claims and compare against industry standards.

## Overview

The benchmarking system includes:

- **Manual Triggers**: Maintainer-only access through GitHub Actions UI
- **PR Comment Integration**: `/benchmark` commands in pull request comments
- **Multi-Protocol Support**: HTTP, MCP, and A2A protocol testing
- **Industry Comparisons**: Automated comparison with nginx, HAProxy, Envoy, and Pingora
- **Dynamic Baselines**: Automatic updates of industry benchmark data
- **Professional Reporting**: HTML and Markdown reports with detailed analysis

## Quick Start

### Manual Benchmark Trigger

1. Navigate to the **Actions** tab in the GitHub repository
2. Select **Manual Benchmarks (Maintainers Only)**
3. Click **Run workflow**
4. Configure your benchmark:
   - **Protocols**: `all`, `http`, `mcp`, or `a2a`
   - **Test Type**: `quick`, `comprehensive`, `latency`, or `throughput`
   - **Duration**: `30s`, `60s`, `120s`, or `300s`
   - **Platform**: `ubuntu-latest`, `ubuntu-22.04-arm`, or `both`
5. Click **Run workflow** to start

### PR Comment Commands

Maintainers can trigger benchmarks directly from pull request comments:

```bash
# Quick test of all protocols (30s duration)
/benchmark

# Test specific protocol
/benchmark http

# Comprehensive test with longer duration
/benchmark all comprehensive 120s

# Latency-focused test for MCP protocol
/benchmark mcp latency

# Test on both platforms
/benchmark http quick 60s both
```

## Configuration Options

### Protocols

- **`all`**: Test all supported protocols (HTTP, MCP, A2A)
- **`http`**: HTTP proxy performance only
- **`mcp`**: Model Context Protocol performance
- **`a2a`**: Agent-to-Agent protocol performance

### Test Types

- **`quick`**: Fast validation (recommended for PR testing)
  - Lower concurrency levels
  - Shorter warm-up periods
  - Essential metrics only

- **`comprehensive`**: Full performance analysis
  - Multiple concurrency levels (16, 64, 256, 512)
  - Extended warm-up and measurement periods
  - Complete metric collection

- **`latency`**: Latency-focused testing
  - Lower concurrency to minimize queuing
  - Focus on p50, p95, p99, p99.9 latencies
  - Detailed latency distribution analysis

- **`throughput`**: Throughput-focused testing
  - High concurrency levels
  - Focus on requests per second
  - Connection reuse optimization

### Duration Options

- **`30s`**: Quick validation (default)
- **`60s`**: Standard testing
- **`120s`**: Extended analysis
- **`300s`**: Comprehensive long-running tests

### Platform Options

- **`ubuntu-latest`**: Standard x86_64 testing (default)
- **`ubuntu-22.04-arm`**: ARM64 testing
- **`both`**: Run on both platforms for comparison

## Understanding Results

### Benchmark Reports

Each benchmark run generates:

1. **HTML Report**: Visual comparison with charts and detailed analysis
2. **Markdown Summary**: Documentation-friendly performance summary
3. **Raw JSON Data**: Detailed metrics for further analysis

### Key Metrics

#### Latency Metrics
- **p50**: Median response time (50th percentile)
- **p95**: 95th percentile response time
- **p99**: 99th percentile response time
- **p99.9**: 99.9th percentile response time

#### Throughput Metrics
- **QPS**: Queries (requests) per second
- **Connection Reuse**: Efficiency of connection pooling
- **Success Rate**: Percentage of successful requests

#### Industry Comparisons
- **nginx**: TechEmpower Round 23 results
- **HAProxy**: TechEmpower Round 23 results
- **Envoy**: Official Envoy benchmarks
- **Pingora**: Cloudflare published results

### Performance Targets

#### HTTP Proxy Performance
- **Target**: <5ms p95 latency, >50,000 QPS
- **Baseline**: nginx (2.1ms p95, 125K QPS)

#### MCP Protocol Performance
- **Target**: <10ms p95 latency, >10,000 QPS
- **Baseline**: HTTP proxy performance (no direct MCP baselines available)

#### A2A Protocol Performance
- **Target**: <15ms p95 latency, >5,000 QPS
- **Baseline**: HTTP proxy performance

## Access Control

### Maintainer-Only Access

Benchmark workflows are restricted to users with `admin` or `maintain` permissions on the repository. This ensures:

- **Resource Management**: Prevents abuse of CI/CD resources
- **Quality Control**: Ensures benchmarks are run by knowledgeable users
- **Cost Control**: Manual triggers only, no automatic execution

### Permission Verification

The system automatically checks user permissions using the GitHub API:

```bash
# Example permission check
curl -H "Authorization: token $GITHUB_TOKEN" \
  "https://api.github.com/repos/owner/repo/collaborators/username/permission"
```

Users without proper permissions will see clear error messages with instructions to contact maintainers.

## Artifact Management

### Retention Policy

- **Retention Period**: 30 days
- **Artifact Limit**: Last 3 benchmark runs per configuration
- **Automatic Cleanup**: Old artifacts are automatically removed

### Artifact Contents

Each benchmark run produces:

```
benchmark-results-{platform}-{run_number}/
├── benchmark_comparison_report.html    # Visual report with charts
├── benchmark_summary.md               # Markdown summary
├── fortio_results_*.json             # Raw Fortio output
├── system_info.json                  # System specifications
└── baseline_comparison.json          # Industry comparison data
```

## Dynamic Baseline Updates

### Automatic Updates

Before each benchmark run, the system checks for updates to industry baselines:

- **TechEmpower Framework**: Latest round results
- **Vendor Blogs**: Cloudflare, Fastly, etc.
- **Academic Papers**: Recent proxy performance studies
- **GitHub Releases**: Performance notes from major proxies

### Change Notifications

When baselines are updated, the benchmark report includes:

- **Change Summary**: What metrics changed and by how much
- **Source Attribution**: Where the new data came from
- **Impact Analysis**: How changes affect AgentGateway comparisons

Example baseline update notice:
```markdown
## 📊 Baseline Updates

Industry baselines were updated before this benchmark run:

- nginx: p95 latency changed from 2.1ms to 1.9ms (TechEmpower Round 24)
- Pingora: throughput changed from 200K QPS to 220K QPS (Cloudflare Blog 2025)
```

## Notifications

### Email Notifications

Professional email notifications are sent to maintainers when benchmarks complete, fail, or are cancelled. Features include:

- **HTML Email Reports**: Professional styling with status indicators and action buttons
- **Performance Summaries**: Detailed configuration and results attached as text files
- **Baseline Update Alerts**: Notifications when industry baselines change
- **Fallback Mechanisms**: GitHub issues created if email delivery fails
- **Status-Specific Content**: Different templates for success, failure, and cancellation

### Notification Setup

To enable email notifications, repository maintainers must configure GitHub secrets:

- `NOTIFICATION_EMAIL_USER`: SMTP username (e.g., `agentgateway-ci@company.com`)
- `NOTIFICATION_EMAIL_PASSWORD`: App-specific password for the email account
- `MAINTAINER_EMAILS`: Comma-separated list of recipient email addresses

**See [Email Notification Setup](./notification-setup.md) for detailed configuration instructions.**

### Notification Content

#### Success Notifications
- ✅ Status indicator with benchmark configuration
- Performance summary with key metrics vs baselines
- Links to detailed results and artifacts
- Baseline update information (if applicable)
- Next steps for result analysis

#### Failure Notifications
- ❌ Status indicator with error context
- Configuration details for debugging
- Links to workflow logs and error details
- Troubleshooting guidance and escalation recommendations

#### Baseline Update Notifications
- 📊 Industry baseline changes detected before benchmark run
- Source information (TechEmpower, vendor releases, etc.)
- Impact assessment and confidence scores
- Integration with benchmark results

### Notification Channels

Currently implemented:
- **Email Notifications**: Professional HTML emails to maintainers
- **GitHub Actions Logs**: Always available in workflow runs
- **PR Comments**: Automated result comments for `/benchmark` commands
- **Fallback GitHub Issues**: Created automatically if email delivery fails

### Testing Notifications

To test the notification system:

1. Ensure all required secrets are configured
2. Trigger a manual benchmark via GitHub Actions UI
3. Enable the "Send notification to maintainers" option
4. Check email inboxes and GitHub issues for delivery

Test command via PR comment:
```bash
/benchmark http quick 30s
```

## Troubleshooting

### Common Issues

#### Permission Denied
```
❌ Unauthorized: Only maintainers can trigger benchmark workflows
```
**Solution**: Contact a repository maintainer to request access or have them run the benchmark.

#### Workflow Timeout
```
⚠️ Benchmark Cancelled or Skipped
Status: timeout
```
**Solution**: Try a shorter duration or simpler test type. Comprehensive tests may take longer than the 60-minute timeout.

#### Docker Build Failures
```
❌ Benchmark Failed
Error Details: Docker build failed
```
**Solution**: Check the workflow logs for specific Docker errors. This may indicate infrastructure issues.

### Getting Help

1. **Check Workflow Logs**: Detailed execution logs are available in the GitHub Actions tab
2. **Review Documentation**: This guide covers most common scenarios
3. **Contact Maintainers**: Repository maintainers can help with access and configuration issues
4. **Open Issues**: For bugs or feature requests, open a GitHub issue

## Advanced Usage

### Custom Configurations

For advanced users, the benchmarking infrastructure supports:

- **Custom Payloads**: Modify JSON payloads in `crates/agentgateway/benches/traffic/payloads/`
- **Custom Configurations**: Adjust proxy configurations in `crates/agentgateway/benches/traffic/configs/`
- **Extended Metrics**: Add custom metrics to the report generation system

### Local Development

To run benchmarks locally:

```bash
# Navigate to benchmark directory
cd crates/agentgateway/benches/traffic/docker

# Run with default settings
./run-docker-benchmarks.sh

# Run with custom configuration
./run-docker-benchmarks.sh --protocols http --type comprehensive --duration 60s
```

### Integration with Development Workflow

Recommended benchmark usage:

1. **PR Validation**: Use `/benchmark http quick` for fast HTTP validation
2. **Feature Testing**: Use `/benchmark all comprehensive` for new protocol features
3. **Performance Regression**: Use `/benchmark all latency` to check for regressions
4. **Release Validation**: Use comprehensive tests before major releases

## Contributing

### Adding New Protocols

To add support for a new protocol:

1. Create configuration file in `crates/agentgateway/benches/traffic/configs/`
2. Add test payloads in `crates/agentgateway/benches/traffic/payloads/`
3. Update the main test orchestrator script
4. Add protocol option to GitHub Actions workflows

### Improving Baselines

To improve baseline accuracy:

1. Research latest industry benchmark data
2. Update `update-baselines.py` with new data sources
3. Add validation for new baseline sources
4. Test baseline update detection

### Enhancing Reports

To enhance benchmark reports:

1. Modify `generate-comparison.py` for new visualizations
2. Add new metrics to the comparison system
3. Improve HTML report templates
4. Add new analysis capabilities

## Security Considerations

### Resource Protection

- **Manual Triggers Only**: No automatic benchmark execution
- **Permission Verification**: GitHub API-based access control
- **Resource Limits**: 60-minute timeout per workflow
- **Artifact Cleanup**: Automatic cleanup prevents storage abuse

### Data Privacy

- **No Sensitive Data**: Benchmarks use synthetic payloads only
- **Public Results**: All benchmark results are publicly accessible
- **Industry Data**: Only publicly available baseline data is used

## Future Enhancements

### Planned Features

- **Historical Tracking**: Performance trend analysis over time
- **Regression Detection**: Automatic alerts for performance degradation
- **Cross-Platform Analysis**: Detailed ARM64 vs x86_64 comparisons
- **Custom Baselines**: Support for organization-specific baselines

### Community Contributions

We welcome contributions to improve the benchmarking system:

- **New Protocol Support**: Add support for additional protocols
- **Enhanced Reporting**: Improve visualization and analysis
- **Baseline Accuracy**: Better industry data sources
- **Performance Optimizations**: Improve benchmark execution efficiency

For questions or contributions, please open an issue or pull request in the repository.
