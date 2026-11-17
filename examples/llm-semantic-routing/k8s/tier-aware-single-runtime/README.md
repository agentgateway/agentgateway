# Kubernetes tier-aware routing

## Prerequisites

- The [shared prerequisites](../../tier-aware-single-runtime/README.md#prerequisites).
- An existing cluster; see [Kind](https://kind.sigs.k8s.io/docs/user/quick-start/)
  for local testing.
- An installed agentgateway controller and matching CRDs; see the
  [installation guide](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/).
  The model API must be enabled (`agentgatewayModels.enabled=true`, the
  v1.6.0 default); see [model routing](https://agentgateway.dev/docs/kubernetes/latest/documentation/llm/models/serve/).
- An existing `agentgateway-proxy` Gateway in `agentgateway-system`, with an
  `http` listener that accepts `AgentgatewayModel` routes. See the
  [Gateway setup guide](https://agentgateway.dev/docs/kubernetes/latest/documentation/setup/gateway/).
- Existing provider Secrets `openai-secret` and `anthropic-secret` in that
  namespace, each with an `Authorization` key containing its provider API key;
  see [backend authentication](https://agentgateway.dev/docs/kubernetes/latest/documentation/security/backend-authn/key/).
- `kubectl` and `envsubst`.

Adapt the namespace, Gateway name, listener references, and Secret names in the
manifests to your installation. Retain other allowed route kinds on the listener.
Use the sample resource names only when they are unused. Run separately from
other vSR examples, which can reuse model, policy, and Service names.

## Configure the integration

Run from this directory. Create the vSR ConfigMap and supporting Deployment:

```bash
set -a
source ../../tier-aware-single-runtime/versions.env
set +a
kubectl -n agentgateway-system create configmap tier-aware-config \
  --from-file=config.yaml=../../tier-aware-single-runtime/semantic-router-config.yaml \
  --dry-run=client -o yaml | kubectl apply -f -
envsubst '${VSR_IMAGE_TAG}' < semantic-router.yaml | kubectl apply -f -
kubectl -n agentgateway-system rollout status deployment/semantic-router --timeout=600s
kubectl apply -f agentgateway-routing.yaml
```

The models route to OpenAI and Anthropic after vSR rewrites the request body.
Gateway-level authorization runs before ExtProc; model-level authorization
checks the selected model afterward. HTTPRoute matching runs too early to
route using a model rewritten by ExtProc, so this example uses AgentgatewayModel.

Check that the policy is accepted and attached:

```bash
kubectl wait -n agentgateway-system agentgatewaypolicy/tiered-semantic-routing \
  --for='jsonpath={.status.ancestors[0].conditions[?(@.type=="Accepted")].status}=True' \
  --timeout=60s
kubectl wait -n agentgateway-system agentgatewaypolicy/tiered-semantic-routing \
  --for='jsonpath={.status.ancestors[0].conditions[?(@.type=="Attached")].status}=True' \
  --timeout=60s
```

Set `ENDPOINT` to your existing Gateway URL, or port-forward in one terminal:

```bash
kubectl port-forward -n agentgateway-system service/agentgateway-proxy 8080:80
```

In another terminal:

```bash
export ENDPOINT=http://127.0.0.1:8080
```

Follow the [shared manual checks](../../tier-aware-single-runtime/README.md#try-the-routing).
For troubleshooting, inspect `kubectl describe agentgatewaypolicy tiered-semantic-routing`
and `kubectl logs deployment/semantic-router` in `agentgateway-system`.

## Cleanup

Stop the port-forward and remove only the example resources:

```bash
kubectl delete -f agentgateway-routing.yaml
kubectl delete deployment,service semantic-router -n agentgateway-system
kubectl delete configmap tier-aware-config -n agentgateway-system
```

Retain the existing Gateway, provider Secrets, cluster, and agentgateway installation.
