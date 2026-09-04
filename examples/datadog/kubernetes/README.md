# Datadog observability on Kubernetes

This example runs agentgateway v1.5.0 on Kubernetes and sends proxy and
controller metrics and LLM traces to Datadog. The tested path uses the
agentgateway controller, a controller-provisioned proxy, the Datadog Agent
DaemonSet, an OpenTelemetry Collector, and a deterministic synthetic provider.
It does not call a paid model.

The Collector duplicates traces to the local fixture for deterministic smoke
assertions and to the Datadog Agent for product verification. The
`transform/gateway_errors` processor is a v1.5.0 compatibility workaround.
Remove it after upgrading to a release that contains
[PR #3261](https://github.com/agentgateway/agentgateway/pull/3261).

## Prerequisites

- A Kubernetes cluster. The commands below can create a single-node
  [Kind](https://kind.sigs.k8s.io/) cluster, but Kind is not required.
- `kubectl`, Helm, Docker, `curl`, and
  [uv](https://docs.astral.sh/uv/).
- [Kind](https://kind.sigs.k8s.io/) when following the local cluster steps.
- Available loopback ports `13000`, `18080`, `18520`, and `19092` for the
  smoke test's temporary port forwards.
- A Datadog organization, API key, and the correct
  [Datadog site](https://docs.datadoghq.com/getting_started/site/).
- Agent Observability enabled for the Datadog organization.
- Network access to pull container images and Helm charts.

Run all commands from this directory:

```sh
cd examples/datadog/kubernetes
```

Export the Datadog credentials without writing them to a manifest:

```sh
export DD_API_KEY="replace-with-your-datadog-api-key"
export DD_SITE="us3.datadoghq.com"
```

## Create a Kind cluster

Skip this section when using an existing cluster. The default Kind
configuration is sufficient; no Kind configuration file is required.

```sh
kind create cluster --name agentgateway-datadog

docker build \
  -t agentgateway-datadog-fixture:local \
  ../fixture
kind load docker-image \
  agentgateway-datadog-fixture:local \
  --name agentgateway-datadog
```

For a non-Kind cluster, publish the fixture image to a registry accessible to
the cluster, then update its image and pull policy in `fixture.yaml`.

## Install the Datadog Agent

Add the Datadog chart repository and create the API key Secret. The Secret must
contain a key named `api-key`, as expected by `datadog-values.yaml`.

```sh
helm repo add datadog https://helm.datadoghq.com
helm repo update datadog

kubectl create namespace datadog
kubectl create secret generic datadog-api-key \
  --namespace datadog \
  --from-literal api-key="${DD_API_KEY}"
```

Install the pinned chart and Agent image. The values enable the Agent's
OTLP/HTTP receiver and disable products that this example does not exercise.
They also disable kubelet TLS verification for Kind's certificate. Restore
`datadog.kubelet.tlsVerify: true` when the Agent trusts your cluster's kubelet
certificate. Override `datadog.site` with the organization site.

```sh
helm upgrade --install datadog datadog/datadog \
  --namespace datadog \
  --version 3.241.0 \
  --values datadog-values.yaml \
  --set-string datadog.site="${DD_SITE}" \
  --wait \
  --timeout 10m

kubectl rollout status daemonset/datadog \
  --namespace datadog \
  --timeout 5m
```

The chart creates a `datadog` Service with `internalTrafficPolicy: Local` for
the OTLP receiver, so Collector traffic reaches an Agent on the same node. The
Datadog Agent must run on every node that can schedule the Collector in a
multi-node adaptation of this example.

## Install agentgateway

Install Gateway API v1.6.0 and the agentgateway v1.5.0 CRDs and controller.
`controller-values.yaml` adds the controller OpenMetrics Autodiscovery check.

```sh
kubectl apply --server-side --force-conflicts \
  -f https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.6.0/standard-install.yaml

helm upgrade --install agentgateway-crds \
  oci://cr.agentgateway.dev/charts/agentgateway-crds \
  --create-namespace \
  --namespace agentgateway-system \
  --version v1.5.0

helm upgrade --install agentgateway \
  oci://cr.agentgateway.dev/charts/agentgateway \
  --namespace agentgateway-system \
  --version v1.5.0 \
  --values controller-values.yaml \
  --wait \
  --timeout 5m
```

Deploy the fixture and Collector, then create the proxy configuration. The
`AgentgatewayParameters` resource adds the proxy OpenMetrics Autodiscovery
check and loads synthetic model rates from the catalog in `gateway.yaml`.

```sh
kubectl apply -f fixture.yaml
kubectl apply -f collector.yaml
kubectl apply -f proxy-parameters.yaml
kubectl apply -f gateway.yaml

kubectl rollout status deployment/datadog-fixture \
  --namespace agentgateway-system \
  --timeout 5m
kubectl rollout status deployment/datadog-collector \
  --namespace agentgateway-system \
  --timeout 5m
kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  --namespace agentgateway-system \
  --timeout 5m
kubectl rollout status deployment/agentgateway-proxy \
  --namespace agentgateway-system \
  --timeout 5m
```

This is the tested controller-provisioned proxy path. Do not also apply
`proxy-values.yaml` to this proxy. That file is an alternative annotation for
users deploying the standalone proxy Helm chart.

## Run the smoke test

Run the smoke script, which creates temporary local port forwards and invokes
the shared Python test:

```sh
./smoke.sh
```

The test verifies:

- Successful, streaming, HTTP 429, and HTTP 500 provider responses.
- Proxy and controller metrics endpoints.
- Input, output, and cached-input token accounting.
- Synthetic estimated cost, time to first token, and time per output token.
- OTLP/HTTP export, W3C parent propagation, and HTTP error classification.
- Omission of prompt and completion content from exported traces.

## Verify Datadog collection

Find the node Agent pod and run successive OpenMetrics checks. Counter metrics
need at least two scrapes before rate samples appear.

```sh
export DD_AGENT_POD="$(kubectl get pods \
  --namespace datadog \
  --selector app=datadog \
  --output jsonpath='{.items[0].metadata.name}')"

kubectl exec --namespace datadog "${DD_AGENT_POD}" -- \
  agent check openmetrics --check-rate
```

The output should include two healthy instances with `component:proxy` and
`component:controller` tags. It should report metric samples for each endpoint.

In Datadog:

1. Open **Metrics > Explorer** and search for
   `agentgateway.requests.count` and
   `agentgateway.controller.reconciliations.count`.
2. Filter by `env:datadog-dev` and `service:agentgateway`, then use
   `component:proxy` or `component:controller`.
3. Import [`../dashboard.json`](../dashboard.json) in **Dashboards**. Enable
   percentile aggregations for the latency distributions before using the p95
   widgets.
4. Open **Agent Observability > Traces** and search for
   `ml_app:agentgateway`. Allow several minutes for processing.

The synthetic `datadog-test` model is not in Datadog's pricing catalog, so
Agent Observability displays `COST UNAVAILABLE`. The fixture calculation is
still present in the `agw.ai.usage.cost.*` span attributes and
`agentgateway.gen_ai.cost.usd.count` metric.

All OpenMetrics series are Datadog custom metrics. This example collects proxy
and controller runtime families with a wildcard. Review custom-metric usage and
label cardinality before using the configuration in production.

The proxy configuration maps the dedicated MCP operation counter to
`agentgateway.mcp.requests.count` and preserves its `resource`, `resource_type`,
and `server` tags. The `resource` value can contain tool names or resource URIs.
If those values or their custom-metric cardinality are unsuitable for your
environment, add `"exclude_metrics": ["mcp_requests"]` to the OpenMetrics
instance in `proxy-parameters.yaml` or `proxy-values.yaml`.

## Troubleshooting

### An OpenMetrics check is missing

Confirm that the annotation identifier matches the container name:
`agentgateway` for the proxy and `controller` for the controller.

```sh
kubectl get pod --namespace agentgateway-system --show-labels
kubectl get pod --namespace agentgateway-system \
  --output jsonpath='{range .items[*]}{.metadata.name}{"\n"}{.metadata.annotations}{"\n\n"}{end}'
```

Do not apply both `proxy-parameters.yaml` and `proxy-values.yaml` to one
workload, and do not scrape the same endpoint through both Autodiscovery and a
separate Prometheus discovery configuration.

### The Agent is unhealthy or telemetry is absent

Verify the API key and site, then inspect the Agent without printing the Secret:

```sh
kubectl get pods --namespace datadog
kubectl logs --namespace datadog "${DD_AGENT_POD}" --container agent --tail 200
kubectl exec --namespace datadog "${DD_AGENT_POD}" -- agent status
```

A healthy OpenMetrics service check proves endpoint reachability; it does not
prove that counter samples reached Datadog. Run `./smoke.sh` between scrape
intervals and repeat the check with `--check-rate`.

### Raw metrics are unavailable

Port-forward the proxy or controller endpoint directly:

```sh
kubectl port-forward --namespace agentgateway-system \
  deployment/agentgateway-proxy 18520:15020
curl --fail http://127.0.0.1:18520/metrics
```

For controller metrics, forward `service/agentgateway 19092:9092` instead.
Management ports should remain private outside troubleshooting.

### Traces do not appear

Check the Collector and Agent, then confirm the tracing policy is attached:

```sh
kubectl logs --namespace agentgateway-system \
  deployment/datadog-collector --tail 200
kubectl get agentgatewaypolicy datadog-tracing \
  --namespace agentgateway-system
```

Agent Observability must be enabled for the organization. A successful OTLP
response alone does not prove product ingestion.

## Clean up

For the dedicated Kind cluster, deleting the cluster removes all resources:

```sh
kind delete cluster --name agentgateway-datadog
```

For an existing cluster, remove the example resources and releases:

```sh
kubectl delete -f gateway.yaml --ignore-not-found
kubectl delete -f proxy-parameters.yaml --ignore-not-found
kubectl delete -f collector.yaml --ignore-not-found
kubectl delete -f fixture.yaml --ignore-not-found
helm uninstall agentgateway --namespace agentgateway-system
helm uninstall agentgateway-crds --namespace agentgateway-system
helm uninstall datadog --namespace datadog
kubectl delete namespace agentgateway-system datadog --ignore-not-found
```

## References

- [Datadog OpenMetrics](https://docs.datadoghq.com/integrations/openmetrics/)
- [Kubernetes Autodiscovery](https://docs.datadoghq.com/containers/kubernetes/integrations/)
- [OTLP ingestion by the Datadog Agent](https://docs.datadoghq.com/opentelemetry/setup/otlp_ingest_in_the_agent/)
- [Agent Observability with OpenTelemetry](https://docs.datadoghq.com/llm_observability/instrument/otel_instrumentation/)
