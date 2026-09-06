## Concurrency Limiting Example

This example bounds how many requests are in flight at once, per caller and per model.

A rate limit counts the requests that started in a window. A `concurrencyLimit` counts the ones
that are running right now, which is what protects a fixed pool of GPU slots from a single
runaway client: a slot is taken when the request is admitted and given back once its response
has been sent, or when the client goes away.

Every rule has to admit the request, and `key` is a CEL expression that selects the counter, so
one policy can hold a listener, a caller, and a caller-and-model pair to different numbers:

```yaml
concurrencyLimit:
- maxConcurrent: 32
- maxConcurrent: 4
  key: jwt.sub
- maxConcurrent: 2
  key: jwt.sub + "/" + llm.requestModel
  limitOverride: >-
    jwt.team == "research" ? (llm.requestModel == "qwen3-research" ? 8 : 0) : 2
```

`limitOverride` computes the limit for the request in place of `maxConcurrent`, so a team can be
given more slots on its own model and none on the others. The counter is still the one `key`
selects; only the number it is compared with changes.

### Running the example

Point the two models at OpenAI-compatible endpoints, and start agentgateway:

```bash
cargo run -- -f examples/traffic-concurrency-limit/config.yaml
```

Requests over the limit are rejected with a `429` while the ones ahead of them are still running,
and go through as soon as a slot is free.

### Sharing the slots across instances

Counters are local to one proxy instance, so with N replicas the effective cap is N times the
number configured. A rule can instead keep its slots in Redis:

```yaml
- maxConcurrent: 16
  key: jwt.team
  shared:
    redis:
      url: redis://redis.internal:6379/0
    lease: 60s
```

Rules with the same settings and prefix count together, which is what makes two replicas of one
gateway share. A slot outlives its request only when the instance holding it went away, and then
only until its lease runs out. When the store cannot be reached, `failureMode: allow`, the
default, lets the request through without a slot, and `deny` rejects it.
