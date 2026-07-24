# Load Testing with k6

## Installation: Server Benchmark Tools

Benchmark is using [k6](https://k6.io/).

### Install k6 and SSE extension

SSE is not supported by default in k6, you have to build k6 with the xk6-sse extension.

```bash
go install go.k6.io/xk6@latest
./xk6 build latest --with github.com/phymbert/xk6-sse@v0.1.12
```

### Run the benchmark

```bash
SERVER_BENCH_MAX_TOKENS=128 ./k6 run bench_k6.js --duration 10m --iterations 500 --vus 8 --env scenario=generation_speed
```

### Scenarios

The script supports two benchmark scenarios, selected via the `SCENARIO` environment variable:

| Scenario | Executor | Description | Key Use Case |
|----------|----------|-------------|--------------|
| `prompt_ingestion_speed` | `per-vu-iterations` | Each VU processes one prompt from the dataset until all iterations are complete. Measures how fast the server can ingest and process prompt tokens. | Evaluating prompt processing throughput under a fixed workload. |
| `generation_speed` | `constant-vus` | VUs continuously send requests for the specified duration. Measures sustained throughput and generation performance under load. | Evaluating sustained load, token generation rate, and error rates over time. |

#### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SERVER_BENCH_URL` | `http://localhost:8080/v1` | Target agentgateway URL |
| `SERVER_BENCH_MODEL_ALIAS` | `example-model` | Model name to request |
| `SERVER_BENCH_DATASET` | `/agentgateway/examples/bench-k6-llm/datasets/prompts_length_128.json` | Path to JSON dataset of prompts |
| `SERVER_BENCH_MAX_TOKENS` | `1024` | Maximum number of tokens to generate |
| `SERVER_BENCH_MAX_PROMPT_TOKENS` | `1024` | Maximum prompt tokens to filter |
| `BEARER_TOKEN` | `your-api-key-here` | API key for authentication |
| `N_VU` | `1` | Number of concurrent virtual users |
| `MAX_DURATION` | `30s` | Maximum duration for `per-vu-iterations` scenario |
| `SCENARIO` | (all) | Select a specific scenario; omit to run both |

### Running examples

```bash
# Run only the prompt ingestion speed scenario (process all prompts once)
SERVER_BENCH_MAX_TOKENS=128 ./k6 run bench_k6.js --env scenario=prompt_ingestion_speed

# Run only the generation speed scenario for 1 minute
SERVER_BENCH_MAX_TOKENS=128 ./k6 run bench_k6.js --duration 1m --env scenario=generation_speed

# Run with custom parameters
SERVER_BENCH_URL="http://localhost:8080" SERVER_BENCH_MODEL_ALIAS="my-model" SERVER_BENCH_MAX_TOKENS=2048 BEARER_TOKEN="secret-token" ./k6 run bench_k6.js --duration 2m --vus 16 --env scenario=generation_speed
```

## Metrics

The following metrics are available, computed from the OAI chat completions response `usage`:

| Metric | Type | Description |
|--------|------|-------------|
| `tokens_second` | Trend | `usage.total_tokens / request duration` |
| `prompt_tokens` | Trend | `usage.prompt_tokens` per request |
| `prompt_tokens_total_counter` | Counter | Cumulative `usage.prompt_tokens` |
| `completion_tokens` | Trend | `usage.completion_tokens` per request |
| `completion_tokens_total_counter` | Counter | Cumulative `usage.completion_tokens` |
| `completions_truncated_rate` | Rate | Completions where `finish_reason === 'length'` |
| `completions_stop_rate` | Rate | Completions where `finish_reason === 'stop'` |

The script will fail if too many completions are truncated, see `k6_completions_truncated_rate`.

### Dataset

Prompts are loaded from a JSON file specified by the `SERVER_BENCH_DATASET` environment variable (default: `datasets/prompts_length_128.json`).

#### How the dataset is used

The benchmark script reads all prompts from the JSON file at startup into a single array. Each virtual user (VU) is assigned a slice of this array. During execution, each iteration picks one prompt from the assigned slice, sends it to the API via `POST /v1/chat/completions`, and records the response metrics (token counts, latency, finish reason).

| File | Description |
|------|-------------|
| `datasets/prompts_length_128.json` | Contains ~1000 French text prompts averaging ~128 tokens each (approximately 500–800 characters). These prompts are long enough to trigger meaningful token generation while keeping prompt ingestion consistent across iterations. |

#### Creating a custom dataset

To create a custom dataset, generate a JSON file containing an array of string prompts. Example:

```json
[
    "Write a detailed analysis of energy trends in 2026.",
    "Summarize the key findings from recent research."
]
```

Ensure the prompts match the expected length for your scenario:

| Scenario | Recommended prompt length |
|----------|--------------------------|
| `prompt_ingestion_speed` | Match the `SERVER_BENCH_MAX_PROMPT_TOKENS` limit (e.g., 128 tokens) to accurately measure prompt processing throughput. |
| `generation_speed` | Variable lengths are fine; the scenario focuses on generation throughput under sustained load. |
