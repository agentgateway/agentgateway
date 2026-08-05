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
make -C controller benchmark
```

That's it - no pre-setup needed. It reuses the existing `kind-create` target for
the cluster (pass `CLUSTER_NAME=<name>` if you don't want the default), and
clones/manages its own `llm-d-benchmark` checkout automatically (see below).

To clean up that managed clone later:

```bash
make -C controller benchmark-clean
```

This doesn't touch the kind cluster or anything deployed to it, just the local
`llm-d-benchmark` clone and its venv.

If you'd rather use your own existing `llm-d-benchmark` clone instead of the
managed one, pass `BENCHMARK_LLM_D_BENCHMARK_DIR=/path/to/llm-d-benchmark` to
either target - `benchmark-clean` then becomes a no-op, since that clone isn't
ours to delete.

You can also run `benchmarking/run-benchmark.sh` directly (same idea, just
`LLM_D_BENCHMARK_DIR` instead of the `BENCHMARK_` prefix, and `--clean` instead
of a separate make target) if you already have a cluster up and don't want to
go through `make`.

What it does, in order:
1. Clones (or reuses/updates a cached clone of) `llm-d-benchmark` and installs
   it via their own `install.sh`, unless `LLM_D_BENCHMARK_DIR` is set
2. Loads the agentgateway image into the cluster (see the gotcha below for why
   this isn't just `kind load docker-image`)
3. Writes a spec file per arm pointing llm-d-benchmark at its own templates plus
   our `scenarios/*.yaml`
4. `standup` -> `smoketest` -> `run` for both arms, with a longer timeout for
   the first image pull (see the gotcha below)
5. Compares both arms with llm-d-benchmark's own `cross_treatment.py` and writes
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
- First standup on a fresh cluster can be slow waiting on the harness pod -
  it's pulling `ghcr.io/llm-d/llm-d-benchmark` (~5.7GB) for the first time,
  which can take longer than the default 120s wait. The script passes
  `--data-access-timeout 600` to standup for this (needs
  llm-d-benchmark#1696 or later - `git pull` if you're on an older clone
  and hit this).
- If you're on Helm 4, `helmfile apply` fails with `if any flags in the group
  [validate dry-run] are set none of the others can be`. That's the installed
  `helm-diff` plugin passing both `--validate` and `--dry-run`, which Helm 4
  now rejects as mutually exclusive. Downgrade to Helm 3
  (`brew install helm@3 && brew unlink helm && brew link helm@3`) until
  `helm-diff` catches up.
