# Kubernetes NetBird integration

## Prerequisites

- The [shared prerequisites](../README.md#prerequisites), including NetBird.
- An existing Kubernetes cluster with a CNI that enforces NetworkPolicy. See
  [Kind](https://kind.sigs.k8s.io/docs/user/quick-start/) for local clusters;
  Kind's default CNI does not enforce NetworkPolicy.
- An installed agentgateway controller, CRDs, and `agentgateway` GatewayClass;
  see the [installation guide](https://agentgateway.dev/docs/kubernetes/latest/documentation/quickstart/install/).
  This configuration targets agentgateway v1.6.0.
- `kubectl`, `envsubst`, and OpenSSL.
- A NetBird proxy that can reach cluster Services. The sample assumes its pods
  are in `netbird-agent-network`, labeled `app.kubernetes.io/name: netbird-proxy`.
  Adapt the namespace and ingress selectors in the manifests for your deployment;
  use a `namespaceSelector` as well when the proxy is in another namespace.
  The NetBird management server must resolve the upstream hostname to its
  private address.

## Configure the gateway

Run from this directory. Set the provider credentials and generate a virtual
key that you will also enter in NetBird:

```bash
export OPENAI_API_KEY=replace-with-an-openai-key
export ANTHROPIC_API_KEY=replace-with-an-anthropic-key
export NETBIRD_VIRTUAL_KEY=$(openssl rand -hex 32)
export NETBIRD_VIRTUAL_KEY_SHA256=$(printf '%s' "${NETBIRD_VIRTUAL_KEY}" \
  | openssl dgst -sha256 -r | awk '{print $1}')

envsubst '${OPENAI_API_KEY} ${ANTHROPIC_API_KEY} ${NETBIRD_VIRTUAL_KEY_SHA256}' \
  < secrets.example.yaml | kubectl apply -f -
kubectl apply -f agent-network-gateway.yaml
kubectl wait -n netbird-agent-network gateway/netbird-agentgateway \
  --for=condition=Programmed --timeout=120s
kubectl rollout status -n netbird-agent-network deployment/netbird-agentgateway \
  --timeout=120s
```

Retain the virtual key in your secret store. The namespace must already exist.
The manifests configure a private ClusterIP Gateway, provider backends, routing,
strict API-key authentication, identity attribution, and ingress limited to the
NetBird proxy. The headless upstream Service resolves to the private gateway
pod address, allowing NetBird to recognize a private provider endpoint.

If you reuse an existing Gateway, merge the listener and infrastructure settings,
adapt the route's parent reference and Service/NetworkPolicy selectors, and
preserve unrelated configuration. Use HTTPS or mTLS for the upstream hop when
pod-network isolation is insufficient.

Confirm that the listener rejects requests without the virtual key. In one
terminal run:

```bash
kubectl port-forward -n netbird-agent-network service/netbird-agentgateway 18080:80
```

In another terminal:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/v1/models
# 401
```

Stop the port-forward. Set the NetBird provider's upstream URL to
`http://netbird-agentgateway-upstream.netbird-agent-network.svc.cluster.local`,
then follow [Connect the provider](../README.md#connect-the-provider) and
[Try a request](../README.md#try-a-request) from an authenticated NetBird client.

## Cleanup

Remove the NetBird provider and policy created for this example. If you applied
the supplied manifests as new resources, remove them with:

```bash
kubectl delete -f agent-network-gateway.yaml
kubectl delete secret -n netbird-agent-network \
  netbird-openai netbird-anthropic netbird-agentgateway-keys
```

If you merged configuration into existing resources, restore their prior
settings instead. Retain the existing namespace, NetBird deployment, cluster,
and agentgateway installation.
