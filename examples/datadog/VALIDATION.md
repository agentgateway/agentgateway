# Development validation

Validated locally on 2026-08-31 using Linux ARM64 containers on Docker Desktop.
These results cover the development candidate, not Datadog publication or
production readiness.

## Versions

- agentgateway: `ghcr.io/agentgateway/agentgateway:v1.5.0`
  (predates the provider error-status fix in PR #3261)
- Datadog Agent: `registry.datadoghq.com/agent:7.82.3`
- Community check candidate: `datadog-agentgateway` 1.0.0 (unpublished)
- Collector image tag: `otel/opentelemetry-collector-contrib:0.146.0`
  (the image reports service version 0.145.0 at startup)

## Passed

| Check | Result |
| --- | --- |
| Released proxy with synthetic provider | Success, HTTP 500/429, streaming, token/cache accounting, estimated cost, TTFT and TPOT |
| Local OTLP capture through Collector | GenAI attributes, service identity, W3C parents, HTTP error normalization |
| Content controls | No prompt/completion attributes by default; explicit synthetic capture produces JSON message arrays |
| Stock OpenMetrics check in Agent | 46 metric samples, 238 histogram buckets, one health service check; no errors or warnings |
| Named check wheel in custom Agent image | Installation succeeds; same sample/bucket counts; no errors or warnings |
| Named check with traffic between scrapes | Positive request, token and estimated cost counter deltas |
| Community package tests | 11 passed, including the running proxy, parser regression, controller fixtures, label filtering, counter reset submissions, endpoint failure and mapping parity |
| Python checks | Ruff and mypy pass; two deprecation warnings originate in Datadog-generated Pydantic models |
| Datadog asset validation | Metadata, dashboards, package, service checks, README, HTTP usage, imports, signatures, integration style, config/models, OpenMetrics limit and CODEOWNERS pass |
| Kubernetes configuration | Annotation/config parsing and chart rendering checked; no cluster deployment performed |
| Real OpenAI configuration | Compose parsing and proxy startup with placeholder credentials; paid API calls and Datadog cost estimation were not exercised |

The counter reset test verifies submission through the Agent's monotonic-count
API. It does not establish multi-replica correctness in a live Datadog account.
Controller metrics use fixtures; MCP and guardrail traffic have not been
exercised end to end.

## Reproduce

From this directory:

```sh
docker compose up -d
uv run smoke.py
docker compose -f compose.yaml -f compose.content.yaml up -d
uv run smoke.py --capture-content
docker compose up -d
uv run smoke.py
```

From a `DataDog/integrations-extras` checkout containing the candidate and
configured as the `extras` repository in `ddev`:

```sh
AGENTGATEWAY_METRICS_ENDPOINT=http://127.0.0.1:18520/metrics ddev -e test agentgateway
ddev -e test -s agentgateway
ddev -e validate metadata agentgateway
ddev -e validate dashboards agentgateway
ddev -e validate config agentgateway
ddev -e validate models agentgateway
uv build --wheel agentgateway
docker build -t agentgateway-datadog-agent:dev agentgateway
```

Follow the README for trial ingestion. Run the check twice with traffic between
scrapes; a first scrape establishes counter baselines. A local check's output
is not evidence of Datadog intake or product conversion.

## Remaining acceptance and publication work

- Configure the trial API key and actual site in the ignored `.env` file.
- Verify metric ingestion, distribution percentiles, dashboard rendering and
  monitor queries in the account. No monitors have been created remotely.
- Enable/verify LLM Observability and check models, token usage, error states,
  parent relationships and synthetic content in that product, not just APM.
- Test Kubernetes Autodiscovery, controller reconciliation, independent replicas,
  restarts and potential duplicate scrapers in a disposable cluster.
- Exercise MCP, guardrails, additional providers, missing usage, cancellation,
  timeouts and long streams; measure overhead and cardinality under load.
- Confirm Solo.io maintainer/contact details and complete Datadog's publisher
  requirements, tile media and any assigned catalog identifiers. Generated
  manifest identifiers are not evidence of catalog registration.
- Submit and review the example and community check PRs. The extras checkout
  currently lacks the required `danehans` remote; it has not been pushed.
- Validate installation of Datadog's published package before documenting a
  catalog install command or claiming the integration is available there.

The default test stack makes no paid model calls and uses no account key.
Stop it with `docker compose down` after validation.
