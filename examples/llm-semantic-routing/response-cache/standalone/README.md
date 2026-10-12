# Standalone Response Cache

Configure response caching on an existing standalone agentgateway using vSR,
Redis, and HomeHub. See the [overview](../README.md) or the
[Kubernetes example](../kubernetes/README.md).

## Prerequisites

- A running [standalone agentgateway](https://agentgateway.dev/docs/standalone/latest/documentation/setup/).
  This example was tested with agentgateway v1.6.0.
- Docker with Compose v2, curl, jq, and uuidgen.
- Available loopback ports `50051` for vSR and `18080` for HomeHub.

The sample configuration assumes agentgateway runs on the Docker host. For an
existing container or remote gateway, replace loopback addresses with endpoints
reachable from that gateway. vSR's provider endpoint is the address used by
agentgateway to reach HomeHub.

vSR downloads an embedding model on first startup. The tested local environment
had 6 CPUs, 10 GiB RAM, and 15 GiB disk available; the first download can take
several minutes.

## Start the Supporting Services

```bash
cd examples/llm-semantic-routing/response-cache/standalone
docker compose --env-file versions.env up -d --wait --wait-timeout 650
```

[`versions.env`](versions.env) pins vSR v0.4.0, Redis, and HomeHub commit
`fd19c81`. Redis stays internal to the Compose network.

## Configure agentgateway

Merge [agentgateway.yaml](agentgateway.yaml) into your existing configuration,
following the [configuration update guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/update/).
The sample config adds the HomeHub model and the vSR ExtProc policy to a listener
on port 3000. Adjust the listener port and backend addresses for your deployment.

## Verify

Set the gateway URL and follow the
[shared four-request walkthrough](../README.md#verify-the-cache):

```bash
export GATEWAY_URL=http://127.0.0.1:3000
```

Check the backend count before and after requests:

```bash
docker compose --env-file versions.env exec support-backend python -c \
  'import urllib.request; print(urllib.request.urlopen("http://localhost:8080/stats").read().decode())'
```

## Troubleshoot

```bash
docker compose --env-file versions.env ps -a
docker compose --env-file versions.env logs --tail=100 semantic-router
docker compose --env-file versions.env exec redis redis-cli --raw FT._LIST
```

If vSR exits or startup times out, check its logs for model-download or
configuration errors. The first model download can take several minutes.
If a paraphrase misses the cache, inspect the similarity score in the debug
headers. See the [cache correctness guidance](https://agentgateway.dev/docs/kubernetes/main/integrations/llm/routing/vllm-semantic-router/#response-caching-in-production)
before adjusting the threshold.

## Cleanup

Stop the supporting services while preserving cached responses and downloaded
models. Remove the example model and ExtProc policy from your gateway config:

```bash
docker compose --env-file versions.env down
```

To also delete the example's Redis data and model volumes:

```bash
docker compose --env-file versions.env down --volumes
```
