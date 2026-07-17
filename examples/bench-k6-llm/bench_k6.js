// k6 load testing script for agentgateway
// Tests the OpenAI-compatible /chat/completions endpoint with SSE streaming
// Requires xk6-sse extension:
//   go install go.k6.io/xk6@latest
//   ./xk6 build latest --with github.com/phymbert/xk6-sse@v0.1.12

import sse from 'k6/x/sse';          // SSE extension for streaming responses
import {check, sleep} from 'k6';      // Validation checks and delays
import {SharedArray} from 'k6/data';  // Shared data loaded once for all VUs
import {Counter, Rate, Trend} from 'k6/metrics';  // Custom metrics
import exec from 'k6/execution';      // Scenario and iteration context

// ── Configuration ──────────────────────────────────────────────────────────
// All values can be overridden via environment variables (passed with --env)

// Target server URL — base path for the API endpoint
const server_url = __ENV.SERVER_BENCH_URL
    ? __ENV.SERVER_BENCH_URL
    : 'http://localhost:8080/api_vllm/rs/v1';

// Model name to target in requests
const model = __ENV.SERVER_BENCH_MODEL_ALIAS
    ? __ENV.SERVER_BENCH_MODEL_ALIAS
    : 'example-model';

// Path to JSON dataset of prompts to send
const dataset_path = __ENV.SERVER_BENCH_DATASET
    ? __ENV.SERVER_BENCH_DATASET
    : '/home/imfo9020/bench-k6-llm/datasets/prompts_length_128.json';

// Maximum number of tokens the model should generate
const max_tokens = __ENV.SERVER_BENCH_MAX_TOKENS
    ? parseInt(__ENV.SERVER_BENCH_MAX_TOKENS)
    : 1024;

// Maximum prompt tokens to use as a filter criterion
const n_prompt_tokens = __ENV.SERVER_BENCH_MAX_PROMPT_TOKENS
    ? parseInt(__ENV.SERVER_BENCH_MAX_PROMPT_TOKENS)
    : 1024;

// Bearer token for API authentication
const bearer_token = __ENV.BEARER_TOKEN
    ? __ENV.BEARER_TOKEN
    : 'your-api-key-here';

// Number of concurrent virtual users
const n_vu = __ENV.N_VU
    ? parseInt(__ENV.N_VU)
    : 1;

// Maximum duration for the per-vu-iterations scenario
const max_duration = __ENV.MAX_DURATION
    ? __ENV.MAX_DURATION
    : '30s';

// ── Shared dataset loading ─────────────────────────────────────────────────
// SharedArray reads and parses the JSON dataset exactly once at setup,
// then makes it available to all VUs (avoids re-parsing per iteration).
// Each entry is mapped to { prompt: data } for simple access in the loop.
const data = new SharedArray('conversations', function () {
    return JSON.parse(open(dataset_path))
        .map(data => ({ prompt: data }));
});

// ── Custom metrics ─────────────────────────────────────────────────────────

// Per-request trends: token counts and throughput
const k6_prompt_tokens = new Trend('k6_prompt_tokens');
const k6_completion_tokens = new Trend('k6_completion_tokens');
const k6_tokens_second = new Trend('k6_tokens_second');
const k6_prompt_processing_second = new Trend('k6_prompt_processing_second');
const k6_emit_first_token_second = new Trend('k6_emit_first_token_second');

// Cumulative counters: total tokens across all requests
const k6_prompt_tokens_total_counter = new Counter('k6_prompt_tokens_total_counter');
const k6_completion_tokens_total_counter = new Counter('k6_completion_tokens_total_counter');

// Rate metrics: completion finish reasons (used for pass/fail)
const k6_completions_truncated_rate = new Rate('k6_completions_truncated_rate');  // true if finish_reason === 'length'
const k6_completions_stop_rate = new Rate('k6_completions_stop_rate');            // true if finish_reason === 'stop'

// Counters for tok-i and tok-o SSE events — custom events from vLLM/Ollama streaming
// tok-i (Token Input): server sends the cumulative count of processed prompt tokens
// tok-o (Token Output): server sends the cumulative count of generated completion tokens
// These provide finer-grained, earlier token metrics than the JSON 'usage' field which
// only arrives at the end of generation. Used as fallback when usage is unavailable.
const k6_tok_i_counter = new Counter('k6_tok_i_counter');
const k6_tok_o_counter = new Counter('k6_tok_o_counter');

// HTTP error counters grouped by status code
const k6_http_errors = new Counter('k6_http_errors');
const k6_http_200 = new Counter('k6_http_200');
const k6_http_400 = new Counter('k6_http_400');
const k6_http_401 = new Counter('k6_http_401');
const k6_http_403 = new Counter('k6_http_403');
const k6_http_404 = new Counter('k6_http_404');
const k6_http_429 = new Counter('k6_http_429');
const k6_http_500 = new Counter('k6_http_500');
const k6_http_502 = new Counter('k6_http_502');
const k6_http_503 = new Counter('k6_http_503');
const k6_http_504 = new Counter('k6_http_504');
const k6_http_other = new Counter('k6_http_other');

// ── Scenario definitions ───────────────────────────────────────────────────
// Two benchmark scenarios are defined; only one runs at a time based on SCENARIO env var.
let scenarios = {
    // per-vu-iterations: each VU processes a fixed number of iterations (one per prompt),
    // then stops. maxDuration caps how long the scenario runs.
    prompt_ingestion_speed: {
        executor: 'per-vu-iterations',
        vus: n_vu,
        iterations: data.length,
        maxDuration: max_duration,
    },
    // constant-vus: a fixed number of VUs continuously send requests for the specified duration.
    generation_speed: {
        executor: 'constant-vus',
        vus: n_vu,
        duration: max_duration,
    },
};

// Exported k6 options — only the selected scenario is enabled.
export const options = {
    scenarios: {},
};

if (__ENV.SCENARIO) {
    // Single scenario mode: run only the scenario specified via --env scenario=X
    options.scenarios[__ENV.SCENARIO] = scenarios[__ENV.SCENARIO];
} else {
    // All scenarios mode: run both scenarios sequentially
    options.scenarios = scenarios;
}

// ── Setup ──────────────────────────────────────────────────────────────────
// Runs once before any VU starts. Used to log configuration and which scenarios will run.
export function setup() {
    console.info(
        `Benchmark config: server_url=${server_url} ` +
        `n_prompt_tokens=${n_prompt_tokens} model=${model} ` +
        `dataset_path=${dataset_path} max_tokens=${max_tokens}`
    );
    if (__ENV.SCENARIO) {
        console.log('Running scenario:', __ENV.SCENARIO);
    } else {
        console.log('Running all scenarios');
    }
}

// ── Main test loop ─────────────────────────────────────────────────────────
// Each VU calls this function per iteration.
// 1. Selects a prompt from the shared dataset
// 2. Sends a POST /chat/completions request with SSE streaming enabled
// 3. Parses incoming SSE events to extract token counts and finish reasons
// 4. Records custom k6 metrics
export default function () {
    // Pick a prompt from the dataset (round-robin across VUs via iteration index)
    const conversation = data[exec.scenario.iterationInInstance % data.length];

    // Build the OpenAI-compatible chat completions payload
    const payload = {
        messages: [
            { role: 'system', content: 'You are ChatGPT, an AI assistant.' },
            { role: 'user', content: conversation.prompt },
        ],
        model: model,
        stream: true,
        max_tokens: max_tokens,
    };

    // HTTP request parameters: POST with JSON body and auth header
    const params = {
        method: 'POST',
        body: JSON.stringify(payload),
        headers: {
            Authorization: `Bearer ${bearer_token}`,
            accept: 'application/json',
            'content-type': 'application/json',
        },
    };

    // ── Timing and state variables (per-request) ───────────────────────────
    const startTime = new Date();             // Request start timestamp
    let promptEvalEndTime = null;             // Time when first token is received
    let prompt_tokens = 0;                    // Prompt tokens from usage field
    let completions_tokens = 0;               // Completion tokens from usage field
    let finish_reason = null;                 // 'stop', 'length', etc.
    let tok_i_count = 0;                      // Cumulative tok-i SSE event counts
    let tok_o_count = 0;                      // Cumulative tok-o SSE event counts

    // ── Open SSE connection ────────────────────────────────────────────────
    // sse.open sends the request and establishes a streaming connection.
    // The callback receives a client object to register event handlers.
    const res = sse.open(`${server_url}/chat/completions`, params, function (client) {
        // ── Standard SSE data events ─────────────────────────────────────
        // Received for every chunk in the streaming response.
        client.on('event', function (event) {
            // Record time to first token (TTFT) on the first event
            if (promptEvalEndTime == null) {
                promptEvalEndTime = new Date();
                k6_emit_first_token_second.add((promptEvalEndTime - startTime) / 1e3);
            }

            // SSE ends with '[DONE]' or empty data — stop listening
            if (event.data === '[DONE]' || event.data === '') {
                return;
            }

            // Parse the JSON chunk and extract token usage / finish reason
            try {
                let chunk = JSON.parse(event.data);

                // Extract finish_reason from the choice if present
                if (chunk.choices && chunk.choices.length > 0) {
                    let choice = chunk.choices[0];
                    if (choice.finish_reason) {
                        finish_reason = choice.finish_reason;
                    }
                }

                // Extract token usage counts from the chunk
                if (chunk.usage) {
                    prompt_tokens = chunk.usage.prompt_tokens;
                    k6_prompt_tokens.add(prompt_tokens);
                    k6_prompt_tokens_total_counter.add(prompt_tokens);

                    completions_tokens = chunk.usage.completion_tokens;
                    k6_completion_tokens.add(completions_tokens);
                    k6_completion_tokens_total_counter.add(completions_tokens);
                }
            } catch (e) {
                // Ignore parsing errors for non-JSON or malformed chunks
            }
        });

        // ── tok-i events (input/prompt token counts) ─────────────────────
        // Custom SSE event from vLLM/Ollama servers carrying cumulative
        // prompt token counts. Fires after each input token is processed.
        // Also used to record time-to-first-token if not set by 'event' handler.
        client.on('tok-i', function (event) {
            if (promptEvalEndTime == null) {
                promptEvalEndTime = new Date();
                k6_emit_first_token_second.add((promptEvalEndTime - startTime) / 1e3);
            }

            try {
                const count = parseInt(event.data);
                if (!isNaN(count)) {
                    tok_i_count += count;
                    k6_tok_i_counter.add(count);
                }
            } catch (e) {
                // Ignore parse errors
            }
        });

        // ── tok-o events (output/completion token counts) ────────────────
        // Custom SSE event from vLLM/Ollama servers carrying cumulative
        // completion token counts. Fires after each output token is generated.
        client.on('tok-o', function (event) {
            try {
                const count = parseInt(event.data);
                if (!isNaN(count)) {
                    tok_o_count += count;
                    k6_tok_o_counter.add(count);
                }
            } catch (e) {
                // Ignore parse errors
            }
        });

        // ── Error handler ────────────────────────────────────────────────
        // Catches errors from the SSE connection.
        // tok-i and tok-o are expected events (not from standard SSE),
        // so we suppress their "unknown event" errors from logging.
        client.on('error', function (e) {
            const errorMsg = e.error();
            if (
                !errorMsg.includes('unknown event: tok-i') &&
                !errorMsg.includes('unknown event: tok-o')
            ) {
                console.log('An unexpected error occurred: ', errorMsg);
            }
        });
    });

    // ── Post-request metric recording ──────────────────────────────────────

    // Basic HTTP success check
    check(res, { 'success completion': (r) => r.status === 200 });

    // Count HTTP response status codes for error analysis
    const statusCode = res.status;
    if (statusCode === 200) {
        k6_http_200.add(1);
    } else {
        k6_http_errors.add(1);
        switch (statusCode) {
            case 400: k6_http_400.add(1); break;
            case 401: k6_http_401.add(1); break;
            case 403: k6_http_403.add(1); break;
            case 404: k6_http_404.add(1); break;
            case 429: k6_http_429.add(1); break;
            case 500: k6_http_500.add(1); break;
            case 502: k6_http_502.add(1); break;
            case 503: k6_http_503.add(1); break;
            case 504: k6_http_504.add(1); break;
            default:
                k6_http_other.add(1);
                console.log(`Unexpected HTTP status code: ${statusCode}`);
        }
    }

    const endTime = new Date();

    // Fallback to tok_i/tok_o counts if usage field is not available
    const actual_prompt_tokens = prompt_tokens > 0 ? prompt_tokens : tok_i_count;
    const actual_completion_tokens = completions_tokens > 0 ? completions_tokens : tok_o_count;

    // Prompt processing speed: tokens per second during prompt evaluation
    const promptEvalTime = promptEvalEndTime - startTime;
    if (promptEvalTime > 0 && actual_prompt_tokens > 0) {
        k6_prompt_processing_second.add(
            actual_prompt_tokens / (promptEvalEndTime - startTime) * 1e3
        );
    }

    // Generation speed: tokens per second during text generation
    const completionTime = endTime - promptEvalEndTime;
    if (actual_completion_tokens > 0 && completionTime > 0) {
        k6_tokens_second.add(actual_completion_tokens / completionTime * 1e3);
    }

    // Record finish reason rates (used for pass/fail thresholds)
    k6_completions_truncated_rate.add(finish_reason === 'length');
    k6_completions_stop_rate.add(finish_reason === 'stop');

    // Small delay between iterations to avoid hammering the server
    sleep(0.3);
}
