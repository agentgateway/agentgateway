# Validation

Unit tests in the crate cover parsing, rendering, replay encoding, limits and state-machine errors. The
integration tests here drive the public APIs callers use, and compare the IR with the legacy converters it
replaces.

## Where behaviour is tested

| Layer | Owns |
| --- | --- |
| Primitive and state-machine unit tests | Local rules, limits, event order and accounting. |
| Protocol adapter tests | Requests parse and render correctly; provider input becomes the right events, and events become the client's wire format. |
| Public pipeline and reply driver tests | Conversion, rewrites, held verdicts and replay working together, for every provider and client pair, buffered and streamed, across arbitrary chunk boundaries. |
| Legacy parity tests | Each legacy fixture is equivalent under the IR, or differs for a stated reason. |
| Gateway tests | Selection, fallback, policy order, retries, transport and usage settlement. |

A behaviour that several layers touch has one owner. A test at another layer should prove a different
boundary, not repeat the owner's assertions. For example, decoders classify provider failures and writers
encode them, while the reply driver tests check that a failure after output gives one client error, keeps
usage and never ends successfully.

Legacy fixtures don't send every setting on every route, so for some settings an adapter test is the only one
that fails when conversion breaks: Chat `temperature`, `max_completion_tokens` over `max_tokens`, Messages
cache marks on earlier turns, and Gemini stop sequences.

Reply snapshots keep each SSE payload as one line of compact JSON, so event names, fields and order stay
visible.

## Legacy parity

The legacy parity tests run the legacy converters' fixtures through the IR. Each input has a `Disposition`:
equivalent, a legacy bug the IR fixes, a deliberate change, a refusal, or outside the route. A new input fails
until it has one. The IR's result is snapshotted beside the unchanged legacy snapshots. The fixtures document
individual cases; their counts don't estimate how much real traffic is compatible.

Each route's tests compare each request with the live legacy conversion. A change asserts the legacy value,
applies only that change to the expected request, and requires the rest to match. Each route's
`without_representation` lists the few encoding differences treated as equal, such as omitted `stream` and
`stream: false`. Policy fields, cache marks and output counts are never normalized away.

Replies are compared the same way, folded to text, reasoning, calls, ending and usage. Every expected refusal
is listed per fixture and route, so an unrelated parse error can't stand in for one.

## Properties and fuzzing

Bounded proptest cases run with the normal tests. They generate SSE payloads, arbitrary bytes and chunk
boundaries, stop sequences, output-check windows, replay state and usage reports. Keep each reduced failing
input as a regression case.

The [fuzz targets](../../../fuzz/README.md) cover requests and replies for crashes and broken wire output. CI
does not build or run them.
