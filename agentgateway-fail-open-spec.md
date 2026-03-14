# Graceful degradation for MCP backend fanout

## Problem

When an `AgentgatewayBackend` has multiple MCP targets, `send_fanout()` iterates targets sequentially and uses `?` to short-circuit on the first connection failure. This means one unreachable target kills the entire session — no tools from any target are returned.

```rust
// crates/agentgateway/src/mcp/handler.rs:354
for (name, con) in self.upstreams.iter_named() {
    streams.push((name, con.generic_stream(r.clone(), &ctx).await?)); // ← bail on first error
}
```

The same pattern exists in `send_notification()` (line 367) and `setup_connections()` in `mcp/upstream/mod.rs`.

This makes the gateway unusable in any environment where backends have different availability characteristics (external APIs, optional services, services with cold-start latency).

Related: #1029

## Proposed change

### 1. Skip failed targets in fanout, log warnings

```rust
pub async fn send_fanout(
    &self,
    r: JsonRpcRequest<ClientRequest>,
    ctx: IncomingRequestContext,
    merge: Box<MergeFn>,
) -> Result<Response, UpstreamError> {
    let id = r.id.clone();
    let mut streams = Vec::new();
    for (name, con) in self.upstreams.iter_named() {
        match con.generic_stream(r.clone(), &ctx).await {
            Ok(stream) => streams.push((name, stream)),
            Err(e) => warn!("target '{}' failed to connect, skipping: {}", name, e),
        }
    }
    if streams.is_empty() {
        return Err(UpstreamError::Send("all MCP targets failed to initialize".into()));
    }
    let ms = mergestream::MergeStream::new(streams, id.clone(), merge);
    messages_to_response(id, ms)
}
```

Apply the same pattern to `send_notification()`, `send_fanout_get()`, `send_fanout_deletion()`, and `setup_connections()`.

### 2. Per-target `required` field (optional, additive)

For users who want strict behavior on specific targets, add an optional field to the target spec:

```yaml
spec:
  mcp:
    targets:
      - name: critical-service
        required: true          # fail session if this target is unreachable (default: false)
        static: { ... }
      - name: nice-to-have
        static: { ... }         # skipped if unreachable
```

Default should be `required: false` (fail-open). This is the safe default — a gateway that drops tools is more useful than a gateway that returns 500.

## Scope

- `crates/agentgateway/src/mcp/handler.rs` — `send_fanout`, `send_notification`, `send_fanout_get`, `send_fanout_deletion`
- `crates/agentgateway/src/mcp/upstream/mod.rs` — `setup_connections`
- CRD schema for `AgentgatewayBackend` — optional `required` field on target spec
- Metrics: emit a counter for skipped targets so operators can alert on degraded state
