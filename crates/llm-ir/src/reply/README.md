# Driving replies

The reply drivers convert a reply from any supported provider protocol into any supported client protocol.
They are synchronous and do no I/O: the caller owns transport, backpressure, timeouts and any asynchronous
decision.

| Driver | For |
|---|---|
| `BufferedReply` | One complete provider JSON body. |
| `StreamReply` | A provider SSE stream, chunk by chunk. |
| `error_reply` | A provider response with a non-success status. |
| `ObservedReply` | A [passthrough reply](#passthrough-replies), read only for usage. |

## Reply context

Build one `ReplyContext` per provider attempt. It selects the decoder and the writer and carries what request
conversion decided.

| Field | Purpose |
|---|---|
| `client`, `upstream` | Choose the client writer and the provider decoder. |
| `scope` | The destination recorded in reasoning state the client carries to its next request. |
| `routes` | Restore tool names and call kinds flattened in the request. |
| `show_reasoning` | Show reasoning summaries when the client asked for them. |
| `include_usage` | Add final Chat Completions stream usage when requested. |
| `created` | The client's timestamp, where its protocol has one. |
| `fallback_reply_id` | Names a reply whose provider sent no id. Unique per attempt, because generated tool-call ids derive from it. |
| `stop_sequences` | Sequences the provider could not enforce, applied to client output. |
| `max_held_bytes` | Bounds what a writer holds and what a decoder builds for one part. |

`ConvertedRequest::reply` supplies `routes`, `show_reasoning`, `include_usage` and `emulated_stop_sequences`.

## Provider HTTP errors

A non-success response is not decoded as a reply. `error_reply(client, upstream, status, body)` reads enough
of the provider's error to classify it, writes the client's error shape and keeps the provider's status. No
usage is counted.

## Buffered replies

1. `BufferedReply::new(context)`, then any `set_rewrites` and `set_checks`.
2. `read(body)` once. The caller bounds the body beforehand.
3. Resolve any [hold](#replies-held-for-a-verdict).
4. Read `usage()` and `telemetry()`, then `into_reply()` for the client's `HttpReply`.

A decode error from `read` is also recorded in the writer, so `into_reply` returns a failure in the client's
protocol, never a partial success.

## Streaming replies

1. `StreamReply::new(context, max_event_bytes)`, then any `set_rewrites` and `set_checks`.
2. For each transport chunk: `push(chunk)`, read usage and telemetry, resolve any hold, and `take_output()`
   before reading the next chunk.
3. Stop when `is_finished()`. If input ends first, call `finish()`; on a transport error, `fail()` then
   `finish()`.
4. If `early_failure()` returns a reply, send it instead of stream bytes.

Taking output between chunks is what lets backpressure reach the provider. `take_output` returns nothing while
an early HTTP failure is still possible.

`push` returns the first decode error in a chunk after processing the rest of it. The client's output has
failed, but keep feeding later chunks: the decoder may still recover usage and the service tier.

`finish` does nothing after a valid final event. Otherwise it fails the client's reply and reports truncated
SSE or an unfinished provider reply.

## Replies held for a verdict

`set_checks` with `OutputChecks` holds output back until the caller has checked it. The driver keeps
accounting for provider events while output is held.

After each `read`, `push` and `finish`, call `progress()`. On `ReplyProgress::Waiting`:

1. Take any output written before the hold.
2. Check `HeldReply::check()` outside this crate, with the caller's own time limits and failure modes.
3. Call `HeldReply::resolve` with `Verdict::Release` or `Verdict::Stop`, then repeat, since releasing can
   expose another check.

`Release` sends the held output. `Stop(message)` discards it and fails the reply with that message: an HTTP
error from `early_failure()` before any output, or a terminal error event after output. A part's start waits
with its first window, so it cannot start the client's stream before that verdict. `IncompleteCall::Stop`
fails the reply the same way. Accounting continues through the provider's end.

Do not read another provider chunk while a reply is held. That slows the provider through backpressure. Events
that do arrive are kept up to `max_held_bytes`; past it the reply fails.

Each text part's deltas gather into windows of `window_bytes`; the rest is checked when the part ends.
`overlap_bytes` repeats a UTF-8-safe suffix of the previous window as context; it is checked again but not
sent twice. A tool call is checked whole, as one JSON value with its name and complete arguments, and is then
sent or dropped. `incomplete_call` says what happens to a call the provider cut off.

## Event and accounting order

Each decoded event passes through, in order:

1. `UsageFold` and `Telemetry`, which see the provider's event unchanged.
2. `StopAt`, for stop sequences the provider could not enforce.
3. `ReplyRewrites`, which undo request edits or apply output policy.
4. Output checks, which hold output back for the caller's verdict.
5. The client writer.

Usage and telemetry therefore describe the provider's work, including what a rewrite or an emulated stop
hides. Stop sequences match the provider's text, as they would if the provider enforced them. Rewrites and
output checks see only text the client can receive.

## Failure rules

- A provider HTTP error keeps its status and takes the client's error shape.
- A decode or transport failure before client output is an HTTP error, from `early_failure()`.
- A failure after output is an error in the client's stream.
- A failed reply never receives a successful final event.
- Usage and service-tier events after a decode failure still update accounting.

## Size limits

| Limit | Set by |
|---|---|
| The body given to `BufferedReply::read` | The caller, before the call. |
| One provider SSE event | `max_event_bytes`. |
| One decoded part, and client output waiting to be written | `ReplyContext::max_held_bytes`. |
| A transport chunk given to `StreamReply::push` | The caller. |

The SSE parser copies each input chunk, so the event limit bounds neither the chunk nor total retained
capacity.

A Responses provider repeats the whole response in its final event. When that event exceeds `max_event_bytes`,
the decoder scans it in pieces, validates the final values and keeps usage without retaining the repeated
output. An oversized event from any other provider fails the client's reply; later input is still read for
accounting.

## Passthrough replies

When the provider speaks the client's protocol, the caller forwards the provider's bytes and gives a copy to
`ObservedReply`, which decodes them only for `UsageFold` and `Telemetry`:

1. `ObservedReply::new(upstream, model, max_event_bytes, max_held_bytes)`.
2. `read` a whole body, or `push` each chunk and then `finish`.
3. Read `usage()` and `telemetry()`.

A reply it cannot decode still reaches the client; only its usage is lost. Exceeding a limit stops the
reading, not the reply.

## Lower-level APIs

A caller that owns its own framing can use the protocol decoders, observers and writers directly. Their events
borrow from the decoded payload, so copy anything that must outlive the callback.
