# OpenShell with agentgateway

This example uses agentgateway as the Kubernetes Gateway API implementation
for an [OpenShell](https://github.com/NVIDIA/OpenShell) deployment. The
OpenShell Helm release creates a dedicated `Gateway` and `GRPCRoute`; setting
`grpcRoute.gateway.className=agentgateway` selects agentgateway to provision
the proxy.

```text
OpenShell CLI
    |
    | gRPC over plaintext HTTP/2
    v
agentgateway proxy
    |
    | gRPC over plaintext HTTP/2
    v
OpenShell gateway --> Agent Sandbox workload
```

The explicit Gateway name, `openshell-ingress`, prevents the proxy Service
created by agentgateway from colliding with the OpenShell Service named
`openshell`.

This configuration disables authentication and TLS for a local development
environment. Use OpenShell authentication and TLS guidance for a production
deployment.

## Versions

`versions.env` pins the OpenShell chart, image, and tested CLI version. The chart
is an official immutable development build containing
[OpenShell PR #4345](https://github.com/NVIDIA/OpenShell/pull/4345).

TODO: Replace the OpenShell development chart with the first stable OpenShell
release that contains PR #4345.

## Prerequisites

- An existing Kubernetes cluster whose CNI enforces ingress and egress
  `NetworkPolicy`. For local testing, see the [Kind quick start](https://kind.sigs.k8s.io/docs/user/quick-start/).
  Kind's default CNI does not enforce NetworkPolicy; use a CNI that does.
- [agentgateway installed](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/)
  with GatewayClass `agentgateway` and Gateway API CRDs, including `GRPCRoute`.
  This example was tested with agentgateway v1.6.0.
- [Agent Sandbox installed](https://github.com/kubernetes-sigs/agent-sandbox#installation)
  with its CRDs and controller. This example was tested with Agent Sandbox v1.0.6.
- `kubectl`, Helm, and a compatible OpenShell CLI. The tested CLI version is
  pinned in `versions.env`.
- Available loopback port `18080` for the temporary port forward.
- Network access to pull images, manifests, and Helm charts.

Run commands from this directory:

```bash
cd examples/openshell
set -a
source versions.env
set +a
```

## Configure OpenShell

Install the immutable OpenShell development chart with the example values:

```bash
helm upgrade --install openshell \
  oci://ghcr.io/nvidia/openshell/helm-chart \
  --version "${OPENSHELL_CHART_VERSION}" \
  --create-namespace \
  --namespace openshell \
  --values openshell-values.yaml \
  --set-string global.image.tag="${OPENSHELL_COMMIT}" \
  --wait \
  --timeout 10m
```

The explicit global image tag pins the gateway, supervisor, and sandbox runtime
images to the same OpenShell commit as the chart.

`openshell-values.yaml` gives the Gateway a name distinct from the backend
Service and overrides the chart's GatewayClass:

```yaml
grpcRoute:
  enabled: true
  gateway:
    create: true
    className: agentgateway
    name: openshell-ingress
```

## Verify the integration

Wait for the OpenShell Gateway API resources and workloads:

```bash
kubectl wait --for=condition=Programmed gateway/openshell-ingress \
  -n openshell --timeout=5m
kubectl wait \
  --for=jsonpath='{.status.parents[0].conditions[?(@.type=="Accepted")].status}'=True \
  grpcroute/openshell -n openshell --timeout=5m
kubectl wait \
  --for=jsonpath='{.status.parents[0].conditions[?(@.type=="ResolvedRefs")].status}'=True \
  grpcroute/openshell -n openshell --timeout=5m
kubectl rollout status deployment/openshell-ingress \
  -n openshell --timeout=5m
kubectl rollout status statefulset/openshell \
  -n openshell --timeout=5m
```

In one terminal, forward the agentgateway proxy Service:

```bash
kubectl port-forward -n openshell service/openshell-ingress 18080:80
```

In a second terminal, use a temporary configuration directory so the test does
not change existing OpenShell gateway registrations:

```bash
export XDG_CONFIG_HOME="$(mktemp -d)"
openshell gateway add http://127.0.0.1:18080 \
  --local --name agentgateway-example
openshell status

openshell sandbox create \
  --name agw-example --detach -- sleep 3600
openshell sandbox exec agw-example \
  --no-login-shell -- sh -c 'printf agentgateway-openshell'
openshell sandbox delete agw-example
```

The exec command prints `agentgateway-openshell`. Stop the port-forward with
Control-C when testing is complete.

## Clean up

Remove the OpenShell release and dedicated namespace only if this example
created them. For an existing OpenShell deployment, restore its previous values:

```bash
helm uninstall openshell --namespace openshell --ignore-not-found
kubectl delete namespace openshell --ignore-not-found
```
