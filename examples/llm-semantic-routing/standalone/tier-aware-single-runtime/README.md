# Tier-aware routing with one vLLM Semantic Router runtime

Run standalone agentgateway and one [vLLM Semantic Router (vSR)](https://vllm-sr.ai/)
with Docker Compose. vSR reads canonical YAML directly; no Kubernetes cluster,
operator, CRDs, classifier downloads, or GPU are required.

This is the standalone counterpart of the
[Kubernetes single-runtime example](../../k8s/tier-aware-single-runtime/).
Both examples use the same vSR configuration.

```text
Client: model=auto + user ID + tier
  -> agentgateway: validate tier context
  -> vSR ExtProc: combine tier and STEM keywords, rewrite model
  -> agentgateway: authorize selected model and translate provider protocol
  -> OpenAI or Anthropic
```

vSR selects a model while agentgateway makes the provider request. The standalone
`llm.policies` configuration runs before model selection, and each model's
`authorization` rules enforce entitlements after selection. Explicit model
requests are subject to the same authorization rules.

| Tier | Allowed models | STEM selection |
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
- A free local port, defaulting to 3000.

Compose uses agentgateway v1.5.0 and
`ghcr.io/vllm-project/semantic-router/vllm-sr:latest` for vSR.
Only agentgateway's HTTP listener is published, on localhost. vSR's gRPC and
management ports remain inside the Compose network.
The TCP health check waits for vSR's ExtProc listener before starting agentgateway;
use the requests below to verify routing readiness.

From the repository root:

```bash
cd examples/llm-semantic-routing/standalone/tier-aware-single-runtime
cp env.example .env
chmod 600 .env
```

Set `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` in `.env`, or export them in your
shell. `.env` is ignored by Git. Credentials are passed only to agentgateway.
Set `PORT` in `.env` if 3000 is occupied.

```bash
docker compose up -d --wait
export ENDPOINT=http://127.0.0.1:3000
```

Adjust `ENDPOINT` if you changed `PORT`. Requests below make billable provider
calls. Both containers mount their configurations read-only.

## Send requests

Send the same prompt with each tier:

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

The default verification checks forbidden models, missing or invalid tier,
missing user ID, client-supplied skip headers, and unknown models. These checks
should not reach a provider:

```bash
./verify.sh
```

Enable successful provider requests explicitly:

```bash
RUN_LIVE_PROVIDER_TESTS=true ./verify.sh
```

This also checks all three STEM selections, all three fallbacks, an allowed
explicit Haiku request, and a streaming Sonnet response. It requires generated
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

The supplied user ID and tier headers demonstrate routing, not authentication.
A caller can claim any tier in this local demo. Before exposing it to users,
validate identity and bind the headers to trusted entitlements before ExtProc
runs, for example with JWT authentication and authorization that requires the
headers to match validated claims. API key authentication alone does not verify
a caller-supplied tier.

Client requests containing `x-vsr-skip-processing` are rejected. The vSR
configuration retains the Kubernetes example's internal skip-processing setting.
Per-model authorization is required even with tier-aware vSR decisions because
clients can explicitly request a model.

## Troubleshooting and cleanup

```bash
docker compose ps
docker compose logs --tail=100 agentgateway semantic-router
```

Provider credential, quota, and model-access errors are separate from routing
errors. Check the selected model and provider response. vSR may log disabled
embedding/cache features; this example intentionally uses no embedding model.
If changing model IDs, update both configuration files and verification
expectations. Restart vSR after editing its configuration:

```bash
docker compose restart semantic-router
```

Stop the example and remove its containers and network:

```bash
docker compose down
```

Remove `.env` separately when its credentials are no longer needed.
