# HTTP Pool Rewrite Audit

## Scope and Method

Audited `crates/hyper-util-fork/src/client/legacy/`, with the focus on the ground-up rewrite in:

- `crates/hyper-util-fork/src/client/legacy/pool.rs`
- `crates/hyper-util-fork/src/client/legacy/client.rs`
- `crates/hyper-util-fork/src/client/legacy/http2_tests.rs`
- `crates/hyper-util-fork/src/client/legacy/connect/*`

I split the review across parallel passes for:

- state/accounting correctness
- concurrency and cancellation behavior
- tests, operability, and maintainability

I also validated the in-tree unit suite with:

```bash
cargo test -p hyper-util-fork client::legacy::pool -- --nocapture
```

That command passed with `31` tests, but it does not exercise the disabled blackbox HTTP/2 suite described below.

## Scorecard

| Aspect | Score | Notes |
| --- | --- | --- |
| State accounting correctness | 1/5 | There are hard bugs in HTTP/2 stream accounting and pending-capacity bookkeeping. |
| Concurrency and lifecycle robustness | 2/5 | Cancellation paths are still brittle, and one waiter path still panics. |
| Regression safety | 2/5 | `pool.rs` has decent gray-box tests, but the end-to-end HTTP/2 suite is commented out and `client.rs` is largely untested. |
| Operability and maintainability | 2/5 | Debug leftovers, TODOs in hot paths, and unfinished cleanup make incidents harder to diagnose. |
| Design clarity | 3/5 | The overall model is understandable, and the active-H2 queue ordering is mostly coherent. |

## Overall Verdict

This rewrite has real structure behind it, and the internal `pool.rs` test coverage is better than it first appears. But it is not yet a trustworthy HTTP/2 pool implementation. The biggest problems are not style issues; they are state-accounting bugs that can corrupt scheduling decisions under real traffic.

If this code is intended for production HTTP/2 pooling, I would treat it as not ready yet.

## Findings

### 1. High: `expected_connecting_capacity` can underflow or wrap on large HTTP/2 peers

Evidence:

- `crates/hyper-util-fork/src/client/legacy/pool.rs:591-618`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:749-759`

The pool increments `expected_connecting_capacity` using the guessed capacity, but on insert it subtracts the actual connection capacity immediately:

- `host.expected_connecting_capacity += expected`
- later `host.expected_connecting_capacity -= capacity`

The code only compensates for `capacity < expected_capacity`. There is no symmetric handling for `capacity > expected_capacity`.

Impact:

- If a peer advertises more concurrent streams than `expected_http2_capacity`, this counter can underflow in debug or wrap in release.
- Once corrupted, `pending <= waiters` stops meaning what the scheduler thinks it means, so new-connection decisions become wrong for that host.

This is a hard correctness bug.

### 2. High: HTTP/2 capacity is released at response-head time, not stream-completion time

Evidence:

- `crates/hyper-util-fork/src/client/legacy/client.rs:241-287`
- `crates/hyper-util-fork/src/client/legacy/client.rs:265-269`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:973-986`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:383-393`

The client returns `Ok(res)` as soon as `try_send_request` yields a `Response`. For HTTP/2, the code explicitly leaves a TODO saying the stream slot should be released when the request actually ends, but that guard is not implemented.

Today the `Pooled` wrapper is dropped as `send_request` returns, and `Pooled::drop` decrements the HTTP/2 load immediately.

Impact:

- Long-lived response bodies are no longer counted as active.
- The pool can believe a connection has free stream capacity earlier than the protocol actually does.
- Under sustained streaming or slow body consumption, fairness and connection-scaling decisions become unreliable.

This is also a real correctness issue, not just a missing optimization.

### 3. High: canceled HTTP/2 waiter handoff can leave stream load permanently overcounted

Evidence:

- `crates/hyper-util-fork/src/client/legacy/pool.rs:646-705`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:914-933`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:596-606`

`send_connection()` calls `maybe_clone()` before it knows whether `tx.send(Ok(this))` will succeed. For HTTP/2, `maybe_clone()` increments `H2Load`. If the receiver is canceled in the handoff race, the code recovers `this`, but it does not undo the reservation that was just taken.

Impact:

- A canceled waiter can consume stream capacity forever in the pool’s model, even though no request actually started.
- On fresh HTTP/2 inserts, this combines badly with `active_h2.maybe_insert_new(...)` plus the `sent == 0` idle path, since the same connection can end up pre-registered as active and then also parked idle after all waiters vanish.

The exact user-visible failure mode depends on timing, but the accounting bug is real.

### 4. Medium-High: failure and low-capacity wakeups can leave live waiters stranded behind canceled ones

Evidence:

- `crates/hyper-util-fork/src/client/legacy/pool.rs:410-452`

The notification loop for shared-failure and low-capacity paths is bounded by `0..to_notify`, but canceled senders still consume an iteration.

Impact:

- If several front-of-queue waiters have already been canceled, the loop can finish without notifying the intended number of live waiters.
- Those live waiters then remain parked until some unrelated later pool event happens.

This is especially relevant because cancellation-heavy traffic is normal for request racing and timeouts.

### 5. Medium: reused HTTP/1 connections handed directly to waiters are marked as not reused

Evidence:

- `crates/hyper-util-fork/src/client/legacy/pool.rs:454-464`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:669-673`
- `crates/hyper-util-fork/src/client/legacy/client.rs:246-249`

`return_idle()` can hand a recycled HTTP/1 connection directly to a waiting checkout, but `send_connection()` always constructs that `Pooled` value with `is_reused: false`.

Impact:

- Retry behavior in `client.rs` depends on whether the connection was reused.
- If a reused idle HTTP/1 socket dies before the next request starts, this path can suppress a retry that should have been allowed.

The pool’s internal bookkeeping survives, but the user-visible failure policy is wrong on this path.

### 6. Medium: the waiter path still contains a production panic

Evidence:

- `crates/hyper-util-fork/src/client/legacy/client.rs:346-348`

If the oneshot sender disappears unexpectedly, the request future panics instead of surfacing an error.

I checked this carefully because some panic reports here would have been false alarms. This one is real: the code still contains an unconditional `panic!` in a runtime path, even if the author expects the invariant to hold.

Impact:

- Any missed edge case in waiter resolution becomes a process-level failure in production.

### 7. Medium: the end-to-end HTTP/2 regression suite is effectively disabled

Evidence:

- `crates/hyper-util-fork/src/client/legacy/mod.rs:8-9`
- `crates/hyper-util-fork/src/client/legacy/http2_tests.rs:1-423`

`mod.rs` still includes `http2_tests`, but the file is commented out wholesale. That disabled suite previously covered exactly the behaviors most likely to regress in this rewrite:

- sequential reuse
- concurrent sharing
- connect-delay fairness
- burst completion
- steady-state connection churn
- scale-out under heavy load
- ALPN downgrade to HTTP/1 in auto mode

Impact:

- The in-file `pool.rs` tests are useful, but they do not exercise the real `client.rs` integration path users actually hit.
- That is why the current pool tests can all pass while the issues above still exist.

### 8. Low: observability and cleanup still look unfinished

Evidence:

- `crates/hyper-util-fork/src/client/legacy/pool.rs:387`
- `crates/hyper-util-fork/src/client/legacy/pool.rs:68`
- `crates/hyper-util-fork/src/client/legacy/client.rs:244-246`
- `crates/hyper-util-fork/src/client/legacy/client.rs:266-269`
- `crates/hyper-util-fork/src/client/legacy/client.rs:377`
- `crates/hyper-util-fork/src/client/legacy/client.rs:384`

Examples:

- normal HTTP/2 return flow logs `tracing::error!("howardjohn: ...")`
- empty hosts are intentionally left behind after idle eviction
- there are still TODOs in request-send, HTTP/2 lifetime tracking, and handshake/capacity logic

None of these is the main problem, but they reinforce the same conclusion: the rewrite is still mid-flight.

## Positive Notes

- The `H2Pool` queue discipline in `crates/hyper-util-fork/src/client/legacy/pool.rs:138-210` is sensible. Once the accounting bugs are fixed, the “available at the front, full at the back” model should work.
- `crates/hyper-util-fork/src/client/legacy/pool.rs` has a meaningful gray-box test suite for cancellation, fairness, idle eviction, auto-capacity caching, and idle caps.
- I checked one suspicion that did not hold up: Hyper 1.9’s `http2::SendRequest::is_ready()` is effectively a connection-open hint, not a stream-capacity signal, so I did not count that as a finding.

## Bottom Line

The rewrite is not bad in the sense of being directionless. It has a coherent shape, and parts of it are clearly thought through.

But on the core question, "how good is this HTTP pool rewrite?", my answer is:

- promising design
- useful internal tests
- not yet correct enough for confident HTTP/2 production use

The top blockers are the accounting bugs in `pool.rs`, the early HTTP/2 release in `client.rs`, and the fact that the blackbox HTTP/2/client suite is currently turned off.
