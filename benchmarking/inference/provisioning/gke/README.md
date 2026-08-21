# GKE benchmark provisioning

This directory contains the `gcloud`-based infrastructure lifecycle for GKE
inference benchmarks. Provisioning is deliberately separate from
`run-benchmark.sh`: benchmark retries cannot create, replace, or destroy cloud
infrastructure.

The scripts are non-interactive. They never install or update command-line
tools and fail immediately when `gcloud`, `kubectl`, or
`gke-gcloud-auth-plugin` is unavailable. Every cloud mutation disables prompts
and passes `--quiet`.

## Reference profile

The defaults reproduce the infrastructure used for the Qwen3-32B optimized
baseline campaigns:

- zonal GKE Standard cluster in `us-central1-a`;
- one `e2-standard-4` system node;
- one `e2-standard-32` harness node;
- a zero-sized Spot GPU pool that scales to eight `a3-highgpu-2g` nodes;
- two `nvidia-h100-80gb` accelerators per GPU node; and
- Premium and Standard Filestore CSI storage classes.

The GPU machine and accelerator are inputs rather than assumptions. Changing
them also changes hardware validation and the allocatable accelerator count
used by the scale-up readiness check.

## Usage

An authenticated principal with permission to enable services and create GKE
and Compute resources is an external prerequisite. Project IAM is a separate
bootstrap boundary: the provisioner neither creates service accounts nor
changes IAM bindings.

By default, an IAM administrator must create
`agentgateway-benchmark-nodes@<project>.iam.gserviceaccount.com`, grant it
`roles/container.defaultNodeServiceAccount` and
`roles/artifactregistry.reader`, and grant the human or CI provisioning
principal `roles/iam.serviceAccountUser` on that account. The `plan` and
`apply` operations validate the node account and its project roles before
creating GKE resources.

For compatibility with an existing project that intentionally uses its
Compute Engine default service account, opt in explicitly:

```bash
BENCHMARK_GKE_NODE_SERVICE_ACCOUNT=default \
BENCHMARK_GKE_PROJECT=<project> \
make -C controller benchmark-gke-provision
```

The default Compute Engine account is often broadly privileged, so a dedicated
account is preferred for CI.

Set the project explicitly. Review the plan before applying it:

```bash
export BENCHMARK_GKE_PROJECT=<project>

make -C controller benchmark-gke-plan
make -C controller benchmark-gke-provision
```

Successful verification prints the provider, context, node-pool, and hardware
exports consumed by `run-benchmark.sh`. It also writes an observed provisioning
record under `benchmarking/inference/results/provisioning/gke/` by default.

Model credentials remain benchmark inputs rather than infrastructure. Before a
GPU treatment, create the configured Hugging Face Secret in the dedicated
`benchmark-secrets` namespace; the provisioner never accepts or stores API
tokens and never creates service account keys.

Provisioning creates the GPU node pool at zero nodes. Scale it only when a
campaign is ready to start:

```bash
make -C controller benchmark-gke-gpu-up
```

The scale command waits for every node and the expected number of allocatable
accelerators. If scale-up is rejected or does not complete before
`BENCHMARK_GKE_GPU_READY_TIMEOUT`, it requests a rollback to zero and fails.

Release GPU capacity explicitly or through the campaign finalizer:

```bash
make -C controller benchmark-gke-gpu-down
```

Destroying the persistent infrastructure is a separate guarded operation. It
refuses to run while benchmark namespaces, persistent volumes, or LoadBalancer
Services remain:

```bash
make -C controller benchmark-gke-destroy
```

The explicitly destructive Make target supplies the confirmation guard. A
direct invocation of `destroy.sh` still requires
`BENCHMARK_GKE_ALLOW_DESTROY=true`.

The project APIs, IAM bindings, service accounts, network, and subnetwork are
treated as shared project infrastructure and are not removed.

The complete three-treatment workflow can use the provisioner as ephemeral CI
infrastructure. In this mode its unconditional finalizer cleans campaign cloud
storage, scales down the GPU pool, and deletes the cluster and its CPU and GPU
node pools:

```bash
BENCHMARK_GKE_PROJECT=<project> \
BENCHMARK_GKE_CLUSTER_LIFECYCLE=destroy \
make -C controller benchmark-gke-all
```

`retain` is the default lifecycle. Cluster destruction is never inferred from
CI detection; it must be selected explicitly.

## Configuration

| Variable | Default |
|---|---|
| `BENCHMARK_GKE_PROJECT` | Required |
| `BENCHMARK_GKE_LOCATION` | `us-central1-a` |
| `BENCHMARK_GKE_CLUSTER` | `agentgateway-benchmark` |
| `BENCHMARK_GKE_NETWORK` / `BENCHMARK_GKE_SUBNETWORK` | `default` / `default` |
| `BENCHMARK_GKE_RELEASE_CHANNEL` | `stable` |
| `BENCHMARK_GKE_SYSTEM_NODEPOOL` | `default-pool` |
| `BENCHMARK_GKE_SYSTEM_MACHINE_TYPE` / `BENCHMARK_GKE_SYSTEM_NODES` | `e2-standard-4` / `1` |
| `BENCHMARK_GKE_HARNESS_NODEPOOL` | `bench-cpu` |
| `BENCHMARK_GKE_HARNESS_MACHINE_TYPE` / `BENCHMARK_GKE_HARNESS_NODES` | `e2-standard-32` / `1` |
| `BENCHMARK_GKE_GPU_NODEPOOL` | `gpu-h100` |
| `BENCHMARK_GKE_GPU_MACHINE_TYPE` | `a3-highgpu-2g` |
| `BENCHMARK_GKE_GPU_ACCELERATOR_TYPE` | `nvidia-h100-80gb` |
| `BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE` | `2` |
| `BENCHMARK_GKE_GPU_TARGET_NODES` | `8` |
| `BENCHMARK_GKE_GPU_DRIVER_VERSION` | `default` |
| `BENCHMARK_GKE_GPU_SPOT` | `true` |
| `BENCHMARK_GKE_GPU_READY_TIMEOUT` | `2400` seconds |
| `BENCHMARK_GKE_NODE_SERVICE_ACCOUNT` | Dedicated account in the selected project; `default` is an explicit compatibility option |

The current implementation provisions zonal clusters. This keeps the GPU
capacity request in one selected zone and prevents a regional node count from
being multiplied across zones.

## Reconciliation and ownership

Resources are labeled `managed_by=agentgateway-benchmark` and
`purpose=inference-benchmark`. An existing cluster without both labels is
never adopted. Compatible resources are reused, safe add-ons are reconciled,
and incompatible node-pool hardware causes a failure instead of an automatic
replacement.

The provisioner enables the Compute Engine, GKE, Filestore, IAM, and Monitoring
APIs but does not disable them during destruction. It validates that the
configured machine and accelerator types are offered in the selected zone;
quota and physical Spot capacity are ultimately verified by the real scale-up
operation.
