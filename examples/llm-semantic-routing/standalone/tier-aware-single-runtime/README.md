# Standalone tier-aware routing

## Prerequisites

- The [shared prerequisites](../../tier-aware-single-runtime/README.md#prerequisites).
- A running standalone agentgateway; see the
  [setup guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/).
- Docker Compose v2 and a free local TCP port 50051 for vSR.

The sample assumes agentgateway runs on the Docker host. For a containerized or
remote gateway, adapt the ExtProc address and network access in the supplied
configuration so it can reach vSR.

## Configure the integration

Run from this directory. Start only the supporting vSR runtime:

```bash
docker compose --env-file ../../tier-aware-single-runtime/versions.env up -d --wait
```

Make `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` available to your existing gateway
process. Merge [agentgateway.yaml](agentgateway.yaml) into its configuration,
preserving unrelated listeners and models; see the
[configuration update guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/update/).
The sample uses port 4000 and contacts vSR on `localhost:50051`.

```bash
export ENDPOINT=http://127.0.0.1:4000
```

Adjust the URL for your listener, then follow the
[shared manual checks](../../tier-aware-single-runtime/README.md#try-the-routing).
Inspect vSR with `docker compose logs --tail=100 semantic-router`, using the
same `--env-file` option. Provider credentials belong only in agentgateway.

## Cleanup

Restore the previous gateway configuration, then remove the supporting runtime:

```bash
docker compose --env-file ../../tier-aware-single-runtime/versions.env down
```

Retain existing gateway credentials that other routes use.
