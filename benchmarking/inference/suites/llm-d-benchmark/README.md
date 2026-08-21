# llm-d-benchmark suite adapter

This directory contains the scenario overlays, workload profiles, renderers,
and tests used by `benchmarking/inference/run-benchmark.sh` when
`BENCHMARK_SUITE=llm-d-benchmark`.

Execution and report rendering are intentionally separate. Suite scripts
capture native evidence and standardized Benchmark Report v0.2 stages;
`benchmarking/inference/reporting` turns completed treatments into comparison
reports.
