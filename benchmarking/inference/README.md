# Inference benchmarks

The inference benchmark tooling runs independently selectable treatments,
retains their native evidence in a shared campaign, and generates publishable
Markdown, PNG, and CSV comparisons. Prism-compatible Benchmark Report v0.2
documents remain available in each treatment's evidence bundle.

## Layout

```text
benchmarking/inference/
  run-benchmark.sh
  suites/
    llm-d-benchmark/
      scenarios/
      workloads/
      scripts/
      tests/
  reporting/
    adapters/
    common/
    tests/
    generate.py
  results/                 # ignored local and CI campaign data
  reports/                 # curated reports suitable for committing
```

`llm-d-benchmark` is currently the implemented execution suite. The suite
boundary allows the EPP performance harness or another inference benchmark to
be added without creating another top-level wrapper.

## Treatments and campaigns

Every invocation runs exactly one treatment. A campaign groups treatments and
repetitions whose inference backend, workload, and infrastructure must match.

| `BENCHMARK_TREATMENT` | Request path |
|---|---|
| `service` | Kubernetes Service directly to vLLM |
| `agentgateway-standalone` | agentgateway in the standalone EPP topology |
| `agentgateway-gateway` | agentgateway on Kubernetes with EPP |
| `envoy-standalone` | Envoy sidecar in the standalone EPP topology |

Required variables:

| Variable | Purpose |
|---|---|
| `BENCHMARK_TREATMENT` | One treatment from the table above |
| `BENCHMARK_CAMPAIGN_ID` | Shared lowercase identifier for comparable runs |

`BENCHMARK_REPETITION` defaults to `1`. Use `1`, `2`, and `3` for a
three-repetition publication campaign. The wrapper rejects a treatment whose
campaign identity differs from existing runs.

## Local simulator example

Run the Service treatment:

```bash
BENCHMARK_TREATMENT=service \
BENCHMARK_CAMPAIGN_ID=local-sim \
make -C controller benchmark
```

Then run standalone agentgateway with the same inputs:

```bash
BENCHMARK_TREATMENT=agentgateway-standalone \
BENCHMARK_CAMPAIGN_ID=local-sim \
make -C controller benchmark
```

The Kind provider is the default. It always imports the selected agentgateway
image archive into the cluster so a mutable local tag cannot silently reuse an
older node image.

## Published optimized-baseline profile

Use the upstream workload variant when producing a report comparable to the
llm-d Qwen3-32B/H100 optimized-baseline report. It preserves the Poisson load
ladder, 300-second request timeout, shared-prefix shape, and per-request output.

There is an important historical-version nuance. The published v0.9 report's
calibration matrix records vLLM v0.23.0, while the image overlay currently
reachable through the mutable v0.9 guide selects v0.26.0. Select
`BENCHMARK_REFERENCE_PROFILE=optimized-baseline-qwen3-32b-h100-v0.9` to pin
the report-era vLLM version, router/EPP v0.9.0, model-server command, resources,
and topology. The normal wrapper default remains vLLM v0.27.1.

The intended topology is:

- Qwen/Qwen3-32B
- vLLM v0.23.0 with the v0.9 reference profile (v0.27.1 otherwise)
- 8 decode replicas
- TP=2
- 16 H100 GPUs
- 6,000-token shared system prompt
- 1,200-token question
- 1,000-token requested output
- rates from 3 through 60 QPS

Common GKE settings for each treatment are:

```bash
BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_KUBE_CONTEXT=<context> \
BENCHMARK_ACCELERATOR_TYPE=gpu \
BENCHMARK_ACCELERATOR_MODEL=h100 \
BENCHMARK_BACKEND_TYPE=vllm \
BENCHMARK_SCENARIO=optimized-baseline \
BENCHMARK_ROUTING_POLICY=optimized-baseline \
BENCHMARK_REFERENCE_PROFILE=optimized-baseline-qwen3-32b-h100-v0.9 \
BENCHMARK_WORKLOAD_VARIANT=upstream \
BENCHMARK_REPLICAS=8 \
BENCHMARK_TENSOR_PARALLELISM=2 \
BENCHMARK_ENDPOINT_PATH=internal \
BENCHMARK_MODEL_STORAGE_PROFILE=high-throughput-shared \
BENCHMARK_WORKLOAD_STORAGE_PROFILE=shared \
BENCHMARK_GPU_RELEASE_POLICY=after-load \
BENCHMARK_CAMPAIGN_ID=<campaign-id> \
BENCHMARK_TREATMENT=<treatment> \
make -C controller benchmark
```

The reference profile requires agentgateway v1.4.1. The wrapper verifies the
observed vLLM, EPP, and agentgateway pod images before starting traffic.
`BENCHMARK_ENDPOINT_PATH=internal` explicitly supplies the proxy Service
ClusterIP to the harness; this prevents Gateway status from selecting an
external GKE load-balancer address. Use `external` only when the external path
is itself the subject of the experiment.

For a controlled model-server version comparison, use
`BENCHMARK_REFERENCE_PROFILE=optimized-baseline-qwen3-32b-h100-v0.9-vllm-v0.27.1`.
It preserves the historical profile's command, topology, EPP version, and
resources while changing only the vLLM image from v0.23.0 to v0.27.1.

`after-load` provides the largest accelerator cost reduction. The wrapper
stops new runtime-metric samples after inference-perf drains its final stage,
scales the model workloads and GPU node pool to zero, copies the completed
runtime metrics while report generation runs, and lets compressed result
collection, evidence packaging, and teardown continue on CPU nodes. GKE GPU
scenarios declare the NVIDIA accelerator resource explicitly so teardown does
not depend on nodes that were removed. Scale the pool back to the treatment's
required size before starting the next treatment. `after-run` waits for
llm-d-benchmark's run phase to return before releasing the pool, while `never`
leaves release to the campaign finalizer.

The wrapper consumes the stable lifecycle marker proposed in
[llm-d-benchmark#1796](https://github.com/llm-d/llm-d-benchmark/issues/1796).
Until that change is available in the selected upstream revision, it falls
back to inference-perf's final-stage log message. Every release writes
`gpu-release.yaml` into the treatment evidence with the detection source and
timestamps. Fast compressed result collection is enabled by default and can
be disabled with `BENCHMARK_FAST_COLLECT=false` for diagnosis.

Run these treatments for the two primary reports:

```text
service
agentgateway-standalone
agentgateway-gateway
```

For three repetitions, rotate treatment order to reduce cluster drift:

```text
round 1: service, agentgateway-standalone, agentgateway-gateway
round 2: agentgateway-gateway, service, agentgateway-standalone
round 3: agentgateway-standalone, agentgateway-gateway, service
```

Set `BENCHMARK_REPETITION` to the round number. Restarting model-server pods
between treatments clears runtime KV-cache state while the persistent model
weight cache can remain populated.

## CI duration and cost

Plan for approximately 6-7 hours and USD 400-425 for one complete GKE
campaign with the Service, standalone agentgateway, and agentgateway on
Kubernetes treatments at `BENCHMARK_REPETITION=1`. This estimate is based on
the `qwen3-32b-h100` model campaign in us-central1 with eight Spot `a3-highgpu-2g`
nodes, 16 H100 GPUs, and the `after-load` GPU release policy.

The measured campaign broke down as follows:

| Treatment | End-to-end time | GPU pool allocation |
|---|---:|---:|
| Kubernetes Service | About 2h05m | 82.5m |
| standalone agentgateway | About 2h10m | 92.4m |
| agentgateway on Kubernetes | About 1h40m | 60.0m |
| Final report generation | 5-10m | 0m |
| **Campaign total** | **About 6h-6h05m** | **About 3h55m** |

The GPU allocation intervals include node-pool provisioning and deletion
reconciliation. `after-load` releases the pool while inference-perf reporting,
result collection, compression, and teardown continue on CPU nodes. H100 Spot
capacity and model-download variability can increase the elapsed time, so CI
should allow at least seven hours for the workflow even though each individual
job should be shorter.

An indicative per-campaign cost breakdown is:

| Resource | Estimated cost (USD) |
|---|---:|
| Eight Spot `a3-highgpu-2g` nodes | 380-400 |
| Temporary Premium and Standard Filestore | 7-10 |
| `e2-standard-32` and `e2-standard-4` CPU nodes for six hours | About 7.25 |
| GKE management | At most 0.60 |
| Result-transfer network egress | 5-7 |
| GitHub 4-core larger runner for six hours | About 4.32 |
| **Estimated total** | **400-425** |

The estimate uses the us-central1 Spot price observed on 2026-08-13 of
USD 12.652385205 per hour for each two-H100 `a3-highgpu-2g` node, or about
USD 101.22 per hour for the eight-node pool. Prices are variable and this is
not a billing quote. See the Google Cloud
[Spot VM](https://cloud.google.com/spot-vms/pricing),
[Filestore](https://cloud.google.com/filestore/pricing), and
[GKE](https://cloud.google.com/kubernetes-engine/pricing) pricing pages.

A standard GitHub-hosted runner is not suitable for the entire campaign. A
single job is too close to GitHub's six-hour job limit, and each treatment
temporarily needs about 17 GiB of local space while collecting its raw
per-request results. Use sequential jobs for the three treatments followed by
a report-generation job, all protected by the same concurrency group. A
4-core larger Linux runner provides 150 GiB of storage; a self-hosted runner
with comparable free space is also sufficient. See GitHub's
[Actions limits](https://docs.github.com/en/actions/reference/limits),
[larger runner specifications](https://docs.github.com/en/actions/reference/runners/larger-runners),
and [runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing).

The three compressed per-request files total approximately 3 GiB. Store them
in durable object storage, such as GCS, rather than depending on the GitHub
Actions artifact quota. Reports and smaller evidence can remain workflow
artifacts. `BENCHMARK_REPETITION=3` runs every treatment three times and should
be budgeted at approximately 18 hours and USD 1,200 to 1,275.

## Results

New results use this campaign-oriented layout:

```text
benchmarking/inference/results/llm-d-benchmark/<campaign-id>/
  campaign-manifest.yaml
  .work/
  runs/
    service/
      repetition-1/
    agentgateway-standalone/
      repetition-1/
    agentgateway-gateway/
      repetition-1/
```

Each repetition contains standardized per-stage reports, native harness
configuration and summaries, rendered Kubernetes configuration, logs, and
runtime metrics when enabled. The upstream workload variant also retains
per-request results so percentiles can be independently audited.

## Generate Markdown and PNG reports

Generate the two Service comparisons after all treatments complete:

```bash
BENCHMARK_CAMPAIGN_DIR=../benchmarking/inference/results/llm-d-benchmark/<campaign-id> \
BENCHMARK_COMPARISONS='service:agentgateway-standalone service:agentgateway-gateway' \
make -C controller benchmark-report
```

The reporting target creates a dedicated Python virtual environment under
`benchmarking/inference/.venv` and installs pinned, non-interactive reporting
dependencies. Direct invocation is also supported after installing
`reporting/requirements.txt`:

```bash
python benchmarking/inference/reporting/generate.py \
  --campaign benchmarking/inference/results/llm-d-benchmark/<campaign-id> \
  --comparison service:agentgateway-standalone \
  --comparison service:agentgateway-gateway \
  --formats markdown,png,csv
```

Each comparison produces:

```text
README.md
metrics.csv
throughput_vs_qps.png
latency_vs_qps.png
ttft_p90_vs_qps.png
```

The generator excludes a duplicate-rate warm-up stage, takes the median point
across repetitions, and refuses to compare treatments with different QPS
ladders or repetition counts. Its Markdown tables and three PNGs follow the
upstream optimized-baseline report layout: input/output/total throughput,
mean TTFT/ITL/NTPOT, and TTFT p90. Failure counts and rates remain available in
`metrics.csv` so latency can be interpreted correctly under overload.

Use `--output benchmarking/inference/reports/<suite>/<profile>/<campaign-id>`
to promote reviewed output into the curated reports tree. For example:

```bash
python benchmarking/inference/reporting/generate.py \
  --campaign benchmarking/inference/results/llm-d-benchmark/<campaign-id> \
  --comparison service:agentgateway-standalone \
  --comparison service:agentgateway-gateway \
  --formats markdown,png,csv \
  --output benchmarking/inference/reports/llm-d-benchmark/optimized-baseline-qwen3-32b-h100/<campaign-id>
```

Generated native-evidence links resolve locally because `results/` contains
the source campaign. Raw results are intentionally ignored by Git. Before
committing a curated report, replace those local links with durable archive
URLs or describe the external archive without creating broken repository
links.

## Configuration highlights

| Variable | Default |
|---|---|
| `BENCHMARK_SUITE` | `llm-d-benchmark` |
| `BENCHMARK_CLUSTER_PROVIDER` | `kind` |
| `BENCHMARK_ACCELERATOR_TYPE` / `BENCHMARK_BACKEND_TYPE` | `sim` / `inference-sim` |
| `BENCHMARK_ROUTER_MODE` | Derived from treatment |
| `BENCHMARK_HARNESS` | `inference-perf` |
| `BENCHMARK_WORKLOAD_VARIANT` | `upstream` |
| `BENCHMARK_RUNTIME_METRICS` | `true` |
| `BENCHMARK_GKE_MONITORING` | `auto` |
| `BENCHMARK_FAST_COLLECT` | `true` |
| `BENCHMARK_GPU_RELEASE_POLICY` | `never` |
| `BENCHMARK_GKE_GPU_NODEPOOL` | `gpu-h100` |
| `BENCHMARK_MODEL_CACHE_POLICY` | `ephemeral` |

Provider-specific storage profiles use provider-prefixed low-level variables.
For example, the GKE high-throughput shared model profile resolves to
`premium-rwx`, while the shared harness profile resolves to `standard-rwx`.

List local and upstream scenarios with:

```bash
benchmarking/inference/run-benchmark.sh --list-scenarios
```

Remove only the managed llm-d-benchmark checkout and its virtual environment:

```bash
make -C controller benchmark-clean
```

This does not delete a Kubernetes cluster or a retained treatment.

## GKE campaign cleanup

GKE benchmark namespaces are labeled with their owning campaign. The wrapper
also acquires a cluster-wide campaign lock, so a different campaign cannot use
the same cluster until cleanup succeeds. With the default ephemeral cache
policy, treatment teardown explicitly deletes its PVCs and waits for their PVs
and Filestore instances to disappear.

CI must still run the campaign finalizer unconditionally. It deletes any
remaining resources owned by the campaign, verifies Filestore deletion, scales
the configured GPU node pool to zero, verifies its managed instance groups and
Kubernetes node count, and releases the campaign lock only after every check
passes:

```yaml
concurrency:
  group: inference-benchmark-gke-${{ vars.BENCHMARK_GKE_CLUSTER }}
  cancel-in-progress: false

steps:
  # Authentication, cluster credentials, scale-up, and benchmark steps go here.

  - name: Finalize GKE benchmark campaign
    if: ${{ always() }}
    env:
      BENCHMARK_CAMPAIGN_ID: ${{ env.BENCHMARK_CAMPAIGN_ID }}
      BENCHMARK_KUBE_CONTEXT: ${{ env.BENCHMARK_KUBE_CONTEXT }}
      BENCHMARK_GKE_PROJECT: ${{ vars.BENCHMARK_GKE_PROJECT }}
      BENCHMARK_GKE_LOCATION: ${{ vars.BENCHMARK_GKE_LOCATION }}
      BENCHMARK_GKE_CLUSTER: ${{ vars.BENCHMARK_GKE_CLUSTER }}
      BENCHMARK_GKE_GPU_NODEPOOL: ${{ vars.BENCHMARK_GKE_GPU_NODEPOOL }}
    run: make -C controller benchmark-gke-cleanup
```

The same cleanup can be invoked manually with those variables. The GPU node
pool defaults to `gpu-h100`; all other GKE identity values are required unless
they can be derived from a standard `gke_<project>_<location>_<cluster>` context
name. A cleanup failure is intentional: it prevents a subsequent campaign from
hiding leaked resources.
