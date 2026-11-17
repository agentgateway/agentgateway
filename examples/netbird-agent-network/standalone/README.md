# Standalone NetBird integration

## Prerequisites

- The [shared prerequisites](../README.md#prerequisites), including NetBird.
- An existing standalone agentgateway installation; see the
  [setup guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/).
  This configuration targets agentgateway v1.6.0.
- OpenSSL for generating the virtual key.
- A private address reachable from the NetBird proxy on TCP 3000. If the proxy
  is containerized, `localhost` refers to that container; use a reachable host
  or container address instead. NetBird management must resolve this address
  as private when validating the provider.

## Configure the gateway

Generate a virtual key and retain it in your secret store:

```bash
export NETBIRD_VIRTUAL_KEY=$(openssl rand -hex 32)
```

Make `NETBIRD_VIRTUAL_KEY`, `OPENAI_API_KEY`, and `ANTHROPIC_API_KEY` available to
your existing gateway process through its service or container environment.
Merge [agent-network-agentgateway.yaml](agent-network-agentgateway.yaml) into
its configuration using the
[configuration update guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/update/).
Preserve unrelated listeners and model routes. The sample enables strict
Bearer-key authentication on port 3000 and routes the listed models to OpenAI
and Anthropic.

Restrict port 3000 to the NetBird proxy before connecting the provider. Use
HTTPS or mTLS for the upstream hop when its private network is not a sufficient
trust boundary.

On the gateway host, confirm that a request without the virtual key is rejected:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/v1/models
# 401
```

If the gateway listens on another address, adjust the diagnostic URL. Set the
NetBird provider's upstream URL to `http://<private-gateway-address>:3000`, then
follow [Connect the provider](../README.md#connect-the-provider) and
[Try a request](../README.md#try-a-request).

## Cleanup

Remove the NetBird provider and policy created for this example. Restore your
previous gateway configuration and remove the example's virtual key from its
environment and secret store.
