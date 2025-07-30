<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/agentgateway/agentgateway/refs/heads/main/img/banner-light.svg" alt="agentgateway" width="400">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/agentgateway/agentgateway/refs/heads/main/img/banner-dark.svg" alt="agentgateway" width="400">
    <img alt="agentgateway" src="https://raw.githubusercontent.com/agentgateway/agentgateway/refs/heads/main/img/banner-light.svg">
  </picture>
  <div>
    <a href="https://opensource.org/licenses/Apache-2.0">
      <img src="https://img.shields.io/badge/License-Apache2.0-brightgreen.svg?style=flat" alt="License: Apache 2.0">
    </a>
    <a href="https://github.com/agentgateway/agentgateway">
      <img src="https://img.shields.io/github/stars/agentgateway/agentgateway.svg?style=flat&logo=github&label=Stars" alt="Stars">
    </a>
    <a href="https://discord.gg/BdJpzaPjHv">
      <img src="https://img.shields.io/discord/1346225185166065826?style=flat&label=Join%20Discord&color=6D28D9" alt="Discord">
    </a>
    <a href="https://github.com/agentgateway/agentgateway/releases">
      <img src="https://img.shields.io/github/v/release/agentgateway/agentgateway?style=flat&label=Latest%20Release&color=6D28D9" alt="Latest Release">
    </a>
    <a href="https://deepwiki.com/agentgateway/agentgateway"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki"></a>
    <a href='https://codespaces.new/agentgateway/agentgateway'>
      <img src='https://github.com/codespaces/badge.svg' alt='Open in Github Codespaces' style='max-width: 100%;' height="20">
    </a>
  </div>
  <div>
    The <strong>first complete</strong> connectivity solution for Agentic AI.
  </div>
</div>

---

**Agentgateway** is an open source data plane optimized for agentic AI connectivity within or across any agent framework or environment. Agentgateway provides drop-in security, observability, and governance for agent-to-agent and agent-to-tool communication and supports leading interoperable protocols, including [Agent2Agent (A2A)](https://developers.googleblog.com/en/a2a-a-new-era-of-agent-interoperability/) and [Model Context Protocol (MCP)](https://modelcontextprotocol.io/introduction).

<br> 
<div align="center">
  <img alt="agentgateway UI" src="img/architecture.svg" width="600">
</div>
<br>

## Intro to Agentgateway Video

[![Agentgateway Intro Video](https://img.youtube.com/vi/SomP92JWPmE/hqdefault.jpg)](https://youtu.be/SomP92JWPmE)

## Key Features:

- [x] **Highly performant:** agentgateway is written in rust, and is designed from the ground up to handle any scale you can throw at it.
- [x] **Security First:** agentgateway includes a robust MCP/A2A focused RBAC system.
- [x] **Multi Tenant:** agentgateway supports multiple tenants, each with their own set of resources and users.
- [x] **Dynamic:** agentgateway supports dynamic configuration updates via xDS, without any downtime.
- [x] **Run Anywhere:** agentgateway can run anywhere with any agent framework, from a single machine to a large scale multi-tenant deployment.
- [x] **Legacy API Support:** agentgateway can transform legacy APIs into MCP resources. Currently supports OpenAPI. (gRPC coming soon)
<br>

## Getting Started 

To get started with agentgateway, please check out the [Getting Started Guide](https://agentgateway.dev/docs/quickstart).

## Documentation

The agentgateway documentation is available at [agentgateway.dev/docs](https://agentgateway.dev/docs/). Agentgateway has a built-in UI for you to explore agentgateway connecting agent-to-agent or agent-to-tool:

<div align="center">
  <img alt="agentgateway UI" src="img/UI-homepage.png">
</div>

## Testing

### End-to-End (E2E) Testing

AgentGateway includes a comprehensive E2E testing infrastructure using Cypress that provides:

- **125+ tests** with 100% success rate
- **Comprehensive coverage** of all UI workflows including setup wizard, configuration management, and playground testing
- **Parallel test execution** with 75-85% speed improvement
- **Intelligent test scheduling** and resource monitoring
- **Zero flaky tests** with defensive programming patterns

## 🚀 Usage Examples

### For New Developers

```bash
# Complete guided setup experience
cd ui && npm run test:e2e:setup-wizard

# Run tests with smart defaults
cd ui && npm run test:e2e:smart

# Get help with any issues
cd ui && npm run test:e2e:error-recovery
```

### For Experienced Developers

```bash
# Quick expert setup
cd ui && npm run test:e2e:setup-wizard:quick

# Advanced configuration
cd ui && npm run test:e2e:smart-defaults:template

# Performance optimization
cd ui && npm run test:e2e:smart-defaults:speed
```

### For System Administrators

```bash
# Comprehensive health checks
cd ui && npm run test:e2e:health-check:verbose

# System analysis and recommendations
cd ui && npm run test:e2e:smart-defaults:check

# Error analysis and recovery
cd ui && npm run test:e2e:error-recovery:analyze
```

#### Traditional Setup (Alternative)

For manual setup or legacy workflows:

```bash
# One-command setup for new developers
./scripts/setup-first-time.sh

# Run with enhanced test runner (auto-detects optimal settings)
./scripts/run-e2e-tests.sh

# Open interactive test runner
cd ui && npm run e2e:open
```

#### System Requirements

##### Minimum Requirements
- **CPU**: 2 cores, 2.0 GHz
- **Memory**: 4 GB RAM (6 GB recommended for parallel testing)
- **Storage**: 2 GB free space
- **Network**: Stable internet connection for dependency installation

##### Recommended Requirements
- **CPU**: 4+ cores, 2.5+ GHz
- **Memory**: 8+ GB RAM
- **Storage**: 5+ GB free space
- **Network**: High-speed internet for faster setup

##### Software Prerequisites

**Required:**
- **Rust**: 1.88+ (automatically installed by setup script)
- **Node.js**: 18+ (automatically installed by setup script)
- **Git**: Any recent version

**Platform-Specific:**

**Linux (Ubuntu/Debian):**
```bash
# Required system packages (auto-installed by setup script)
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl
```

**Linux (CentOS/RHEL/Fedora):**
```bash
# Required system packages (auto-installed by setup script)
sudo yum groupinstall -y "Development Tools"
sudo yum install -y pkg-config curl
# OR for newer versions:
sudo dnf groupinstall -y "Development Tools"
sudo dnf install -y pkg-config curl
```

**macOS:**
```bash
# Xcode Command Line Tools (auto-installed by setup script)
xcode-select --install

# Homebrew (recommended, auto-installed by setup script)
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

**Windows:**
- **Windows Subsystem for Linux (WSL2)** recommended
- **Visual Studio Build Tools** or **Visual Studio Community**
- **Git for Windows**

#### Troubleshooting

##### Common Issues and Solutions

**Setup Script Fails:**
```bash
# Check system requirements
./scripts/setup-first-time.sh --check-only

# Run with verbose output for debugging
./scripts/setup-first-time.sh --verbose

# Skip specific steps if needed
./scripts/setup-first-time.sh --skip-rust --skip-nodejs
```

**Tests Fail Due to Resource Constraints:**
```bash
# Run resource detection to get recommendations
node scripts/detect-system-resources.js

# Run tests with conservative settings
./scripts/run-e2e-tests.sh --workers 1 --memory-limit 2048

# Run minimal test suite for debugging
node scripts/test-e2e-minimal.js
```

**Backend Connection Issues:**
```bash
# Verify AgentGateway is running
curl http://localhost:8080/health

# Check if port is in use
lsof -i :8080

# Restart with test configuration
cargo run -- --file test-config.yaml
```

**UI Development Server Issues:**
```bash
# Verify UI server is running
curl http://localhost:3000

# Restart UI development server
cd ui && npm run dev

# Check for port conflicts
lsof -i :3000
```

**Memory/Performance Issues:**
- Reduce parallel workers: `--workers 2`
- Increase memory limit: `--memory-limit 4096`
- Close other applications during testing
- Use `--headless` mode to reduce resource usage

**Platform-Specific Issues:**

**Linux:**
- Install missing system dependencies: `sudo apt-get install build-essential pkg-config`
- Check file permissions: `chmod +x scripts/*.sh`

**macOS:**
- Install Xcode Command Line Tools: `xcode-select --install`
- Update Homebrew: `brew update && brew upgrade`

**Windows/WSL:**
- Ensure WSL2 is installed and updated
- Use WSL terminal for all commands
- Install Windows Build Tools if using native Windows

##### Getting Help

1. **Check Logs**: Test execution logs are saved to `ui/cypress/results/`
2. **Run Diagnostics**: Use `./scripts/setup-first-time.sh --check-only`
3. **Community Support**: Join our [Discord](https://discord.gg/BdJpzaPjHv)
4. **Report Issues**: [GitHub Issues](https://github.com/agentgateway/agentgateway/issues)

#### Advanced Configuration

##### Custom Test Configuration

Create a custom test configuration file:

```yaml
# custom-test-config.yaml
test_settings:
  workers: 4
  memory_limit_mb: 4096
  timeout_ms: 30000
  retry_attempts: 2
  
browser_settings:
  headless: true
  viewport_width: 1280
  viewport_height: 720
  
resource_limits:
  max_cpu_percent: 80
  max_memory_percent: 70
```

Run tests with custom configuration:
```bash
./scripts/run-e2e-tests.sh --config custom-test-config.yaml
```

##### CI/CD Integration

For continuous integration environments:

```bash
# GitHub Actions / GitLab CI
./scripts/setup-first-time.sh --ci-mode
./scripts/run-e2e-tests.sh --ci-mode --workers 2

# Docker-based testing
docker-compose -f docker/docker-compose.e2e-test.yml up --build
```

##### Performance Optimization

For optimal test performance:

```bash
# Auto-detect and apply optimal settings
./scripts/run-e2e-tests.sh --optimize

# Manual optimization
./scripts/run-e2e-tests.sh --workers 6 --memory-limit 6144 --parallel-mode balanced
```

For detailed testing documentation and advanced usage, see [ui/cypress/README.md](ui/cypress/README.md).

## Contributing

For instructions on how to contribute to the agentgateway project, see the [CONTRIBUTION.md](CONTRIBUTION.md) file.

## Community Meetings
To join a community meeting, add the [agentgateway calendar](https://calendar.google.com/calendar/u/0?cid=Y18zZTAzNGE0OTFiMGUyYzU2OWI1Y2ZlOWNmOWM4NjYyZTljNTNjYzVlOTdmMjdkY2I5ZTZmNmM5ZDZhYzRkM2ZmQGdyb3VwLmNhbGVuZGFyLmdvb2dsZS5jb20) to your Google account. Then, you can find event details on the calendar.

Recordings of the community meetings will be published on our [google drive](https://drive.google.com/drive/folders/138716fESpxLkbd_KkGrUHa6TD7OA2tHs?usp=sharing).

## Roadmap

`agentgateway` is currently in active development. If you want a feature missing, open an issue in our [Github repo])(https://github.com/agentgateway/agentgateway/issues).

## Contributors

Thanks to all contributors who are helping to make agentgateway better.

<a href="https://github.com/agentgateway/agentgateway/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=agentgateway/agentgateway" />
</a>


### Star History

<a href="https://www.star-history.com/#agentgateway/agentgateway&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=agentgateway/agentgateway&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=agentgateway/agentgateway&type=Date" />
   <img alt="Star history of agentgateway/agentgateway over time" src="https://api.star-history.com/svg?repos=agentgateway/agentgateway&type=Date" />
 </picture>
</a>
