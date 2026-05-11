# Kind deployment for xDS UI validation

Stands up a local kind cluster running agentgateway in xDS mode, so the dev UI
can be validated against a real `/config_dump` shape rather than file-mode
fallback data.

## What it deploys

- A kind cluster (`agw-xds-test` by default).
- Gateway API CRDs (v1.5.0).
- The agentgateway controller and CRDs from the published OCI charts at
  `cr.agentgateway.dev` (v1.1.0).
- Test workloads in `agentgateway-test`:
  - `httpbin` Deployment + Service (HTTP backend).
  - `mcp-everything` Deployment + Service (`tzolov/mcp-everything-server:v3`,
    StreamableHTTP transport on port 3001).
  - Two `AgentgatewayBackend` CRs: an AI backend pointing at host ollama
    (see "Ollama dependency" below), and an MCP backend pointing at the
    in-cluster `mcp-everything` Service.
- A `Gateway` with three listeners (8080 → httpbin, 9000 → AI, 9001 → MCP)
  and one `HTTPRoute` per listener, bound via `sectionName`. The controller
  turns these into xDS pushes to the agentgateway data plane pod.

## Ollama dependency

The AI backend (`ollama-smallthinker`) is configured to call ollama at
`host.docker.internal:11434`. To exercise actual AI traffic through the
gateway you need ollama running on the host with the `smallthinker` model
pulled:

```sh
ollama serve              # in one terminal
ollama pull smallthinker  # one-time
```

For UI validation (config rendering, mapper coverage) you do **not** need
ollama running — the controller pushes the backend config to xDS regardless
of whether the upstream is reachable.

We deliberately keep ollama on the host instead of deploying it in-cluster
to avoid the cold-start cost (~5 GB of image + model pulls, ~5–10 min on
first run).

No local binaries are built — see the conversation history if you need to
swap in locally-built proxy/controller images.

## Prerequisites

- `kind`
- `kubectl`
- `helm`
- `docker` (running)

## Run

```sh
./scripts/kind-deployment/setup.sh
```

When complete, the script prints a `kubectl port-forward` command for the
data plane pod. Run that in a separate terminal:

```sh
kubectl -n agentgateway-test port-forward pod/<GW_POD> 15000 8080 9000 9001
```

Port roles:
- `15000` — admin / `/config_dump` (consumed by the dev UI)
- `8080` — httpbin listener
- `9000` — AI listener (LLM Playground sends `POST /v1/chat/completions` here)
- `9001` — MCP listener (MCP Playground connects here)

Then start the dev UI:

```sh
cd ui && yarn dev
```

Open http://localhost:3000. The XDS Mode banner should appear and the
HierarchyTree should render a single Gateway on :8080 backed by httpbin.

## Validate

- Dev console — watch for `configMapper: unknown ...` warns (each is a
  schema gap worth filing).
- Network tab — `/config_dump` should be the only config endpoint hit;
  `/config` should not be requested.
- Save buttons should be disabled with a tooltip.

## Tear down

```sh
./scripts/kind-deployment/teardown.sh
```

Deletes the kind cluster.

## Tweaks

Override defaults with env vars:

```sh
CLUSTER_NAME=my-cluster \
AGENTGATEWAY_VERSION=v1.1.0 \
GATEWAY_API_VERSION=v1.5.0 \
./scripts/kind-deployment/setup.sh
```

To extend the test config (apply oidc, tls, mcp, etc.), add manifests under
`manifests/` and re-apply with `kubectl apply -f`.
