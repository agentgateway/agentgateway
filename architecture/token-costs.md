# Token Cost Multipliers (`tokenCosts`)

## Overview

Token cost multipliers allow operators to express rate limit budgets in cost-proportional
units rather than raw token counts. By assigning per-category multipliers (input, output,
cache write, cache read), `requests_per_unit` on the rate limit server becomes a meaningful
cost ceiling that reflects actual model pricing.

## Rate limiting in agentgateway

Agentgateway supports four rate limiting modes, distinguished by scope and unit:

| Mode | Scope | Unit | Configured via |
|------|-------|------|----------------|
| Local request | Per-instance | HTTP requests | `localRateLimit` with type `requests` |
| Local token | Per-instance | LLM tokens | `localRateLimit` with type `tokens` |
| Remote request | Global (Envoy + Redis) | HTTP requests | `remoteRateLimit` with type `requests` |
| Remote token | Global (Envoy + Redis) | LLM tokens | `remoteRateLimit` with type `tokens` |

**Token cost multipliers apply only to token-based rate limiting** (both local and remote).
Request-based rate limiting is unaffected — it continues to count 1 unit per HTTP request
regardless of `tokenCosts` configuration.

### How token-based rate limiting works

Token-based rate limiting charges budget in two phases:

1. **Pre-flight (request time)** — Before the request reaches the LLM, the gateway
   estimates the cost using the input token count (obtained via tokenization or the
   request body) and pre-charges that amount against the budget.

2. **True-up (response time)** — After the LLM responds, the gateway knows the actual
   token breakdown (input, output, cache read, cache write). It computes the difference
   between the actual cost and the pre-flight estimate, then amends the budget.

Without `tokenCosts`, both phases use raw token counts (1 budget unit = 1 token).
With `tokenCosts`, the charges are scaled by category-specific multipliers.

### Which rate limiters are affected

| Mode | Pre-flight | True-up | Notes |
|------|-----------|---------|-------|
| **Remote token** | ✅ Weighted | ✅ Weighted | `hits_addend` sent to Envoy uses weighted values |
| **Local token** | ✅ Weighted | ✅ Weighted | Gate check and true-up both use weighted budget units |
| **Remote request** | — | — | Not affected (counts requests, not tokens) |
| **Local request** | — | — | Not affected (counts requests, not tokens) |

## Budget formula

When `tokenCosts` is configured, budget units are computed as:

```
budget_units = (base_input × input)
             + (output_tokens × output)
             + (cache_write_tokens × cacheWrite)
             + (cache_read_tokens × cacheRead)

where base_input = total_input_tokens − cache_read_tokens − cache_write_tokens
```

Cache tokens are subtracted from total input before applying the input multiplier so that
they are not double-counted — each token category is billed exactly once at its own rate.

### Pre-flight charge

At request time, only estimated input tokens are known. The pre-flight charge accounts for
prompt caching when the backend has a `promptCaching` configuration with `cacheMessages: true`:

```
Without caching (or < 2 messages in request):
  pre_flight_charge = estimated_input × input

With caching enabled:
  cache_point = (message_count − 2) − cacheMessageOffset   (clamped ≥ 0)
  cached_messages  = cache_point + 1
  cached_tokens    = estimated_input × (cached_messages / message_count)
  uncached_tokens  = estimated_input − cached_tokens

  pre_flight_charge = (cached_tokens × cacheRead) + (uncached_tokens × input)
```

The cache-aware estimate assumes tokens are roughly evenly distributed across messages and
that messages up to the cache point will be served from cache on repeat calls. This reduces
pre-flight overcharging for workloads with high cache hit rates — without it, the gateway
would charge all input tokens at the full `input` multiplier even though most will
ultimately be billed at the discounted `cacheRead` rate.

### True-up amendment

After the LLM responds, the delta between actual weighted cost and pre-flight is applied:

```
actual_cost = (base_input × input)
            + (output × output)
            + (cache_write × cacheWrite)
            + (cache_read × cacheRead)

delta = actual_cost − pre_flight_charge
```

The gateway stores the pre-flight charge computed at request time so the true-up delta
is always computed against the exact value that was originally deducted, regardless of
whether the cache-aware or simple formula was used.

The delta is applied via `amend_tokens` to both local and remote token rate limiters.
For the remote limiter, this fires an asynchronous gRPC call to the Envoy ratelimit server.

> **Negative deltas and the remote limiter:** The Envoy ratelimit protocol does not
> support negative amendments (refunds). When the true-up delta is negative — for
> example because the pre-flight overestimated input tokens — the remote limiter
> silently drops the amendment. This means the remote budget may be slightly
> overcharged in cases where actual input is lower than estimated. The local token
> limiter does support negative amendments, so local budgets remain accurate.
> This is a pre-existing limitation of the remote ratelimit path, not specific to
> `tokenCosts`.

## Configuration

`tokenCosts` can be set on `AgentgatewayBackend` or `AgentgatewayPolicy`:

```yaml
apiVersion: gateway.agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: claude-sonnet
spec:
  ai:
    provider:
      bedrock:
        modelId: anthropic.claude-3-5-sonnet-20241022-v2:0
  policies:
    ai:
      tokenCosts:
        input: 1         # non-cached input tokens — baseline (1×)
        output: 5        # output tokens — 5× baseline
        cacheWrite: 1.25 # tokens that create a new cache entry
        cacheRead: 0.1   # tokens served from cache — 90% discount
```

### Field reference

| Field | Type | Default | Description |
|---|---|---|---|
| `input` | `float64` | `1.0` | Multiplier for non-cached input tokens |
| `output` | `float64` | `1.0` | Multiplier for output (completion) tokens |
| `cacheWrite` | `float64` | `1.0` | Multiplier for tokens that create a new cache entry |
| `cacheRead` | `float64` | `1.0` | Multiplier for tokens served from an existing cache entry |

All fields are optional. Omitting the entire `tokenCosts` block or any individual field
defaults that multiplier to `1.0`. All values must be positive (`> 0`).

## Deriving multipliers from model pricing

Choose a baseline token category (typically non-cached input) and compute ratios:

```
multiplier = category_price / baseline_price
```

### Example: Anthropic Claude 3.5 Sonnet on Bedrock

| Category | Price | Multiplier |
|---|---|---|
| Input (non-cached) | $3.00 / MTok | 1.00 (baseline) |
| Cache write | $3.75 / MTok | 1.25 |
| Cache read | $0.30 / MTok | 0.10 |
| Output | $15.00 / MTok | 5.00 |

With these multipliers, `requests_per_unit: 1000000` means roughly 1M input-token-equivalents
of spend — a cost ceiling that correctly accounts for expensive output tokens and
discounted cache reads.

### Example: Anthropic Claude 3.5 Haiku on Bedrock

| Category | Price | Multiplier |
|---|---|---|
| Input (non-cached) | $0.80 / MTok | 1.00 (baseline) |
| Cache write | $1.00 / MTok | 1.25 |
| Cache read | $0.08 / MTok | 0.10 |
| Output | $4.00 / MTok | 5.00 |

Both models share the same ratios — only the absolute budget scale differs. Adjust
`requests_per_unit` on the ratelimit server to reflect the per-model baseline cost.

## Merge semantics

`tokenCosts` follows the same precedence rules as all other AI policy fields:

```
AgentgatewayPolicy (route) < AgentgatewayBackend
```

Backend-level `tokenCosts` override route-level ones. If a backend sets no `tokenCosts`,
the route's value is used as a fallback.

## Backward compatibility

- **No `tokenCosts` set** — all multipliers default to `1.0`, identical to pre-feature behaviour.
- **Partial object** (e.g. only `output` set) — unset fields default to `1.0`.
- Existing `requests_per_unit` values continue to work unchanged. Their semantic meaning
  shifts from "raw tokens" to "budget units" only once multipliers != 1.0 are configured.

## See also

- [`examples/ratelimiting/global/token-costs-backend.yaml`](../examples/ratelimiting/global/token-costs-backend.yaml) — annotated example
- [`examples/ratelimiting/global/README.md`](../examples/ratelimiting/global/README.md) — global rate limiting walkthrough
- [`architecture/configuration.md`](configuration.md) — full configuration reference
