# Benchmarking

The inference-perf Helm chart is adapted from the GIE benchmarking framework. See inference-perf/README.md for details.

More generally, the agentgateway benchmarking framework mirror the GIE benchmarking framework.

## Directory structure

```
benchmarking/
  run-benchmark.sh          # Orchestrates agentgateway + baseline runs
  download-results.sh       # Downloads results locally
  values.yaml               # Base inference-perf config
  inference-perf/           # Helm chart
  single-workload/          # Per-target overrides
    agentgateway-values.yaml
    baseline-values.yaml
```

## Prerequisites

- A running Kubernetes cluster with agentgateway deployed
- Following resources must exist in `NAMESPACE`:
  - `Gateway` resource (default: `inference-gateway`) exposing agentgateway on port 8080
  - `Service` named `llm-d-baseline` (override via `BASELINE_SVC`) exposing the baseline model server on port 80
- `helm` and `kubectl` available locally + configured against the cluster
- A GCS bucket for storing results (todo S3)

## Running a benchmark

```bash
WORKLOAD=single-workload \
NAMESPACE=<your-namespace> \
GCS_BUCKET=<your-bucket> \
./run-benchmark.sh
```

Auto-discovery work thanks to `kubectl get gateway` and `kubectl get svc` (can be overriden)

```bash
GW_URL=http://<ip>:8080 BASELINE_URL=http://<ip>:80 ... ./run-benchmark.sh
```

### GCS credentials

`KSA_NAME` on GKE (todo AWS):

```bash
KSA_NAME=my-ksa ... ./run-benchmark.sh
```

Create a GCP secret if outside GKE (todo AWS):

```bash
GCS_CREDENTIALS_SECRET=gcs-key GCS_PROJECT=<project-id> ... ./run-benchmark.sh
```

### Environment variables

| Variable | Description | Default |
|---|---|---|
| `NAMESPACE` | Kubernetes namespace | mandatory |
| `WORKLOAD` | Workload subdirectory (e.g. `single-workload`) | mandatory |
| `GW_NAME` | Gateway resource name | `inference-gateway` |
| `BASELINE_SVC` | Baseline service name | `llm-d-baseline` |
| `GW_URL` | Override gateway URL (skips discovery) | auto-discovered |
| `BASELINE_URL` | Override baseline URL (skips discovery) | auto-discovered |
| `GCS_BUCKET` | GCS bucket for results | optional |
| `GCS_CREDENTIALS_SECRET` | k8s secret name containing `key.json` | optional |
| `GCS_PROJECT` | GCP project ID (`GOOGLE_CLOUD_PROJECT` in pod) | optional |
| `KSA_NAME` | Kubernetes service account for Workload Identity | optional |
| `HF_TOKEN` | Hugging Face token | optional |
| `BENCHMARK_TIMEOUT` | kubectl wait timeout for the job | `3600s` |

## Downloading results

```bash
./download-results.sh <bucket> <timestamp> [workload]
```

Results are saved to `output/<timestamp>/agentgateway/` and `output/<timestamp>/baseline/`.

## Adding workload scenarios

Add a new subdirectory (e.g. `prefix-cache/`) with `agentgateway-values.yaml` and `baseline-values.yaml`, then run:

```bash
WORKLOAD=prefix-cache ... ./run-benchmark.sh
```
