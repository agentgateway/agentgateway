# Datadog observability on Kubernetes

Configure an existing agentgateway controller and proxy to export metrics and
traces to Datadog. Autodiscovery collects proxy and controller metrics; the
proxy sends OTLP/HTTP traces to the node-local Datadog Agent. See the
[overview](../README.md) for manual verification and the dashboard.

## Prerequisites

- An existing Kubernetes cluster. For local testing, see the
  [Kind quick start](https://kind.sigs.k8s.io/docs/user/quick-start/).
- [agentgateway installed](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/),
  including its CRDs, controller, and GatewayClass `agentgateway`.
  This example was tested with agentgateway v1.6.0.
- kubectl, Helm, curl, a Datadog ingestion API key and site, and Agent
  Observability enabled to inspect LLM traces.

The files assume namespace `agentgateway-system`, Deployment `agentgateway`,
and controller container `controller`. Adjust these for your installation.
Select your kubectl context and set `DD_API_KEY` and `DD_SITE`. Run commands in
the same shell:

```sh
cd examples/datadog/kubernetes
source versions.env
```

[`versions.env`](versions.env) pins the Datadog chart and Agent. HomeHub's image
is pinned in `homehub.yaml`.

## Install Datadog

```sh
helm repo add datadog https://helm.datadoghq.com
helm repo update datadog
kubectl create namespace datadog
kubectl create secret generic datadog-api-key --namespace datadog \
  --from-literal api-key="${DD_API_KEY}"

helm upgrade --install datadog datadog/datadog \
  --namespace datadog \
  --version "${DATADOG_CHART_VERSION}" \
  --values datadog-values.yaml \
  --set-string datadog.site="${DD_SITE}" \
  --set-string agents.image.tag="${DATADOG_AGENT_VERSION}" \
  --wait --timeout 10m
```

The values disable unrelated Datadog products. For local Kind testing, set
`datadog.kubelet.tlsVerify=false` if required for its kubelet certificate.

The chart's OTLP Service uses `internalTrafficPolicy: Local`. Run an Agent on
every node hosting an agentgateway proxy so traces reach an Agent on the same
node.

## Configure agentgateway and HomeHub

Apply the controller's OpenMetrics annotation to its existing Deployment:

```sh
kubectl patch deployment agentgateway --namespace agentgateway-system \
  --type merge --patch-file controller-annotations.yaml
```

If that container already has Datadog checks, merge the OpenMetrics instance
into its existing annotation instead. Helm upgrades can replace this annotation;
retain it in your installation's values when adapting the example.

Apply the example Gateway and telemetry configuration. If reusing an existing
Gateway, adjust references and omit `gateway.yaml`'s Gateway definition:

```sh
kubectl apply -f homehub.yaml -f proxy-parameters.yaml -f gateway.yaml
kubectl rollout status deployment/homehub-support \
  --namespace agentgateway-system --timeout=5m
kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  --namespace agentgateway-system --timeout=5m
kubectl rollout status deployment/agentgateway-proxy \
  --namespace agentgateway-system --timeout=5m
```

`controller-annotations.yaml` enables controller metric collection.
`proxy-parameters.yaml` attaches the proxy OpenMetrics check to the
controller-provisioned Deployment.

## Verify

In one terminal, forward the gateway:

```sh
kubectl port-forward --namespace agentgateway-system \
  service/agentgateway-proxy 13000:80
```

In a second terminal, forward proxy metrics:

```sh
kubectl port-forward --namespace agentgateway-system \
  deployment/agentgateway-proxy 18520:15020
```

In another terminal, follow the [manual verification](../README.md#verify-collection):

```sh
export GATEWAY_URL=http://127.0.0.1:13000
export METRICS_URL=http://127.0.0.1:18520/metrics
```

Find the node Agent and run successive OpenMetrics checks:

```sh
export DD_AGENT_POD="$(kubectl get pods --namespace datadog \
  --selector app=datadog --output jsonpath='{.items[0].metadata.name}')"
kubectl exec --namespace datadog "${DD_AGENT_POD}" -- \
  agent check openmetrics --check-rate \
  --discovery-min-instances 2 --discovery-timeout 30
```

The output should include instances tagged `component:proxy` and
`component:controller` when both run on the same node. On multiple nodes, run
the check on the Agent hosting each component, using `--discovery-min-instances 1`. Confirm metrics and LLM traces in Datadog,
then stop the port-forwards with Control-C.

## Troubleshoot

```sh
kubectl get pods --namespace datadog
kubectl exec --namespace datadog "${DD_AGENT_POD}" -- agent status
kubectl describe deployment/agentgateway-proxy --namespace agentgateway-system
kubectl get agentgatewaypolicy datadog-tracing --namespace agentgateway-system -o yaml
```

If metrics are missing, inspect the OpenMetrics check's sample output and
Autodiscovery annotations. If traces are missing, check the API key, site,
Agent health, and OTLP receiver. The Datadog Agent must be on the proxy's node.

## Cleanup

Remove the integration resources and restore any Gateway settings you changed.
Restore the controller's previous Datadog annotation if you merged with existing
checks; otherwise remove the example annotation:

```sh
kubectl delete agentgatewaypolicy datadog-tracing --namespace agentgateway-system
kubectl delete httproute homehub-support --namespace agentgateway-system
kubectl delete agentgatewaybackend homehub-support datadog-otlp --namespace agentgateway-system
kubectl delete -f proxy-parameters.yaml -f homehub.yaml
kubectl patch deployment agentgateway --namespace agentgateway-system \
  --type merge -p '{"spec":{"template":{"metadata":{"annotations":{"ad.datadoghq.com/controller.checks":null}}}}}'
helm uninstall datadog --namespace datadog
kubectl delete secret datadog-api-key --namespace datadog
```

Only uninstall Datadog and delete its secret if this example created them.
Delete `gateway/agentgateway-proxy` separately only if you created it for this
example; keep any existing Gateway.
