# Datadog standalone observability

This example targets **agentgateway v1.5.0** and **Datadog Agent 7.82.3**. It
collects metrics with Datadog's stock OpenMetrics check and sends traces through
OpenTelemetry to Datadog Agent Observability. No Datadog SDK is required inside
agentgateway.

## Prerequisites

The local test requires:

- [Docker](https://docs.docker.com/get-docker/) with
  [Docker Compose](https://docs.docker.com/compose/install/), installed through
  Docker Desktop or the Compose CLI plugin.
- [uv](https://docs.astral.sh/uv/) to create the Python environment and run the
  smoke test.
- `curl` to inspect raw metrics and send the optional real-provider request.
- Network access on the first run to download container images and Python
  packages.
- Available loopback ports `13000`, `18080`, and `18520`.

Exporting synthetic telemetry requires a Datadog organization, an API key, and
the correct [Datadog site](https://docs.datadoghq.com/getting_started/site/).
Agent Observability, which Datadog documentation also calls LLM Observability,
must be enabled for the organization to verify traces in that product. Local
validation does not require a Datadog account.

The optional real-provider test requires an OpenAI API key with billing enabled
and an OpenAI model that supports the Chat Completions API. It makes a paid API
request.

All commands below run from the example directory unless stated otherwise:

```sh
cd examples/datadog/standalone
```

## Run the local test

Start the default configuration and run the smoke test:

```sh
docker compose up -d
uv run ../smoke.py
```

The smoke test verifies:

- Successful provider responses and HTTP 429/500 errors.
- Streaming and non-streaming OpenAI-compatible responses.
- Input, output, and cached-input token accounting.
- Synthetic estimated cost, time to first token, and time per output token.
- OTLP/HTTP protobuf export, W3C parent propagation, and HTTP error
  classification.
- Omission of prompt and completion content from exported traces.

The fixture prices are synthetic and do not represent provider pricing. The
test saves the raw metric payload to the ignored `var/metrics.txt` file.

Inspect the live Prometheus payload directly:

```sh
curl http://127.0.0.1:18520/metrics
```

| Endpoint | Host address | Purpose |
| --- | --- | --- |
| Gateway | `127.0.0.1:13000` | OpenAI-compatible requests |
| Metrics | `127.0.0.1:18520` | Raw Prometheus metrics |
| Fixture | `127.0.0.1:18080` | Health and captured-trace endpoints |

All ports bind only to IPv4 loopback. Inside the Compose network, the gateway
still exposes metrics on port 15020. No admin port is published. If a port is
occupied, update both the Compose configuration and the smoke test.

Leave the stack running to continue with synthetic Datadog export, or stop it:

```sh
docker compose down
```

## Send synthetic telemetry to Datadog

1. Create an ignored `.env` file with the API key and site for the Datadog
   organization:

   ```dotenv
   DD_API_KEY=replace-with-your-datadog-api-key
   DD_SITE=us3.datadoghq.com
   ```

   Keep this file private with `chmod 600 .env`. Use the site where the
   organization resides; for US1 use `datadoghq.com`. API and application keys
   are different: an API key permits ingestion. API queries may additionally
   require a suitably scoped application key, but this example does not.

2. Start the synthetic configuration with the Datadog override, generate
   traffic, and run two OpenMetrics scrapes:

   ```sh
   docker compose -f compose.yaml -f compose.datadog.yaml up -d
   uv run ../smoke.py --datadog
   docker compose -f compose.yaml -f compose.datadog.yaml \
     exec datadog agent check openmetrics --check-rate
   ```

   The first counter scrape establishes a baseline. `--check-rate` performs
   successive checks so counter metrics can appear. Repeat `uv run ../smoke.py
   --datadog` across scrape intervals when populating rate charts.

3. In Datadog, open **Metrics > Explorer** and search for these exact metrics:

   - `agentgateway.requests.count`
   - `agentgateway.gen_ai.token.usage.sum`
   - `agentgateway.gen_ai.cost.usd.count`

   Filter them by `env:datadog-dev`. A successful
   `agentgateway.openmetrics.health` service check proves that the endpoint was
   reachable; it does not prove that counter samples reached Datadog.

4. In **Dashboards**, import [`dashboard.json`](../dashboard.json) with the
   dashboard JSON import action. Set the `env` template variable to
   `datadog-dev`. Enable percentile
   aggregations for the latency distributions in Metrics Summary before using
   the p95 widgets. Controller, MCP, and guardrail widgets remain empty until
   their corresponding components or traffic are present. To see an exact
   metric name used by a widget, edit the widget and inspect its query.

5. Open **Agent Observability > Traces** and search for
   `ml_app:agentgateway`. Verify models, token counts, errors, and trace
   relationships. Allow several minutes for processing. A successful OTLP
   response or a trace in APM alone does not prove ingestion into Agent
   Observability.

   Agent Observability displays `COST UNAVAILABLE` for the synthetic
   `datadog-test` model because that model is not in Datadog's pricing catalog.
   This is expected. The cost calculated from the fixture rates remains visible
   in the `agw.ai.usage.cost.*` span tags and the
   `agentgateway.gen_ai.cost.usd.count` metric.

The `--datadog` smoke mode validates requests and local metrics, but it does not
assert Datadog ingestion. The override sends metadata-only traces through the
Collector to the Agent's OTLP/HTTP receiver. Use only non-sensitive test prompts
with this development configuration.

Stop the stack when finished to avoid unnecessary trial usage:

```sh
docker compose -f compose.yaml -f compose.datadog.yaml down
```

To return to local trace capture, run `docker compose up -d` without the
Datadog override.

## Troubleshooting

### The Datadog Agent container is unhealthy

An invalid API key or incorrect `DD_SITE` can make the Agent unhealthy even
when its local OpenMetrics check can reach agentgateway. Verify the organization
site and API key, then inspect the Agent:

```sh
docker compose -f compose.yaml -f compose.datadog.yaml ps
docker compose -f compose.yaml -f compose.datadog.yaml \
  exec datadog agent status
```

Do not share `docker compose config`, container inspection output, or the local
`.env` file; those outputs can contain credentials.

### The OpenMetrics check succeeds but metrics are missing in Datadog

Run the check with rate calculation:

```sh
docker compose -f compose.yaml -f compose.datadog.yaml \
  exec datadog agent check openmetrics --check-rate
```

Confirm that the output reports metric samples, then:

1. Run `uv run ../smoke.py --datadog` again between scrape intervals.
2. Verify `DD_API_KEY` and `DD_SITE`.
3. Allow several minutes for intake and indexing.
4. Search in **Metrics > Explorer** using the exact metric name.
5. Remove filters other than `env:datadog-dev` while troubleshooting.

The service check alone is insufficient because it only reports whether the
metrics endpoint responded successfully.

### Raw metrics are unavailable

Check the containers and query the loopback endpoint:

```sh
docker compose ps
curl --fail http://127.0.0.1:18520/metrics
```

The gateway container does not need a Docker health check to serve metrics. If
the port is already in use, change the host-side `18520` mapping and the URL in
`smoke.py` together.

### Traces do not appear in Agent Observability

- Confirm that Agent Observability is enabled for the organization.
- Confirm that the Agent is healthy and that `DD_SITE` selects the correct
  organization.
- Search for `ml_app:agentgateway` and allow several minutes for processing.
- Remember that the default local stack exports traces to the fixture. Include
  `compose.datadog.yaml` to export them to Datadog.

### Agent Observability reports `COST UNAVAILABLE`

This is expected for the synthetic `datadog-test` model. Datadog can estimate
cost only when it recognizes the provider and returned model and receives token
usage. Use the span's `agw.ai.usage.cost.total` tag to inspect agentgateway's
synthetic calculation.

### Dashboard percentile widgets are empty

Enable percentile aggregations for the corresponding distribution metrics in
Metrics Summary, wait for processing, and confirm that more than one scrape has
occurred. Controller panels require the configuration in the
[Kubernetes guide](../kubernetes/README.md).

## Optional prompt and completion capture

By default, traces exported by this example do not contain prompt or completion
content. To validate content capture with synthetic data locally, add the
content override:

```sh
docker compose -f compose.yaml -f compose.content.yaml up -d
uv run ../smoke.py --capture-content
```

To send synthetic captured content to the Datadog trial, include both overrides:

```sh
docker compose -f compose.yaml -f compose.content.yaml \
  -f compose.datadog.yaml up -d
uv run ../smoke.py --datadog
```

Then find the span under `ml_app:agentgateway` and inspect its Messages section.
The `--datadog` mode sends traffic but does not assert product ingestion.

Restore metadata-only local mode with `docker compose up -d`. For Datadog
export, omit `compose.content.yaml` when starting the stack again.

The content override adds these expressions to `config.tracing.fields.add`:

```yaml
gen_ai.input.messages: 'llm.prompt'
gen_ai.output.messages: 'llm.completion.map(c, {"role":"assistant", "content":c})'
```

The values must arrive as valid JSON message arrays. Use synthetic traffic
first. Before production, apply content redaction and size limits, choose a
sampling policy, and review access controls in both Agent Observability and APM.
Capturing content adds memory and serialization overhead. Enabling capture is
not a redaction mechanism. Gateway observability covers only operations crossing
the proxy; it does not replace application instrumentation or configure quality
evaluations.

## Send a request to a real OpenAI model

The synthetic provider remains the default. To test a real model, add these
values to the ignored `.env` file:

```dotenv
OPENAI_API_KEY=replace-with-your-openai-api-key
OPENAI_MODEL=replace-with-a-chat-completions-model
```

Keep the OpenAI key private. This mode makes a paid API request. Do not combine
it with `compose.content.yaml`; exported traces remain metadata-only.

Start agentgateway with the real-provider configuration and Datadog Agent:

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

Allow several minutes for processing, then find the span in **Agent
Observability > Traces** under `ml_app:agentgateway`. Datadog can estimate cost
when the provider, returned model, and token usage match its pricing catalog.

agentgateway calculates cost only when its own model catalog contains rates for
the returned model. [`config-openai.yaml`](config-openai.yaml) does not include
provider rates because they vary over time and by agreement. To compare the two
estimates, add the exact provider model and its current per-million-token rates
to a local copy of that configuration:

```yaml
config:
  modelCatalog:
    - inline:
        providers:
          openai:
            models:
              replace-with-exact-provider-model:
                rates:
                  input: "replace-with-input-rate"
                  output: "replace-with-output-rate"
                  cacheRead: "replace-with-cached-input-rate"
```

Merge `modelCatalog` into the existing `config` object rather than adding a
second `config` key.

For one request, compare Datadog's **Estimated Cost** with
`agw.ai.usage.cost.total` on the same span. The
`agentgateway.gen_ai.cost.usd.count` metric is cumulative; compare its change
across scrape intervals while no unrelated traffic is running. These values may
differ because Datadog and agentgateway use separate pricing catalogs.

Stop the real-provider stack when finished:

```sh
docker compose -f compose.openai.yaml -f compose.datadog.yaml down
```

## Deploy OpenMetrics collection

### Stock Agent configuration

Use [`openmetrics.yaml`](openmetrics.yaml) with a stock Datadog Agent outside
Docker Compose, changing `gateway:15020` and the tags for the deployment. The
file collects all proxy and runtime metric families. Explicit mappings preserve
the names used by the dashboard. `raw_metric_prefix` removes the source
`agentgateway_` prefix before the wildcard collects the remaining families.

Keep `use_latest_spec: true` for the v1.5.0 proxy. It serves OpenMetrics counter
type names with a `text/plain` content type; Datadog's default Prometheus parser
can silently omit request, cost, and other counters. The Go controller uses the
Prometheus parser with `use_latest_spec: false`.

All OpenMetrics series are Datadog custom metrics. Wildcard collection increases
custom-metric volume, particularly when labels create many contexts. Review
usage and tag cardinality for the deployment and retain the Agent's sample
limit.

### Key mapped metrics

The wildcard also collects runtime and transport metrics. The table below lists
key stable mappings, including the names used by the dashboard. See agentgateway's
[source metric catalog](../../../schema/metrics.md) for the source families.

<!-- markdownlint-disable MD013 -->
| Datadog metric | Meaning |
| --- | --- |
| `agentgateway.requests.count` | HTTP request counter; use `status` and `reason` for errors and `protocol:mcp` for MCP transport traffic |
| `agentgateway.mcp.requests.count` | MCP operation counter, separated by method, resource type, server, and resource |
| `agentgateway.request.duration` | HTTP latency distribution, in seconds |
| `agentgateway.gen_ai.request.duration` | LLM request latency distribution, in seconds |
| `agentgateway.gen_ai.time_to_first_token` | Time-to-first-token distribution, in seconds |
| `agentgateway.gen_ai.time_per_output_token` | Time-per-output-token distribution, in seconds |
| `agentgateway.gen_ai.token.usage.sum` | Token totals, separated by `token_type` |
| `agentgateway.gen_ai.cost.usd.count` | Estimated cumulative USD for requests with known usage and pricing |
| `agentgateway.cost_catalog.lookups.count` | Pricing lookup outcomes, including missing or unpriced models |
| `agentgateway.controller.reconciliations.count` | Optional controller reconciliation counter |
<!-- markdownlint-enable MD013 -->

Histograms also export monotonic `.sum` and `.count` metrics. Token histogram
`.count` counts observations, not tokens. Input tokens already include cached
input, so do not add cache categories to input totals. Cost is incomplete when
usage or pricing is missing and is not an invoice. Error status is not a default
GenAI histogram dimension, so default metrics cannot provide per-model error
rates.

The label allowlist preserves the built-in identities of collected families
while preventing user-configured custom dimensions from being exported
automatically. Avoid request IDs, user IDs, prompts, and arbitrary URLs as
metric labels. If custom dimensions distinguish independent counters, add every
bounded identity label or aggregate before scraping; dropping one can merge
independent series incorrectly.

The `agentgateway.mcp.requests.count` metric preserves the source counter's
`resource`, `resource_type`, and `server` labels. The `resource` value can
contain tool names or resource URIs, so review its possible values and resulting
custom-metric cardinality before production use. To prevent collection of this
metric, add the source name to `exclude_metrics`:

```yaml
exclude_metrics:
  - mcp_requests
```

## agentgateway v1.5.0 error-status compatibility

In v1.5.0, a provider HTTP 429 or 500 is recorded in the GenAI span's
`http.status` attribute, but the separate OpenTelemetry span status remains
`Unset`. The gateway received an HTTP response successfully at the transport
layer even though the GenAI operation failed. This was fixed after v1.5.0 by
[PR #3261](https://github.com/agentgateway/agentgateway/pull/3261).

For v1.5.0, [`collector.yaml`](collector.yaml) applies
`transform/gateway_errors` only to GenAI spans with `http.status >= 400`. It
marks the span as an error and supplies `error.type` when missing. Successful
spans, unrelated HTTP spans, and existing error types are preserved. Remove the
processor from `collector.yaml` after upgrading to a release that contains
[PR #3261](https://github.com/agentgateway/agentgateway/pull/3261); that release
performs the classification in the gateway and records the numeric HTTP status
as `error.type`.

## References

- [Datadog OpenMetrics](https://docs.datadoghq.com/integrations/openmetrics/)
- [Kubernetes Autodiscovery](https://docs.datadoghq.com/containers/kubernetes/integrations/)
- [LLM Observability with OpenTelemetry](https://docs.datadoghq.com/llm_observability/instrumentation/otel_instrumentation/)
- [Agent Observability cost](https://docs.datadoghq.com/llm_observability/investigate/cost/)
- [Datadog community integrations](https://docs.datadoghq.com/agent/guide/use-community-integrations/)
