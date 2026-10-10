# Datadog observability

These examples collect agentgateway metrics with the Datadog OpenMetrics
integration and export OpenTelemetry traces to Datadog Agent Observability.
They use agentgateway v1.5.0, Datadog Agent 7.82.3, and a deterministic
OpenAI-compatible fixture so the default workflows do not call paid models.

Choose the guide that matches how you run agentgateway:

- [Standalone](standalone/README.md) runs agentgateway, the fixture, an
  OpenTelemetry Collector, and the Datadog Agent with Docker Compose.
- [Kubernetes](kubernetes/README.md) installs the agentgateway controller and a
  controller-provisioned proxy, deploys the fixture and Collector, and uses
  Datadog Kubernetes Autodiscovery.

Both guides use the shared [`smoke.py`](smoke.py) test and
[`dashboard.json`](dashboard.json). The fixture implementation and container
image are under [`fixture`](fixture/).

The examples send metadata-only traces by default. Prompt and completion
capture is an explicit standalone option for synthetic data. Review sampling,
redaction, access controls, metric cardinality, and custom-metric volume before
adapting either example for production.
