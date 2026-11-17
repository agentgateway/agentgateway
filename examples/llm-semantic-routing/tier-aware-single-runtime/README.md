# Tier-aware routing with one vSR runtime

Choose the [standalone](../standalone/tier-aware-single-runtime/README.md) or
[Kubernetes](../k8s/tier-aware-single-runtime/README.md) guide to configure an
existing agentgateway with one [vLLM Semantic Router (vSR)](https://vllm-sr.ai/).
Both modes use [the same routing configuration](semantic-router-config.yaml)
and the vSR release in [versions.env](versions.env), tested with agentgateway v1.6.0.

```text
Client -> agentgateway tier validation -> vSR tier + keyword selection
       -> agentgateway model authorization -> OpenAI or Anthropic
```

| Tier | Allowed models | Selected for a STEM prompt |
| --- | --- | --- |
| Basic | GPT-4.1, GPT-5.4 | GPT-5.4 |
| Standard | Basic models and Claude Haiku 4.5 | Claude Haiku 4.5 |
| Pro | Standard models and Claude Sonnet 4.6 | Claude Sonnet 4.6 |

The `auto` model selects a tier-specific model when the prompt matches a STEM
keyword; other prompts fall back to GPT-4.1. Keywords make this example
predictable without a classifier model. See vSR's
[signals](https://vllm-sr.ai/docs/tutorials/signal/overview) for other options.

## Prerequisites

- An existing agentgateway installation as described in the deployment guide.
- OpenAI and Anthropic credentials with access to the configured models.
- `curl` for the manual checks below. Requests make billable provider calls.

## Try the routing

After completing a deployment guide, set `ENDPOINT` to your gateway's URL.
Send the same STEM prompt with each tier:

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

Expect HTTP 200 with generated text and these headers:

| Tier | `x-vsr-selected-model` | `x-vsr-selected-decision` |
| --- | --- | --- |
| Basic | `gpt-5.4` | `basic_stem` |
| Standard | `claude-haiku-4-5-20251001` | `standard_stem` |
| Pro | `claude-sonnet-4-6` | `pro_stem` |

Repeat with `Say hello.` as the prompt: all tiers should select `gpt-4.1`.
Confirm that a Basic caller cannot request Sonnet directly:

```bash
curl -sS -i "$ENDPOINT/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -H 'X-Authz-User-Id: demo-user' \
  -H 'X-Entitlement-Tier: basic' \
  -d '{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"Say hello."}],"max_tokens":64}'
# HTTP 403
```

An `auto` request with a missing user ID, missing or unknown tier, or a
client-supplied `X-VSR-Skip-Processing` header should also receive HTTP 403.
The ExtProc policy fails closed if vSR is unavailable.

## Trusted tier context

The sample trusts caller-supplied user and tier headers. Before exposing it,
authenticate the caller and require those headers to match verified entitlements,
for example with JWT claims or external authorization. API-key authentication
alone does not verify the tier. Keep per-model authorization enabled because a
caller can request a model directly and bypass automatic selection.

## Troubleshooting

Inspect the selected-model header, gateway policy status, and vSR logs. Check
provider credentials, quotas, and model access when a correctly routed request
fails upstream. Disabled embedding/cache messages are expected. If you change
model IDs, update the shared vSR configuration and both gateway configurations.
Restart the vSR container or Deployment after updating its ConfigMap/configuration.
