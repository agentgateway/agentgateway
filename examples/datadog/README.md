# Datadog observability

Collect agentgateway metrics with Datadog OpenMetrics and send OpenTelemetry
traces directly to the Datadog Agent. Choose a deployment:

- [Standalone](standalone/README.md): Docker Compose.
- [Kubernetes](kubernetes/README.md): controller-provisioned proxy and
  OpenMetrics Autodiscovery for both proxy and controller.

Both use agentgateway v1.6.0 and the public
[HomeHub demo](https://github.com/solo-io/homehub-support-demo) image, pinned to
`sha-fd19c81`. HomeHub returns fixed OpenAI-compatible responses without an LLM
API key. A Datadog organization, API key, and site are required. Enable Agent
Observability to inspect LLM traces. Traces omit prompt and completion content.

## Verify collection

After starting either deployment, set `GATEWAY_URL` and `METRICS_URL` as shown in
its guide. Send a request:

```sh
curl --fail "${GATEWAY_URL}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -d '{"model":"homehub-support","stream":false,"messages":[{"role":"user","content":"How do I factory-reset my HomeHub X2?"}],"max_tokens":96}'
```

The response describes the HomeHub X2 reset procedure and reports 12 input and
24 output tokens. Inspect the proxy's raw request and token metrics:

```sh
curl --fail "${METRICS_URL}" \
  | grep -E '^agentgateway_(requests|gen_ai_client_token_usage)'
```

Run the Datadog Agent's OpenMetrics check using the command in your deployment
guide. Counters need successive scrapes before rates appear; send another
request between scrape intervals.

In Datadog:

1. Open **Metrics > Explorer**, filter by `env:datadog-dev`, and find
   `agentgateway.requests.count` and `agentgateway.gen_ai.token.usage.sum`.
   Kubernetes also collects `agentgateway.controller.reconciliations.count`.
2. Open **Agent Observability > Traces**, search for `ml_app:agentgateway`, and
   confirm the HomeHub model and token counts. Allow several minutes for
   processing. A successful OTLP response alone does not prove ingestion.
3. Optionally import [dashboard.json](dashboard.json) and select
   `env:datadog-dev`. Enable percentile aggregations for latency distributions
   before using p95 widgets.

HomeHub has no real provider pricing, so estimated-cost panels can be empty or
show `COST UNAVAILABLE`. Streaming timing, MCP, and guardrail panels require
corresponding traffic. The demo uses non-streaming responses.

To try real-provider telemetry, adapt the backend using the
[LLM basic example](../llm-basic/README.md). Real requests require provider
credentials and can incur charges. agentgateway v1.6.0 includes pricing for
common public models; Datadog uses its own pricing catalog.

## Metric configuration

[Standalone OpenMetrics](standalone/openmetrics.yaml) and the Kubernetes
[proxy parameters](kubernetes/proxy-parameters.yaml) retain the mappings used by
the dashboard. Proxy collection uses `use_latest_spec: true`; the controller
uses `false`. The parsers differ, and using the wrong one can omit counters.

Wildcard collection includes runtime metrics. Review custom-metric volume and
label cardinality when adapting the configuration. MCP metrics can include tool
names and resource URIs; exclude `mcp_requests` if those labels are unsuitable.

## References

- [Datadog OpenMetrics](https://docs.datadoghq.com/integrations/openmetrics/)
- [OpenTelemetry ingestion by the Datadog Agent](https://docs.datadoghq.com/opentelemetry/setup/otlp_ingest_in_the_agent/)
- [Agent Observability with OpenTelemetry](https://docs.datadoghq.com/llm_observability/instrument/otel_instrumentation/)
