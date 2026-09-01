# Datadog observability

This example targets **agentgateway v1.5.0** and **Datadog Agent 7.82.3**. It provides immediate metrics collection with the stock OpenMetrics check, plus a separate OTLP trace path for Datadog LLM Observability. No Datadog SDK is needed inside agentgateway.

The default Docker Compose configuration is `compose.yaml`. Running `docker compose up -d` without additional Compose files starts agentgateway, a deterministic OpenAI-compatible provider fixture, and a local OpenTelemetry Collector. The fixture also captures OTLP/HTTP protobuf traces for local assertions. This configuration does not call paid models or send data to Datadog. Adding `compose.datadog.yaml` as an override starts the Datadog Agent and exports synthetic telemetry to Datadog.

## Prerequisites

The local test requires:

- [Docker](https://docs.docker.com/get-docker/) with [Docker Compose](https://docs.docker.com/compose/install/), installed through Docker Desktop or the Compose CLI plugin.
- [uv](https://docs.astral.sh/uv/) to create the Python environment and run `smoke.py`.
- Network access on the first run to download container images and Python packages.
- Available loopback ports `13000`, `18080`, and `18520`.

Exporting the synthetic telemetry also requires a Datadog organization, its API key, and the correct [Datadog site](https://docs.datadoghq.com/getting_started/site/). Datadog LLM Observability must be enabled in the organization to verify traces in that product. The local test does not require a Datadog account.

The optional real-provider test requires an OpenAI API key with billing enabled and the name of an OpenAI model available to that account. It makes a paid API request. The selected model must support the OpenAI Chat Completions API.

## Run the local test

```sh
cd examples/datadog
docker compose up -d
uv run smoke.py
```

The test verifies HTTP success/500/429 responses, streaming, input/output/cache token accounting, estimated USD cost, time to first token, time per output token, OTLP/HTTP protobuf export, HTTP error status normalization, W3C parent propagation, and omission of prompt/response content. The provider's test prices are **synthetic**, not real provider pricing. Metrics are saved to ignored `var/metrics.txt`.

Traces pass through the OpenTelemetry Collector configured in `collector.yaml`. The Collector image version is fixed in `compose.yaml` for reproducible testing. In v1.5.0, a provider HTTP 429/500 is recorded in the GenAI span's `http.status` attribute, but the separate OpenTelemetry span status remains `Unset`. The gateway treats a received HTTP response as a successful transport result and sets span status to `Error` only when the upstream call itself returns an error, such as a connection failure. Consequently, an observability backend may receive the correct HTTP status without classifying the GenAI operation as an error. This was fixed on `main` by [PR #3261](https://github.com/agentgateway/agentgateway/pull/3261), after the v1.5.0 release.

Because this example runs v1.5.0, the Collector marks GenAI spans with `http.status >= 400` as errors and supplies `error.type` when missing. Treating a provider rejection such as 429 as a failed GenAI operation is deliberate. At the HTTP server-span layer, OpenTelemetry normally leaves 4xx status unset because the client may be at fault. This rule is scoped to GenAI operations rather than every gateway HTTP span. Successful spans and existing error types are preserved. Remove the `transform/gateway_errors` processor in `collector.yaml` after upgrading to a release that contains PR #3261; that release classifies provider responses in the gateway and records the numeric HTTP status as `error.type`.

Host ports bind only to IPv4 loopback: gateway `127.0.0.1:13000`, metrics `127.0.0.1:18520`, fixture capture endpoint `127.0.0.1:18080`. Inside the Compose network, the proxy still uses metrics port 15020. No admin port is published. If a port is occupied, adjust both Compose and the smoke test.

## Send synthetic telemetry to a Datadog trial

1. Create a local `.env` file containing the trial organization's API key and site. Keep this file private (`chmod 600 .env`); it is ignored by Git. Do not paste keys into issues, example YAML, screenshots, or command output.

   ```dotenv
   DD_API_KEY=replace-with-your-trial-api-key
   DD_SITE=us3.datadoghq.com
   ```

   Use the site where your organization actually resides; for US1 use `datadoghq.com`. API and application keys are different: an API key permits ingestion. Dashboard/API queries may additionally need a suitably scoped application key, but the example does not require one.

2. Start Docker Compose with the Datadog override. This adds the Datadog Agent and sends synthetic metrics and traces to the organization associated with `DD_API_KEY`:

   ```sh
   docker compose -f compose.yaml -f compose.datadog.yaml up -d
   uv run smoke.py --live
   docker compose -f compose.yaml -f compose.datadog.yaml exec datadog agent check openmetrics
   ```

   Do not share `docker compose config` or container inspection output: environment variables can contain the API key. This setup intentionally omits Docker socket/host filesystem mounts, container autodiscovery, and log collection; it needs only network access to the synthetic gateway. It is a development setup, not a production Agent deployment.

3. In Datadog, verify `agentgateway.requests.count`, `agentgateway.gen_ai.token.usage.sum`, and `agentgateway.gen_ai.cost.usd.count`, scoped to `env:datadog-dev`. A successful health check alone does not prove counters were collected. Counters need successive scrapes; repeat the smoke test across scrape intervals to populate rate charts.

4. Import `dashboard.json` using Datadog's dashboard JSON import. Enable percentile aggregations on the latency distributions in Metrics Summary for p95 panels. Scope the `env` variable to `datadog-dev`. Controller/MCP/guardrail panels remain empty until corresponding traffic exists.

5. Open LLM Observability and verify gateway LLM spans under `ml_app:agentgateway` or the configured service name. Confirm models, token counts, errors, and trace relationships—not just presence in APM. Allow several minutes for processing. LLM Observability must be enabled for the organization; a successful OTLP response alone is not proof of product ingestion.

   Agent Observability displays `COST UNAVAILABLE` for the synthetic `datadog-test` model because that model is not in Datadog's pricing catalog. This is expected. The synthetic cost calculated by agentgateway remains available in the `agw.ai.usage.cost.*` span tags and the `agentgateway.gen_ai.cost.usd.count` metric.

The `--live` smoke test verifies requests and local metrics; it **does not claim** to verify Datadog ingestion. The development override sends metadata-only traces through the Collector to the Agent's OTLP/HTTP receiver. Only synthetic data should be used in this example.

Stop the stack when finished to avoid unnecessary trial usage:

```sh
docker compose -f compose.yaml -f compose.datadog.yaml down
```

To return to local trace capture, stop the Datadog stack and run `docker compose up -d` without the override.

## Send a request to a real OpenAI model

The synthetic provider remains the default. To verify Datadog's cost estimate for a real model, stop that stack and explicitly start the real-provider configuration. Add the following values to the ignored `.env` file, selecting a model available to your OpenAI account:

```dotenv
OPENAI_API_KEY=replace-with-your-openai-api-key
OPENAI_MODEL=replace-with-a-chat-completions-model
```

Keep the OpenAI key private. Requests in this mode use the OpenAI API and may incur charges. Do not combine this configuration with `compose.content.yaml`; prompt and response capture remains disabled.

Start agentgateway with the real OpenAI configuration and the Datadog Agent:

```sh
docker compose down
docker compose -f compose.openai.yaml -f compose.datadog.yaml up -d
```

Send one request through the `openai-live` alias:

```sh
curl http://127.0.0.1:13000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "openai-live",
    "messages": [{"role": "user", "content": "Reply with exactly: Datadog test complete."}],
    "max_completion_tokens": 32
  }'
```

Allow several minutes for processing, then find the span in Datadog Agent Observability under `ml_app:agentgateway`. Datadog can estimate cost when the configured provider and returned model name match its pricing catalog and token usage is present. This estimate uses Datadog's pricing data; compare it separately with the cost attributes and metrics calculated by agentgateway.

Stop the real-provider stack when finished:

```sh
docker compose -f compose.openai.yaml -f compose.datadog.yaml down
```

## Production metrics configuration

Use `openmetrics.yaml` with the stock Datadog Agent, changing `gateway:15020` and the tags to match your deployment. The file collects all proxy and runtime metric families except the per-resource MCP request counter. Explicit mappings preserve the metric names used by the dashboard; `raw_metric_prefix` removes the source `agentgateway_` prefix before the wildcard collects the remaining families.

**Keep `use_latest_spec: true` for the v1.5.0 proxy.** It serves OpenMetrics counter TYPE names with a `text/plain` content type. Datadog's default Prometheus parser can silently omit request, cost, and other counters. The Go controller uses the Prometheus parser (`use_latest_spec: false`).

### Kubernetes

Choose one proxy configuration method; do not apply both proxy files to the same workload:

- Standalone proxy Helm chart: merge `kubernetes/proxy-values.yaml` into its values.
- Controller-provisioned proxy: use `kubernetes/proxy-parameters.yaml` and reference it from the Gateway's `spec.infrastructure.parametersRef`. Adapt the namespace and merge with existing parameters. For DaemonSet proxies, put the same pod-template overlay under `spec.daemonSet`.

When running the controller, additionally merge `kubernetes/controller-values.yaml` into its Helm values to collect controller metrics on port 9092. This configuration applies to the controller pod and is independent of the proxy method selected above. It collects every metric exposed by the controller, including Go and process runtime metrics, and preserves the curated names used by the dashboard for reconciliation metrics. Datadog treats these series as custom metrics, so review their volume and tag cardinality for your deployment.

The Autodiscovery annotation identifier must match the container name: `agentgateway` for the proxy, `controller` for the controller. Configure the Datadog Agent in that cluster separately. These examples do not install it or change your existing cluster.

Use `%%host%%` and scrape each pod individually. Do not scrape a load-balanced Service that mixes process-local counters. Prometheus annotations or PodMonitors alone do not configure Datadog unless you separately enable Prometheus discovery. Avoid duplicate collection through simultaneous discovery methods. Keep management ports private; allow only the appropriate Datadog Agent traffic through NetworkPolicy. Do not expose admin port 15000.

### Metric meanings and limits

| Datadog metric | Meaning |
| --- | --- |
| `agentgateway.requests.count` | HTTP request counter; use status/reason tags for errors and `protocol:mcp` for MCP transport traffic |
| `agentgateway.request.duration` | HTTP latency distribution, in seconds |
| `agentgateway.gen_ai.request.duration` | LLM request latency distribution, in seconds |
| `agentgateway.gen_ai.time_to_first_token` | Time-to-first-token distribution, in seconds |
| `agentgateway.gen_ai.time_per_output_token` | Time-per-output-token distribution, in seconds |
| `agentgateway.gen_ai.token.usage.sum` | Token totals, separated by `token_type` |
| `agentgateway.gen_ai.cost.usd.count` | Estimated cumulative USD for requests with known usage/pricing |
| `agentgateway.cost_catalog.lookups.count` | Pricing lookup outcomes, including missing/unpriced models |
| `agentgateway.controller.reconciliations.count` | Optional controller reconciliation counter |

Histograms also export monotonic `.sum` and `.count`. Token histogram `.count` counts observations, **not tokens**. Input tokens already include cached input, so do not add cache categories to input totals. Cost is incomplete when usage/pricing is missing; it is not an invoice. Error status is not a default GenAI histogram dimension, so default metrics cannot provide per-model error rates.

Built-in series identity labels are preserved for the collected metric families. The label allowlist prevents user-configured custom dimensions from being exported automatically. Per-resource MCP counters are deliberately omitted because their `resource` label can contain unbounded or sensitive tool names and URIs; HTTP MCP traffic remains available. To collect `mcp_requests`, remove it from `exclude_metrics`, explicitly map it to `mcp.requests`, and add `resource`, `resource_type`, and `server` to `include_labels` after reviewing the possible values. Do not drop `resource` while collecting this counter because doing so can merge independent series incorrectly.

Avoid request IDs, user IDs, prompts, and arbitrary URLs as metric labels. If you customize gateway metric dimensions, include all distinguishing bounded labels or aggregate upstream before scraping. The generic OpenMetrics check counts these as custom metrics; budget for tags and distributions and retain its sample limit.

## Optional prompt/completion capture

By default no prompts or completions leave the gateway. To enable prompt and completion capture for the synthetic test data, add the content-capture override:

```sh
docker compose -f compose.yaml -f compose.content.yaml up -d
uv run smoke.py --capture-content
```

To test synthetic content in the trial account, also include `-f compose.datadog.yaml` and use `smoke.py --live`. Restore metadata-only mode with `docker compose up -d` (or the Datadog override without the content override).

The content-capture override adds the following to `config.tracing.fields.add`:

```yaml
gen_ai.input.messages: 'llm.prompt'
gen_ai.output.messages: 'llm.completion.map(c, {"role":"assistant", "content":c})'
```

The values must arrive as valid JSON message arrays in Datadog. Use synthetic traffic first. Before production, apply content redaction and size limits, choose sampling, and review access controls in both LLM Observability and APM. Capturing content adds memory/serialization overhead. Enabling content capture is not a redaction mechanism. Gateway observability covers only operations crossing the proxy; it does not replace application instrumentation or configure quality evaluations automatically.

## Migrate to the named integration

The proposed `DataDog/integrations-extras/agentgateway` check packages the mappings and dashboard. It is not bundled with the Agent or published yet. Install its versioned wheel in a custom Agent image, configure the check name `agentgateway` with `component: proxy` or `controller`, and remove the corresponding generic `openmetrics` instance. Never run both against the same endpoint. Metric names remain unchanged; first counter samples after migration establish a new baseline.

The default metrics dashboard works with either check. The scrape-health service check is `agentgateway.openmetrics.health` for both configurations because both use the `agentgateway` namespace.

## Validation status

The local synthetic stack and the real Agent check can be tested without an account. Account ingestion, dashboard rendering, LLM Observability conversion, live Kubernetes deployment, multi-replica behavior, and overhead/load testing require separate acceptance checks. Do not equate local test success with completion of those checks.

References: [OpenMetrics](https://docs.datadoghq.com/integrations/openmetrics/), [Kubernetes Autodiscovery](https://docs.datadoghq.com/containers/kubernetes/integrations/), [LLM Observability OTLP](https://docs.datadoghq.com/llm_observability/instrumentation/otel_instrumentation/), [Agent Observability cost](https://docs.datadoghq.com/llm_observability/investigate/cost/), [community installation](https://docs.datadoghq.com/agent/guide/use-community-integrations/).
