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

`versions.env` pins the versions used to test this example. The OpenShell chart
is an official immutable development build containing
[OpenShell PR #4345](https://github.com/NVIDIA/OpenShell/pull/4345).

TODO: Replace the OpenShell development chart with the first stable OpenShell
release that contains PR #4345.

## Prerequisites

- A Kubernetes cluster whose CNI enforces both ingress and egress Kubernetes
  `NetworkPolicy`. OpenShell creates policies for sandbox workloads, but the
  Kubernetes API accepts them even when the CNI does not enforce them.
- `kubectl`, Helm, and a compatible OpenShell CLI. The tested CLI version is
  pinned in `versions.env`.
- [Kind](https://kind.sigs.k8s.io/) at the version pinned in `versions.env` and
  Docker when following the local cluster steps. Older Kind releases cannot
  create the pinned Kubernetes 1.37 cluster.
- Available loopback port `18080` for the temporary port forward.
- Network access to pull images, manifests, and Helm charts.

Run commands from this directory:

```bash
cd examples/openshell
set -a
source versions.env
set +a
```

## Create a Kind cluster with Calico

Skip this section when using an existing cluster with a NetworkPolicy-enforcing
CNI. Kind's default CNI does not enforce `NetworkPolicy`, so the supplied Kind
configuration disables it and the following commands install Calico.

```bash
kind create cluster \
  --name agentgateway-openshell \
  --image "${KIND_NODE_IMAGE}" \
  --config kind-config.yaml

kubectl create \
  -f "https://raw.githubusercontent.com/projectcalico/calico/${CALICO_VERSION}/manifests/v3_projectcalico_org.yaml"
kubectl create \
  -f "https://raw.githubusercontent.com/projectcalico/calico/${CALICO_VERSION}/manifests/tigera-operator.yaml"
kubectl create \
  -f "https://raw.githubusercontent.com/projectcalico/calico/${CALICO_VERSION}/manifests/custom-resources.yaml"

kubectl rollout status deployment/tigera-operator \
  --namespace tigera-operator --timeout=5m
kubectl wait --for=create namespace/calico-system --timeout=5m
kubectl wait --for=create deployment/calico-kube-controllers \
  --namespace calico-system --timeout=5m
kubectl wait --for=create daemonset/calico-node \
  --namespace calico-system --timeout=5m
kubectl rollout status deployment/calico-kube-controllers \
  --namespace calico-system --timeout=5m
kubectl rollout status daemonset/calico-node \
  --namespace calico-system --timeout=5m
```

The verification script runs live ingress and egress policy probes. It stops
before testing OpenShell if the cluster CNI does not enforce either direction.

## Install agentgateway

Install the Gateway API CRDs and the pinned agentgateway controller:

```bash
kubectl apply --server-side --force-conflicts \
  -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml"

helm upgrade --install agentgateway-crds \
  oci://cr.agentgateway.dev/charts/agentgateway-crds \
  --create-namespace \
  --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}"

helm upgrade --install agentgateway \
  oci://cr.agentgateway.dev/charts/agentgateway \
  --namespace agentgateway-system \
  --version "${AGENTGATEWAY_VERSION}" \
  --wait \
  --timeout 5m

kubectl wait --for=condition=Accepted gatewayclass/agentgateway --timeout=5m
```

## Install Agent Sandbox and OpenShell

OpenShell uses the Kubernetes Agent Sandbox API for its sandbox workloads.
Install the pinned controller before installing OpenShell:

```bash
kubectl apply \
  -f "https://github.com/kubernetes-sigs/agent-sandbox/releases/download/${AGENT_SANDBOX_VERSION}/sandbox.yaml"
kubectl rollout status deployment/agent-sandbox-controller \
  --namespace agent-sandbox-system \
  --timeout 5m
```

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

### Manual verification

First, confirm that the cluster CNI enforces both ingress and egress
`NetworkPolicy`. Create two temporary pods and verify connectivity before
applying a policy:

```bash
kubectl create namespace openshell-policy-probe
kubectl run policy-server -n openshell-policy-probe \
  --image=busybox:1.37.0 --labels=app=policy-server \
  --command -- sh -c \
  'mkdir -p /www && echo ready >/www/index.html && httpd -f -p 8080 -h /www'
kubectl run policy-client -n openshell-policy-probe \
  --image=busybox:1.37.0 --labels=app=policy-client \
  --command -- sleep 3600
kubectl expose pod policy-server -n openshell-policy-probe \
  --port=8080 --target-port=8080
kubectl wait --for=condition=Ready pod/policy-server pod/policy-client \
  -n openshell-policy-probe --timeout=120s
kubectl exec -n openshell-policy-probe policy-client -- \
  wget -q -T 5 -O - http://policy-server:8080
```

The final command prints `ready`. Apply an ingress-deny policy:

```bash
kubectl apply -n openshell-policy-probe -f - <<'EOF'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-server-ingress
spec:
  podSelector:
    matchLabels:
      app: policy-server
  policyTypes:
    - Ingress
EOF

sleep 5
kubectl exec -n openshell-policy-probe policy-client -- \
  wget -q -T 5 -O - http://policy-server:8080
```

The request must time out or otherwise fail. Remove the ingress policy and
confirm that connectivity recovers before testing egress:

```bash
kubectl delete networkpolicy deny-server-ingress -n openshell-policy-probe
sleep 5
kubectl exec -n openshell-policy-probe policy-client -- \
  wget -q -T 5 -O - http://policy-server:8080

kubectl apply -n openshell-policy-probe -f - <<'EOF'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-client-egress
spec:
  podSelector:
    matchLabels:
      app: policy-client
  policyTypes:
    - Egress
EOF

sleep 5
kubectl exec -n openshell-policy-probe policy-client -- \
  wget -q -T 5 -O - http://policy-server:8080
```

The first request prints `ready`; the request after applying the egress policy
must fail. Remove the probe namespace:

```bash
kubectl delete namespace openshell-policy-probe
```

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

### Automated verification

Run the smoke test with the matching OpenShell CLI on `PATH`:

```bash
./verify.sh
```

Set `OPENSHELL_BIN` when the CLI is installed elsewhere:

```bash
OPENSHELL_BIN=/path/to/openshell ./verify.sh
```

The script verifies NetworkPolicy enforcement, waits for the `Gateway` and
`GRPCRoute`, connects the CLI through the agentgateway proxy, creates a sandbox,
executes a deterministic command, and deletes the sandbox. It uses a temporary
OpenShell configuration directory and does not change the user's registered
gateways.

## Clean up

Remove the OpenShell release and namespace:

```bash
./cleanup.sh
```

The agentgateway, Gateway API, Agent Sandbox, and Calico installations remain
available for other examples. Delete the disposable Kind cluster to remove
everything:

```bash
kind delete cluster --name agentgateway-openshell
```
