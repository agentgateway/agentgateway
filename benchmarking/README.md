# Benchmarking

This compares agentgateway as EPP's proxy sidecar against a plain Kubernetes Service
baseline, using [llm-d-benchmark](https://github.com/llm-d/llm-d-benchmark) instead
of custom scripts we'd have to maintain ourselves. Both arms are decode-only right
now, no prefill - keeps it a fair comparison since the baseline can't do P/D
disaggregation at all.

See [agentgateway/agentgateway#85](https://github.com/agentgateway/agentgateway/issues/85)
and the [P/D disaggregation follow-up](https://github.com/agentgateway/agentgateway/issues/2809).

## TODO

The model server here is
[`llm-d-inference-sim`](https://github.com/llm-d/llm-d-inference-sim), a GPU-free
simulator. It's only there to check the workflow actually works end to end
(standup -> smoketest -> run -> compare). Swapping in a real model server on real
hardware is a separate follow-up, still TBD.

## What's in here

- `scenarios/baseline.yaml` - plain Service in front of the decode pod, no gateway,
  no EPP, no proxy (`gateway.className: none`)
- `scenarios/agentgateway.yaml` - EPP with agentgateway as the sidecar proxy, no
  Kubernetes Gateway (`gateway.className: epponly`, `router.proxy.proxyType: agentgateway`)
- `results/` - the comparison CSV and plots from the last run

## Running it yourself

You need `llm-d-benchmark` cloned somewhere and its CLI installed, check their
[quickstart](https://github.com/llm-d/llm-d-benchmark/blob/main/docs/quickstart.md)
if you haven't set it up before.

```bash
# point this at wherever you cloned llm-d-benchmark
export LLM_D_BENCHMARK_DIR=/path/to/llm-d-benchmark
export AGTW_BENCHMARKING_DIR=$(pwd)/benchmarking   # run from the agentgateway repo root

cd "$LLM_D_BENCHMARK_DIR"
source .venv/bin/activate
```

llm-d-benchmark needs a spec file pointing at its own templates plus our scenario
file, so write one for each arm:

```bash
for arm in baseline agentgateway; do
cat > /tmp/spec-${arm}.yaml <<EOF
base_dir: ${LLM_D_BENCHMARK_DIR}
values_file:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/values/defaults.yaml
template_dir:
  path: ${LLM_D_BENCHMARK_DIR}/config/templates/jinja
scenario_file:
  path: ${AGTW_BENCHMARKING_DIR}/scenarios/${arm}.yaml
EOF
done
```

Spin up a Kind cluster and run both arms:

```bash
kind create cluster --name agtw-benchmark

llmdbenchmark --spec /tmp/spec-baseline.yaml     standup -p gap2-baseline     --skip-smoketest
llmdbenchmark --spec /tmp/spec-agentgateway.yaml standup -p gap2-agentgateway --skip-smoketest

llmdbenchmark --spec /tmp/spec-baseline.yaml     smoketest -p gap2-baseline
llmdbenchmark --spec /tmp/spec-agentgateway.yaml smoketest -p gap2-agentgateway

llmdbenchmark --spec /tmp/spec-baseline.yaml     run -p gap2-baseline     -l inference-perf -w sanity_random.yaml
llmdbenchmark --spec /tmp/spec-agentgateway.yaml run -p gap2-agentgateway -l inference-perf -w sanity_random.yaml
```

Then compare. `cross_treatment.py` is just a library function, not a CLI command,
so this is a quick python snippet instead of one line:

```bash
mkdir -p /tmp/comparison-input
ln -sf <path-to-baseline-results>     /tmp/comparison-input/baseline
ln -sf <path-to-agentgateway-results> /tmp/comparison-input/agentgateway

python3 -c "
from pathlib import Path
from llmdbenchmark.analysis.cross_treatment import generate_cross_treatment_summary
generate_cross_treatment_summary(Path('/tmp/comparison-input'), output_dir=Path('/tmp/comparison-output'))
"
```

## Stuff that'll probably trip you up

- The router chart's built-in agentgateway image preset points at a tag
  (`cr.agentgateway.dev/agentgateway:v0.9.0`) that doesn't actually exist in the
  registry. `scenarios/agentgateway.yaml` already overrides it to `latest-dev`,
  just make sure that image is loaded into your cluster
  (`kind load docker-image cr.agentgateway.dev/agentgateway:latest-dev --name <cluster>`).
- First standup on a fresh cluster times out waiting on the harness pod - it's
  pulling `ghcr.io/llm-d/llm-d-benchmark` (~5.7GB) for the first time and that
  takes longer than the wait timeout. Just let the pull finish and re-run
  `standup`, it'll use the cached image the second time.
