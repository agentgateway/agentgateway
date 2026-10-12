# Datadog observability on Kubernetes

Run the agentgateway controller, its proxy, HomeHub, and a Datadog Agent on
each node. Autodiscovery collects proxy and controller metrics. The proxy sends
OTLP/HTTP traces directly to the node-local Datadog Agent. See the
[overview](../README.md) for the manual verification and dashboard.

## Before you begin

Install Docker, kind 0.29.0 or later, kubectl, Helm, and curl. You need a Datadog
organization, its ingestion API key and site, and Agent Observability enabled
to inspect LLM traces. Set `DD_API_KEY` and `DD_SITE` in your shell.

Run the following commands in the same shell from this directory:

```sh
cd examples/datadog/kubernetes
source versions.env
kind create cluster --name agentgateway-datadog --image "${KIND_NODE_IMAGE}"
```

[`versions.env`](versions.env) pins the versions used by the installation
commands. Skip cluster creation when using an existing cluster and select its
kubectl context. HomeHub's image is pinned in `homehub.yaml`.

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

The values disable unrelated products and kubelet TLS verification for kind's
certificate. Set `datadog.kubelet.tlsVerify=true` when the Agent trusts your
cluster's kubelet certificate.

The chart's OTLP Service uses `internalTrafficPolicy: Local`. Run an Agent on
every node hosting an agentgateway proxy so traces reach an Agent on the same
node.

## Install agentgateway and HomeHub

```sh
kubectl apply --server-side --force-conflicts \
  -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml"

helm upgrade --install agentgateway-crds \
  oci://cr.agentgateway.dev/charts/agentgateway-crds \
  --create-namespace --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}"
helm upgrade --install agentgateway \
  oci://cr.agentgateway.dev/charts/agentgateway \
  --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}" \
  --values controller-values.yaml --wait --timeout 5m

kubectl apply -f homehub.yaml -f proxy-parameters.yaml -f gateway.yaml
kubectl rollout status deployment/homehub-support \
  --namespace agentgateway-system --timeout=5m
kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  --namespace agentgateway-system --timeout=5m
kubectl rollout status deployment/agentgateway-proxy \
  --namespace agentgateway-system --timeout=5m
```

`controller-values.yaml` enables controller metric collection.
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

On the single-node kind cluster, the output should include instances tagged
`component:proxy` and `component:controller`. On multiple nodes, run the check
on the Agent hosting each component. Confirm metrics and LLM traces in Datadog,
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

Delete the disposable cluster:

```sh
kind delete cluster --name agentgateway-datadog
```

On an existing cluster, remove this example's resources:

```sh
kubectl delete -f gateway.yaml -f proxy-parameters.yaml -f homehub.yaml
helm uninstall agentgateway agentgateway-crds --namespace agentgateway-system
helm uninstall datadog --namespace datadog
kubectl delete secret datadog-api-key --namespace datadog
```
