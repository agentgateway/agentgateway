# Tier-aware routing with one vLLM Semantic Router runtime

This example uses Docker Compose to run agentgateway and one
[vLLM Semantic Router (vSR)](https://vllm-sr.ai/) runtime. vSR selects a model
based on the caller's access tier and STEM keywords in the prompt. It reads
its routing rules from YAML and requires no Kubernetes cluster or GPU.

This is the standalone counterpart of the
[Kubernetes single-runtime example](../../k8s/tier-aware-single-runtime/).
Both examples use the same vSR configuration.

Requests pass through vSR before agentgateway calls the selected provider:

```text
Client: model=auto + user ID + tier
  -> agentgateway: validate tier context
  -> vSR ExtProc: combine tier and STEM keywords, rewrite model
  -> agentgateway: authorize selected model and translate provider protocol
  -> OpenAI or Anthropic
```

vSR selects a model while agentgateway makes the provider request. The standalone
`llm.policies` configuration runs before model selection, and each model's
`authorization` rules check whether the caller's tier allows the selected model.
These rules also apply when a caller requests a model by name.

| Tier | Allowed models | Model selected for a STEM prompt |
| --- | --- | --- |
| Basic | GPT-4.1, GPT-5.4 | GPT-5.4 |
| Standard | Basic models and Claude Haiku 4.5 | Claude Haiku 4.5 |
| Pro | Standard models and Claude Sonnet 4.6 | Claude Sonnet 4.6 |

Requests using `auto` without a matching STEM keyword fall back to GPT-4.1 in
all tiers. Keywords keep this example deterministic and can be replaced or
combined with other [vSR signals](https://vllm-sr.ai/docs/tutorials/signal/overview).

## Start

Prerequisites:

- Docker with Docker Compose v2, curl, and jq.
- OpenAI and Anthropic API keys with access to the configured models.
- A free local port, defaulting to 4000.

Compose uses agentgateway v1.5.0 and the `vllm-sr:latest` image for vSR.
Only agentgateway's HTTP listener is published, on localhost. vSR's gRPC and
management ports remain inside the Compose network. Both containers mount
their configurations read-only.
The TCP health check waits for vSR's ExtProc listener before starting agentgateway.

Use the requests below to verify routing readiness.

From the repository root:

```bash
cd examples/llm-semantic-routing/standalone/tier-aware-single-runtime
cp env.example .env
chmod 600 .env
```

Set `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` in `.env`, or export them in your
shell. `.env` is ignored by Git to help prevent accidentally committing API keys.
Credentials are passed only to agentgateway.
Set `PORT` in `.env` if 4000 is occupied.

```bash
docker compose up -d --wait
export ENDPOINT=http://127.0.0.1:4000
```

Adjust `ENDPOINT` if you changed `PORT`.

## Send requests

Send the same prompt with each tier. These requests make billable calls to
OpenAI and Anthropic:

```bash
for tier in basic standard pro; do
  curl --fail-with-body -sS -i "$ENDPOINT/v1/chat/completions" \
    -H 'Content-Type: application/json' \
    -H 'X-Authz-User-Id: demo-user' \
    -H "X-Entitlement-Tier: $tier" \
    -H 'X-VSR-Debug: true' \
    -d '{"model":"auto","messages":[{"role":"user","content":"Define quantum physics in one sentence."}],"max_tokens":64}'
done
```

Expect HTTP 200 with generated text and these debug response headers:

| Tier | `x-vsr-selected-model` | `x-vsr-selected-decision` |
| --- | --- | --- |
| Basic | `gpt-5.4` | `basic_stem` |
| Standard | `claude-haiku-4-5-20251001` | `standard_stem` |
| Pro | `claude-sonnet-4-6` | `pro_stem` |

Replace the prompt with `Say hello.` to check the GPT-4.1 fallback. Replace
`auto` with `claude-sonnet-4-6` and use tier `basic` to check HTTP 403.

## Verify

Run the authorization and invalid-request checks:

```bash
./verify.sh
```

These checks cover models outside the caller's tier, missing or invalid headers,
and unknown models. They should not reach a provider.

To also test successful routing with billable provider calls, run:

```bash
RUN_LIVE_PROVIDER_TESTS=true ./verify.sh
```

The live tests check STEM routing and fallback for each tier, an allowed request
for Haiku by name, and a streaming Sonnet response. They check for generated
text as well as the expected selected-model headers.

To verify failure behavior, stop vSR and send a valid request:

```bash
docker compose stop semantic-router
curl -sS -i --max-time 30 "$ENDPOINT/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: basic' \
  -d '{"model":"auto","messages":[{"role":"user","content":"Say hello."}]}'
docker compose up -d --wait
```

Expect a gateway error rather than provider output. ExtProc is configured to
fail closed, including for explicitly named models.

## Trusted tier context

This example trusts the user ID and tier headers, so a caller can claim any tier.
Before exposing it to users, authenticate callers and verify their tier before
ExtProc runs. For example, combine JWT authentication with authorization rules
that require the headers to match validated token claims. API key authentication
alone does not verify a caller-supplied tier.

Client requests containing `x-vsr-skip-processing` are rejected. The vSR
configuration uses this header internally. Keep per-model authorization enabled
because clients can bypass automatic model selection by requesting a model by name.

## Troubleshooting

Check container status and recent logs:

```bash
docker compose ps
docker compose logs --tail=100 agentgateway semantic-router
```

Check the selected model to diagnose routing, and the response body for provider
credential, quota, or model-access errors. Messages about disabled embedding or
cache features are expected for this configuration.

If you change model IDs, update `agentgateway.yaml`, `semantic-router-config.yaml`,
and the expected models in `verify.sh`. Restart vSR after editing its configuration:

```bash
docker compose restart semantic-router
```

## Cleanup

Stop the example and remove its containers and network:

```bash
docker compose down
```

Remove `.env` separately when its credentials are no longer needed.
