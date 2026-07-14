# AgentGateway Benchmarking

This benchmarks agentgateway+EPP against a plain Kubernetes Service to see what overhead the routing layer adds. Both targets run the same mock model server and the same inference-perf workloads - the only difference is what's in front.

Follows the same approach as the upstream GIE benchmark.

## Prerequisites

- [Docker](https://docs.docker.com/)
- [Kind](https://kind.sigs.k8s.io/)
- [Helm 3+](https://helm.sh/)
- [kubectl](https://kubernetes.io/docs/reference/kubectl/)

## Running

```bash
chmod +x ./setup-benchmarks.sh
./setup-benchmarks.sh
```

Builds the agentgateway image, starts a Kind cluster, deploys both namespaces, runs the benchmark jobs, and pulls results into `output/default-run/`.

After that, generate the comparison charts:

```bash
python3 generate_plots.py
```

Saves `ttft_comparison.png`, `latency_comparison.png`, and `throughput_comparison.png` under `output/`.

To clean up: `kind delete cluster --name agentgateway-benchmark`

## What's in here

- `setup-benchmarks.sh` - runs everything: cluster, images, deployments, jobs, result collection
- `generate_plots.py` - parses results and plots the comparisons
- `benchmark.ipynb` - notebook version of the same plots
- `workloads/prefill-heavy-values.yaml` - long input, short output (TTFT-focused)
- `workloads/decode-heavy-values.yaml` - short input, long output (latency/throughput-focused)
- `inference-perf/` - Helm chart for the inference-perf load generator
