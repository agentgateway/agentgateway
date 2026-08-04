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

Results aren't checked in here - running the steps below produces a `results/`
directory locally (gitignored), or check the PR description for the latest numbers.

## Running it yourself

```bash
make -C controller benchmark BENCHMARK_LLM_D_BENCHMARK_DIR=/path/to/llm-d-benchmark
```

That's it - it reuses the existing `kind-create` target for the cluster, so pass
`CLUSTER_NAME=<name>` too if you don't want the default. You need `llm-d-benchmark`
cloned somewhere with its CLI installed first, check their
[quickstart](https://github.com/llm-d/llm-d-benchmark/blob/main/docs/quickstart.md)
if you haven't set it up before.

You can also run `benchmarking/run-benchmark.sh` directly (same env vars, just
`LLM_D_BENCHMARK_DIR` instead of the `BENCHMARK_` prefix) if you already have a
cluster up and don't want to go through `make`.

What it does, in order:
1. Loads the agentgateway image into the cluster (see the gotcha below for why
   this isn't just `kind load docker-image`)
2. Writes a spec file per arm pointing llm-d-benchmark at its own templates plus
   our `scenarios/*.yaml`
3. `standup` -> `smoketest` -> `run` for both arms, retrying `standup` once if it
   times out on the first image pull (see the gotcha below)
4. Compares both arms with llm-d-benchmark's own `cross_treatment.py` and writes
   the CSV + plots to `results/` (gitignored, not checked in)

## Stuff that'll probably trip you up

- The router chart's built-in agentgateway image preset points at a tag
  (`cr.agentgateway.dev/agentgateway:v0.9.0`) that doesn't actually exist in the
  registry. `scenarios/agentgateway.yaml` already overrides it to `latest-dev`.
- Loading that image with a plain `kind load docker-image` fails with
  `ctr: content digest ... not found`. `cr.agentgateway.dev` publishes a
  multi-arch index with buildx attestation manifests, and on Docker Desktop's
  containerd image store, `docker save` keeps the whole index without the
  content for platforms/attestations you never actually pulled - `kind`'s
  `ctr images import --all-platforms` then chokes on the missing digest. The
  script works around this with `skopeo` (`brew install skopeo`), which
  flattens the image to a single-platform tar before `kind load image-archive`.
- First standup on a fresh cluster times out waiting on the harness pod - it's
  pulling `ghcr.io/llm-d/llm-d-benchmark` (~5.7GB) for the first time and that
  takes longer than the wait timeout. The script retries once after waiting for
  the namespace's pods to go Ready; if that's still not enough, let the pull
  finish and re-run.
