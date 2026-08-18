# Inference report generation

`generate.py` converts suite-native stage results into a canonical metric model
and emits deterministic Markdown, PNG, and CSV comparisons. The
`llm-d-benchmark` adapter reads Benchmark Report v0.2 documents. Additional
suite adapters should preserve the same metric definitions instead of adding
suite-specific behavior to the renderers.

Install the pinned dependencies in a virtual environment or use
`make -C controller benchmark-report`, which manages
`benchmarking/inference/.venv` automatically.

Prism is an optional output format. When selected, the generator copies each
selected treatment's self-contained repetition bundles under `prism/` without
using Prism-normalized values as input to the Markdown report.
