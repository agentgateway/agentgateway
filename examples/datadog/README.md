# Datadog observability

This example targets **agentgateway v1.5.0** and **Datadog Agent 7.82.3**. It provides immediate metrics collection with the stock OpenMetrics check, plus a separate OTLP trace path for Datadog LLM Observability. No Datadog SDK is needed inside agentgateway.

The default Compose stack stays local and uses a synthetic OpenAI-compatible provider. It does not call paid models or send data to Datadog. Only the opt-in Datadog override exports telemetry.

## Run the local test

Requires Docker Compose and [uv](https://docs.astral.sh/uv/).

```sh
cd examples/datadog
docker compose up -d
uv run smoke.py
```

The test verifies HTTP success/500/429 responses, streaming, input/output/cache token accounting, estimated USD cost, time to first token, time per output token, OTLP/HTTP protobuf export, HTTP error status normalization, W3C parent propagation, and omission of prompt/response content. The provider's test prices are **synthetic**, not real provider pricing. Metrics are saved to ignored `var/metrics.txt`.

Traces pass through the pinned OpenTelemetry Collector in `collector.yaml`. In v1.5.0, a provider HTTP 429/500 is recorded in the GenAI span's `http.status` attribute, but the separate OpenTelemetry span status remains `Unset`. The gateway treats a received HTTP response as a successful transport result and sets span status to `Error` only when the upstream call itself returns an error, such as a connection failure. Consequently, an observability backend may receive the correct HTTP status without classifying the GenAI operation as an error.

The Collector marks GenAI spans with `http.status >= 400` as errors and supplies `error.type` when missing. Treating a provider rejection such as 429 as a failed GenAI operation is deliberate. At the HTTP server-span layer, OpenTelemetry normally leaves 4xx status unset because the client may be at fault. This rule is scoped to GenAI operations rather than every gateway HTTP span. Successful spans and existing error types are preserved. Carry this normalization into your production trace pipeline until gateway instrumentation classifies provider responses itself.

Host ports bind only to IPv4 loopback: gateway `127.0.0.1:13000`, metrics `127.0.0.1:18520`, mock capture endpoint `127.0.0.1:18080`. Inside the Compose network, the proxy still uses metrics port 15020. No admin port is published. If a port is occupied, adjust both Compose and the smoke test.

## Send synthetic telemetry to a Datadog trial

1. Create a local `.env` file containing the trial organization's API key and site. Keep this file private (`chmod 600 .env`); it is ignored by Git. Do not paste keys into issues, example YAML, screenshots, or command output.

   ```dotenv
   DD_API_KEY=replace-with-your-trial-api-key
   DD_SITE=us3.datadoghq.com
   ```

   Use the site where your organization actually resides; for US1 use `datadoghq.com`. API and application keys are different: an API key permits ingestion. Dashboard/API queries may additionally need a suitably scoped application key, but the example does not require one.

2. Start the opt-in stack:

   ```sh
   docker compose -f compose.yaml -f compose.datadog.yaml up -d
   uv run smoke.py --live
   docker compose -f compose.yaml -f compose.datadog.yaml exec datadog agent check openmetrics
   ```

   Do not share `docker compose config` or container inspection output: environment variables can contain the API key. This setup intentionally omits Docker socket/host filesystem mounts, container autodiscovery, and log collection; it needs only network access to the synthetic gateway. It is a development setup, not a production Agent deployment.

3. In Datadog, verify `agentgateway.requests.count`, `agentgateway.gen_ai.token.usage.sum`, and `agentgateway.gen_ai.cost.usd.count`, scoped to `env:datadog-dev`. A successful health check alone does not prove counters were collected. Counters need successive scrapes; repeat the smoke test across scrape intervals to populate rate charts.

4. Import `dashboard.json` using Datadog's dashboard JSON import. Enable percentile aggregations on the latency distributions in Metrics Summary for p95 panels. Scope the `env` variable to `datadog-dev`. Controller/MCP/guardrail panels remain empty until corresponding traffic exists.

5. Open LLM Observability and verify gateway LLM spans under `ml_app:agentgateway` or the configured service name. Confirm models, token counts, errors, and trace relationships—not just presence in APM. Allow several minutes for processing. LLM Observability must be enabled for the organization; a successful OTLP response alone is not proof of product ingestion.

The `--live` smoke test verifies requests and local metrics; it **does not claim** to verify Datadog ingestion. The development override sends metadata-only traces through the Collector to the Agent's OTLP/HTTP receiver. Only synthetic data should be used in this example.

Stop the stack when finished to avoid unnecessary trial usage:

```sh
docker compose -f compose.yaml -f compose.datadog.yaml down
```

To return to local trace capture, stop the Datadog stack and run `docker compose up -d` without the override.

## Production metrics configuration

Use `openmetrics.yaml` with the stock Datadog Agent, changing `gateway:15020` and the tags to match your deployment. The file contains a curated metric list and the same metric names as the proposed `agentgateway` community check.

**Keep `use_latest_spec: true` for the v1.5.0 proxy.** It serves OpenMetrics counter TYPE names with a `text/plain` content type. Datadog's default Prometheus parser can silently omit request, cost, and other counters. The Go controller uses the Prometheus parser (`use_latest_spec: false`).

### Kubernetes

Choose one proxy configuration method; do not apply both proxy files to the same workload:

- Standalone proxy Helm chart: merge `kubernetes/proxy-values.yaml` into its values.
- Controller-provisioned proxy: use `kubernetes/proxy-parameters.yaml` and reference it from the Gateway's `spec.infrastructure.parametersRef`. Adapt the namespace and merge with existing parameters. For DaemonSet proxies, put the same pod-template overlay under `spec.daemonSet`.

When running the controller, additionally merge `kubernetes/controller-values.yaml` into its Helm values to collect controller metrics on port 9092. This configuration applies to the controller pod and is independent of the proxy method selected above.

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

Built-in series identity labels are preserved. Per-resource MCP counters are deliberately omitted because their resource label can contain unbounded or sensitive URIs; HTTP MCP traffic remains available. Avoid request IDs, user IDs, prompts, and arbitrary URLs as metric labels. If you customize gateway metric dimensions, include all distinguishing bounded labels or aggregate upstream before scraping. Dropping distinguishing labels can merge independent counters incorrectly. The generic OpenMetrics check counts these as custom metrics; budget for tags and distributions and retain its sample limit.

## Optional prompt/completion capture

By default no prompts or completions leave the gateway. The explicit synthetic opt-in is runnable:

```sh
docker compose -f compose.yaml -f compose.content.yaml up -d
uv run smoke.py --capture-content
```

To test synthetic content in the trial account, also include `-f compose.datadog.yaml` and use `smoke.py --live`. Restore metadata-only mode with `docker compose up -d` (or the Datadog override without the content override).

The opt-in adds the following to `config.tracing.fields.add`:

```yaml
gen_ai.input.messages: 'llm.prompt'
gen_ai.output.messages: 'llm.completion.map(c, {"role":"assistant", "content":c})'
```

The values must arrive as valid JSON message arrays in Datadog. Use synthetic traffic first. Before production, apply content redaction and size limits, choose sampling, and review access controls in both LLM Observability and APM. Capturing content adds memory/serialization overhead. This opt-in is not a redaction mechanism. Gateway observability covers only operations crossing the proxy; it does not replace application instrumentation or configure quality evaluations automatically.

## Migrate to the named integration

The proposed `DataDog/integrations-extras/agentgateway` check packages the mappings and dashboard. It is not bundled with the Agent or published yet. Install its versioned wheel in a custom Agent image, configure the check name `agentgateway` with `component: proxy` or `controller`, and remove the corresponding generic `openmetrics` instance. Never run both against the same endpoint. Metric names remain unchanged; first counter samples after migration establish a new baseline.

The default metrics dashboard works with either check. The scrape-health service check is `agentgateway.openmetrics.health` for both configurations because both use the `agentgateway` namespace.

## Validation status

The local synthetic stack and the real Agent check can be tested without an account. Account ingestion, dashboard rendering, LLM Observability conversion, live Kubernetes deployment, multi-replica behavior, and overhead/load testing require separate acceptance checks. Do not equate local test success with completion of those checks.

See [VALIDATION.md](VALIDATION.md) for recorded results, reproduction commands, and remaining acceptance work.

References: [OpenMetrics](https://docs.datadoghq.com/integrations/openmetrics/), [Kubernetes Autodiscovery](https://docs.datadoghq.com/containers/kubernetes/integrations/), [LLM Observability OTLP](https://docs.datadoghq.com/llm_observability/instrumentation/otel_instrumentation/), [community installation](https://docs.datadoghq.com/agent/guide/use-community-integrations/).
