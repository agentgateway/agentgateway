# NetBird Agent Network with agentgateway

Connect an existing [NetBird Agent Network](https://docs.netbird.io/agent-network/quickstart)
to agentgateway using the [standalone](standalone/README.md) or
[Kubernetes](kubernetes/README.md) configuration. NetBird authorizes callers,
injects a virtual API key, and forwards trusted identity headers to the gateway.

```text
Authorized NetBird client -> Agent Network proxy -> private agentgateway -> LLM providers
```

## Prerequisites

- A working NetBird Agent Network deployment and an authenticated NetBird client.
  Follow the [upstream quickstart](https://docs.netbird.io/agent-network/quickstart)
  for server, proxy, DNS, TLS, and client setup. NetBird 0.78.1 and dashboard
  2.92.0 are the minimum versions tested with this integration.
- A running agentgateway with a private listener reachable by the NetBird proxy.
  Restrict direct access to that listener to the proxy using your firewall or
  NetworkPolicy. A shared API key alone cannot make identity headers trustworthy.
- OpenAI and Anthropic credentials for the sample routes, plus `curl` and `jq`.

First configure the gateway using one of the deployment guides above, then
complete the following steps in the NetBird dashboard.

## Connect the provider

1. Open **Agent Network > Providers** and select **agentgateway**.
2. Set the upstream URL to the private listener configured in the deployment
   guide. Enter the same `NETBIRD_VIRTUAL_KEY` used by the gateway, keep the
   provider enabled, and keep **Forward identity metadata** enabled. Leave the
   model list empty to allow the gateway's models.
3. Open **Agent Network > Policies** and allow the intended caller's source
   group to access this provider. Use a dedicated provider and policy for this
   example so they can be removed independently.

See NetBird's [provider](https://docs.netbird.io/agent-network/providers) and
[policy](https://docs.netbird.io/agent-network/policies) guides for details.

## Try a request

Copy the generated Agent Network endpoint from the Providers page. Run these
commands on the authenticated NetBird client; provider credentials and the
virtual key stay on the servers. If the endpoint uses a private CA, configure
`CURL_CA_BUNDLE` with its trusted certificate.

```bash
export NETBIRD_AGENT_ENDPOINT=replace-with-your-agent-network-hostname
curl -fsS "https://${NETBIRD_AGENT_ENDPOINT}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Reply with the word connected."}],
    "max_tokens": 16
  }' | jq

curl -fsS "https://${NETBIRD_AGENT_ENDPOINT}/v1/messages" \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2023-06-01' \
  --data-binary '{
    "model": "claude-haiku-4-5",
    "messages": [{"role": "user", "content": "Reply with the word connected."}],
    "max_tokens": 16
  }' | jq
```

These requests use the upstream provider APIs and may incur charges. Expect a
response containing `connected` and a record with the caller identity in
**Agent Network > Usage & Logs**. An authenticated client whose group has no
matching policy should receive HTTP 403. A request to the same endpoint from
outside NetBird must not reach the provider; depending on private DNS and proxy
configuration it can fail to resolve, fail to connect, or receive HTTP 403.

The gateway maps `x-netbird-user-id` and `x-netbird-groups` to `agentgateway.user`
and `agentgateway.group` for attribution. Group headers contain display names
as CSV; do not use them as stable group IDs for authorization.

## Cleanup

Remove only the NetBird provider and policy created for this example, then
follow the deployment guide's cleanup. Retain your existing NetBird deployment,
clients, cluster, and agentgateway installation.
