# Benchmarks

Two suites measure conversion as the gateway calls it, on the same agent traffic. A third, through the whole
gateway, is still to do.

| Suite | Question | Run |
|---|---|---|
| [IR](conversion.rs) | What does the IR cost for each provider pair? | `cargo bench -p agent-llm-ir --bench conversion` |
| [Legacy versus IR](legacy_vs_ir.rs) | Is the IR faster or slower than the converters it replaces, on the same input? | `cargo bench -p agent-llm-ir --bench legacy_vs_ir` |
| Gateway | What does a request cost end to end? | [To do](#gateway-to-do) |

Both suites use Divan with allocation profiling. Setup checks every case's output before timing, so a case
that drops content fails rather than looking fast. Timed stream cases drop client output as it is written, as
a socket write would, so peak memory is what the converter holds rather than the whole reply.

## Workload

[`support/workload.rs`](support/workload.rs) builds the inputs for both suites:

- **Requests** are agent conversations in each client protocol: a system prompt, four tools, a question, then
  tool turns until the body reaches its size. Each turn is a `read_file` call and a result of about 5 KiB of
  code with quotes, escapes, tabs and some non-ASCII.
- **Replies** are prose followed by one `read_file` call. Streams send about one token per event, each in its
  own transport read, as providers flush them. Gemini streams end events with CRLF, as Google does.

The workload leaves out reasoning replay, images, parallel tool calls, provider errors and failures
mid-stream. The tests cover those for correctness, not cost.

## IR suite

Every pair of the four protocols, at three sizes:

| Bench | What is timed |
|---|---|
| `request` | `prepare` and `render` with model defaults, client bytes to provider bytes, for the 12 converted pairs. |
| `request_passthrough_read_only` | Reading the model and settings of a same-protocol request. Gateway edits are not included. |
| `request_edit_*` | One Messages-to-Responses request: visit every text, with no match or replacing one token, then render. |
| `request_representative_workload` | Messages-to-Responses at 64 KiB with standard, escape-heavy, schema-heavy, many-small-turn and reasoning-settings inputs. |
| `reply_buffered` | `BufferedReply`: provider body to client body, with usage. |
| `reply_stream` | `StreamReply`: one push per provider event, usage read and output taken after each. |
| `reply_buffered_rewrite_*` | One Messages-to-Responses reply rewrite, with no match or replacing one token. |
| `reply_stream_checks_release`, `reply_stream_rewrite_and_checks` | One Messages-to-Responses stream with 1 KiB check windows and 256-byte overlap, releasing every check, without or with a rewrite. |
| `reply_passthrough_buffered`, `reply_passthrough_stream` | `ObservedReply`: a same-protocol reply read only for usage. |

## Legacy versus IR

The nine pairs the legacy converters serve without Bedrock or Vertex: five conversions (`messages>responses`,
`messages>chat`, `chat>messages`, `chat>gemini`, `responses>chat`) and four passthroughs. Each side does what
the gateway does with it:

| | Legacy | IR |
|---|---|---|
| Converted request | Parse the typed request, translate, serialize. | `prepare`, then `render` with model defaults. |
| Same-protocol request | Parse the typed request and serialize it again. | Read the model and settings; send the client's bytes. |
| Converted reply | Translate, read usage, serialize. | `BufferedReply` or `StreamReply`. |
| Same-protocol reply | Parse, read usage, serialize again; or the passthrough stream reader. | `ObservedReply`; send the provider's bytes. |

Streams on both sides record usage through the same `StreamingUsageGuard` and are polled with a no-op waker,
so the time is conversion work. In the gateway, legacy streams also wake themselves after every provider
event, which costs a scheduler trip per event; this suite leaves that out.

### What this comparison does not show

- **Only pairs both sides serve.** Bedrock and Vertex are served only by legacy. Seven converted pairs are
  served only by the IR (`responses>messages`, `chat>responses`, and every Gemini pair except `chat>gemini`);
  the IR suite covers them.
- **Only traffic the IR takes.** Requests that need tokenizing, prompt guards that call a service, content
  capture, or Messages prompt caching on a passthrough stay on legacy. Neither suite measures those paths.
- **Validation differs.** The IR refuses repeated keys and unknown, repeated or out-of-order stream events,
  and checks streamed text against the final value. Legacy accepts more, so its time covers less work.
- **Converter work only.** Gateway work around the converters is left out on both sides: legacy's request
  preparation and whole-body model routing, and the IR's passthrough edits and final transformations.
- **The legacy passthrough is a model.** Parse-and-reserialize has not been checked against every
  same-protocol gateway path.
- **No reasoning.** Legacy carries replay state differently or not at all, so a fair case needs agreed
  behaviour first.

## Gateway (to do)

A suite through the whole proxy should answer what the converter suites cannot:

- Model routing: legacy parses the whole body; with the IR enabled the router reads only the model. The
  `model_router` bench in `agentgateway` (`-F internal_benches`) measures that read alone.
- Passthrough: legacy parses and writes the body again; the IR forwards it.
- Scheduling: legacy streams wake themselves once per provider event.
- Fallback: the cost of deciding a request stays on legacy.

It needs a mock provider that doesn't dominate the time, a multi-thread runtime as in production, gateway
logging off, and allocation counts from every thread, since Divan counts only the bench thread.

## Isolated components

`cargo bench -p agent-llm-ir --bench components` measures Chat tool-call output with 1, 16 and 128 calls, and
Messages-to-Gemini requests with 4, 32 and 128 tool schemas. It uses the system allocator, as the IR suite
does; the legacy comparison uses jemalloc, so compare allocations only within one harness.

`cargo bench -p agent-llm-ir --bench legacy_vs_ir -- sse::` compares `SseParser` with `tokio_sse_codec` on the
four provider streams, with one event per read, 7-byte reads and 4096-byte reads. Fragmented Gemini cases are
left out because `tokio_sse_codec` merges events split across a CRLF.

## Comparing results

Compare revisions on the same idle host with the same toolchain and build flags. Record the revision, host,
command and input sizes with published results. Raw runs are not kept here.
