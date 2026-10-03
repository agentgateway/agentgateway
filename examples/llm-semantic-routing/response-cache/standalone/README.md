# Standalone Response Cache

Run agentgateway v1.5.0, vSR, Redis Open Source, and the HomeHub Python backend
with Docker Compose. No LLM credentials are required because HomeHub returns
fixed answers without calling an LLM provider. See the
[overview](../README.md) for the cache policy and request flow, or use the
[Kubernetes example](../kubernetes/README.md) to run in a kind cluster.

## Before You Begin

Install these tools:

- Docker with Compose v2
- curl
- jq

vSR downloads an embedding model on first startup, so allocate at least 6 CPUs,
10 GiB of memory, and 15 GiB of free disk space to Docker for the model and the
other services. The first download can take several minutes.

## Start

From the repository root:

```bash
cd examples/llm-semantic-routing/response-cache/standalone
docker compose --env-file versions.env up -d --wait --wait-timeout 650
```

`versions.env` selects agentgateway v1.5.0 and vSR `latest`. Docker Compose
pulls the current vSR image on startup. The [overview](../README.md) explains
why the example uses `latest`.

The agentgateway proxy listens on `127.0.0.1:3000`. To use another port, export `PORT`
before starting and verifying. Redis, HomeHub, and vSR are internal to the
Compose network. The shared Python script is mounted read-only into
`python:3.12-alpine` and served as `support-backend:8080`.

> [!NOTE]
> HomeHub does not stream responses, so ExtProc streaming is disabled and
> requests use `stream: false`. Agentgateway sends complete request and response
> bodies to vSR, which can cache the response in Redis.

## Verify

```bash
./verify.sh
./verify.sh --shared-vsr --redis-restart
```

The script resets the example cache and backend counter, then checks an
initial miss, identical and paraphrased cache hits, and an uncached question.
It checks vSR's debug headers and the backend counter to confirm that hits
bypass HomeHub.

`--shared-vsr` recreates vSR and checks that the new instance reuses Redis
entries. `--redis-restart` restarts Redis and checks that the cache survives.

Send a request manually:

```bash
curl -i "http://127.0.0.1:${PORT:-3000}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-VSR-Debug: true' \
  -d '{"model":"auto","stream":false,"messages":[{"role":"user","content":"How do I factory-reset my HomeHub X2?"}],"max_tokens":96}'
```

## Troubleshoot

```bash
docker compose --env-file versions.env ps -a
docker compose --env-file versions.env logs --tail=100 semantic-router agentgateway
docker compose --env-file versions.env exec redis redis-cli --raw FT._LIST
docker compose --env-file versions.env exec support-backend python -c \
  'import urllib.request; print(urllib.request.urlopen("http://localhost:8080/stats").read().decode())'
```

If vSR exits or startup times out, check its logs for model-download or
configuration errors. The first model download can take several minutes.
If a paraphrase misses the cache, inspect the similarity score in the debug
headers. See the [cache correctness guidance](https://agentgateway.dev/docs/kubernetes/main/integrations/llm/routing/vllm-semantic-router/#response-caching-in-production)
before adjusting the threshold.

## Cleanup

Stop the stack while preserving cached responses and downloaded models:

```bash
docker compose --env-file versions.env down
```

To also delete the example's Redis data and model volumes:

```bash
docker compose --env-file versions.env down --volumes
```
