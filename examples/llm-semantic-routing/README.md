# Cost-Based Semantic Routing with vLLM Semantic Router

This example adds an experimental path for routing OpenAI-compatible chat
traffic through [vLLM Semantic Router](https://vllm-semantic-router.com/)
before agentgateway forwards the request to OpenAI.

The target policy is:

- Send routine coding, refactoring, tests, docs, and simple debugging to
  `gpt-5.4-nano`.
- Send complex distributed systems design, formal verification, advanced
  debugging, and research synthesis to `gpt-5.5`.

Your existing agentgateway metrics, logs, traces, model catalog, and cost
tracking remain the source of truth. The scripts in this directory generate
controlled traffic and capture Semantic Router decision headers so you can join
routing decisions with agentgateway observability data.

## What This Adds

- `k8s/semantic-router-values.yaml`: Helm values for a two-model Semantic
  Router config.
- `k8s/agentgateway-experiment.yaml`: OpenAI backends for routed and forced
  baseline lanes, header-specific baseline `HTTPRoute`s, and an ExtProc policy
  attached only to the routed lane.
- `data/eval-corpus.jsonl`: labeled prompts for calibration and smoke testing.
- `scripts/run_eval.py`: runs `routed`, `always_low_cost`, and
  `always_expensive` lanes through the same gateway.
- `scripts/summarize_results.py`: summarizes local cost estimates, latency, and
  routing accuracy.
- `promql/queries.promql`: PromQL templates for agentgateway cost, token, and
  latency metrics.

## Before You Begin

This example assumes you already have a working agentgateway LLM path with
cost and observability data available:

- [Install agentgateway with Helm](https://agentgateway.dev/docs/kubernetes/main/install/helm/).
- [Set up an agentgateway proxy](https://agentgateway.dev/docs/kubernetes/main/setup/gateway/).
- [Configure OpenAI as an LLM provider](https://agentgateway.dev/docs/kubernetes/main/llm/providers/openai/).
- [Price LLM requests with a model cost catalog](https://agentgateway.dev/docs/kubernetes/main/llm/costs/).
- [Install an OpenTelemetry stack](https://agentgateway.dev/docs/kubernetes/main/observability/otel-stack/).

## Deploy

Install or update Semantic Router:

```bash
helm upgrade -i semantic-router oci://ghcr.io/vllm-project/charts/semantic-router \
  --version v0.0.0-latest \
  --namespace agentgateway-system \
  -f examples/llm-semantic-routing/k8s/semantic-router-values.yaml

kubectl wait --for=condition=Available deployment/semantic-router \
  -n agentgateway-system \
  --timeout=600s
```

Attach the experiment route and ExtProc policy:

```bash
kubectl apply -f examples/llm-semantic-routing/k8s/agentgateway-experiment.yaml

kubectl describe httproute openai-semantic-routing -n agentgateway-system
kubectl describe agentgatewaypolicy semantic-router-extproc -n agentgateway-system
```

Set the existing catch-all OpenAI backend to the expensive model so ad hoc
baseline traffic and dashboards compare against the same `always_expensive`
baseline:

```bash
kubectl patch agentgatewaybackend openai \
  -n agentgateway-system \
  --type=merge \
  -p '{"spec":{"ai":{"provider":{"openai":{"model":"gpt-5.5"}}}}}'
```

The existing `HTTPRoute/openai` can stay in place. The routed experiment lane
uses `openai-router-selected`, whose `openai.model` field is intentionally
omitted so Semantic Router can choose the model. The forced baseline lanes use
`X-Eval-Lane: always_low_cost` and `X-Eval-Lane: always_expensive` header
matches.

Before running the full corpus, confirm that the expensive model ID is accepted
by your target OpenAI endpoint. This example uses `gpt-5.4-nano` for the
low-cost lane and `gpt-5.5` for the expensive lane.

## Run

Set your gateway address:

```bash
export INGRESS_GW_ADDRESS="http://$(kubectl get gateway agentgateway-proxy \
  -n agentgateway-system \
  -o jsonpath='{.status.addresses[0].value}')"
```

Preview the plan without sending model traffic:

```bash
python3 examples/llm-semantic-routing/scripts/run_eval.py --limit 2 --dry-run
```

Run a small smoke test first:

```bash
python3 examples/llm-semantic-routing/scripts/run_eval.py \
  --limit 2 \
  --delay-sec 1
```

Then summarize the JSONL result file:

```bash
python3 examples/llm-semantic-routing/scripts/summarize_results.py \
  examples/llm-semantic-routing/results/<RUN_ID>.jsonl
```

Verify streamed ExtProc and immediate responses without calling OpenAI:

```bash
curl -i "$INGRESS_GW_ADDRESS/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -H "X-VSR-Debug: true" \
  -d '{
    "model": "auto",
    "messages": [
      {"role": "user", "content": "VSR_IMMEDIATE_RESPONSE_PROBE"}
    ],
    "max_tokens": 16
  }'
```

Look for a 200 response with `x-vsr-fast-response`. This request is matched by
a deliberately narrow probe decision and should not reach OpenAI.

Run the full synthetic corpus when the smoke test looks good:

```bash
python3 examples/llm-semantic-routing/scripts/run_eval.py \
  --delay-sec 1
```

Add `--capture-output` only when you want to collect answer text for human
satisfaction scoring.

## Measurement

Cost reduction:

- Use `agentgateway_gen_ai_client_cost_usd_total` from your model catalog setup
  as the cost-of-record.
- Compare the `routed` lane against `always_expensive`, which is the
  `gpt-5.5` counterfactual in this example.
- The runner also prints a local estimate from response usage as a quick
  cross-check.

Routing accuracy:

- The runner compares `x-vsr-selected-model` against each corpus row's
  `expected_model`.
- Review false negatives first: any hard prompt sent to `gpt-5.4-nano`.
- Then review false positives: routine work sent to `gpt-5.5`.

User satisfaction:

- Run with `--capture-output`.
- Fill in `data/ratings-template.csv` with `id,lane,satisfaction,right_model`.
- Pass it to the summarizer with `--ratings ratings.csv`.

Latency:

- The runner records client-side end-to-end latency.
- Use `promql/queries.promql` for agentgateway p95 route latency, LLM request
  duration, time to first token, and output-token throughput.

## Tuning

Start by tuning `advanced_need_band.outputs[].name: expensive_lane` in
`k8s/semantic-router-values.yaml`.

- Lower `gte: 0.35` if hard prompts are under-escalated.
- Raise it if routine prompts are over-escalated.
- Add better candidates under `advanced_reasoning_intent` or
  `routine_coding_intent` when errors are semantic rather than threshold-only.

This example uses agentgateway `requestBodyMode: FullDuplexStreamed` together
with `global.router.streamed_body.enabled: true`, so Semantic Router accumulates
streamed request-body chunks before applying the routing and immediate-response
pipeline. agentgateway does not support a separate `Streamed` mode.

## Cleanup

```bash
kubectl delete -f examples/llm-semantic-routing/k8s/agentgateway-experiment.yaml
helm uninstall semantic-router -n agentgateway-system
```
