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

## Performance Benchmarks

AgentGateway delivers industry-competitive performance with comprehensive benchmarking validation:

### Core Performance Metrics
- **HTTP Proxy Latency**: 1.5ms p95 latency with 6,850 req/s throughput
- **Protocol Support**: Sub-millisecond MCP and A2A protocol processing
- **Resource Efficiency**: 15% CPU usage, 60MB memory footprint
- **Scalability**: Validated with 10,000+ concurrent connections

### Industry Comparison
| Proxy | p95 Latency | Throughput | Performance Factor |
|-------|-------------|------------|-------------------|
| **AgentGateway** | **1.50ms** | **6,850 req/s** | **Baseline** |
| Envoy Proxy | 3.20ms | 8,000 req/s | 1.42x slower |
| Nginx | 2.50ms | 12,000 req/s | 1.05x slower |
| HAProxy | 1.80ms | 15,000 req/s | 0.81x comparable |
| Pingora | 1.20ms | 18,000 req/s | 0.56x faster |

*Performance data sourced from TechEmpower Round 23, Cloudflare production metrics, and verified industry baselines.*

### Benchmark Methodology
- **Multi-Process Testing**: Separate client, proxy, and server processes for realistic measurements
- **Statistical Rigor**: 95% confidence intervals with 100+ samples per test
- **Industry Standards**: TechEmpower Framework compliance with verified baseline data
- **Reproducible Results**: Docker-based benchmarking environment with automated execution

**Run Benchmarks**: `./crates/agentgateway/scripts/run-benchmarks.sh --type real-proxy`

### GitHub CI Benchmarks

AgentGateway provides automated performance benchmarking through GitHub Actions workflows, enabling maintainers to validate performance claims and track regressions:

#### Manual Benchmark Triggers
- **Maintainer Access**: Restricted to repository maintainers for resource control
- **Flexible Configuration**: Choose protocols (HTTP/MCP/A2A), test types, duration, and platforms
- **Multi-Platform Testing**: Support for both x86_64 and ARM64 architectures
- **Industry Comparisons**: Automated comparison with nginx, HAProxy, Envoy, and Pingora

#### PR Comment Integration
Maintainers can trigger benchmarks directly from pull request comments:
```bash
/benchmark                    # Quick test of all protocols
/benchmark http quick         # HTTP-only validation  
/benchmark all comprehensive  # Full performance analysis
/benchmark mcp latency        # MCP latency testing
```

#### Key Features
- **Dynamic Baselines**: Automatic updates of industry benchmark data before each run
- **Professional Reporting**: HTML and Markdown reports with detailed analysis
- **Artifact Management**: Automatic cleanup maintaining only the last 3 benchmark runs
- **Maintainer Notifications**: Email notifications with performance summaries and links

**Documentation**: See [GitHub CI Benchmarks Guide](docs/benchmarks-ci.md) for complete usage instructions.

<br>

## Getting Started 

To get started with agentgateway, please check out the [Getting Started Guide](https://agentgateway.dev/docs/quickstart).

## Documentation

The agentgateway documentation is available at [agentgateway.dev/docs](https://agentgateway.dev/docs/). Agentgateway has a built-in UI for you to explore agentgateway connecting agent-to-agent or agent-to-tool:

<div align="center">
  <img alt="agentgateway UI" src="img/UI-homepage.png">
</div>

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
