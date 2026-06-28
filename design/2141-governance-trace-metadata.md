# EP-2141: Recommended Governance Metadata for Gateway Traces

- Issue: [#2141](https://github.com/agentgateway/agentgateway/issues/2141)
- Related: [examples/telemetry](../examples/telemetry/README.md), [`dtrace`](../crates/agentgateway/src/proxy/dtrace.rs)
- Status: proposed
- Date: 6/16/2026

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

agentgateway already emits debug-trace events (`Policy`, `AuthorizationResult`, `RouteSelection`, MCP tool calls, and
backend lifecycle events) through the `dtrace` pipeline and can export spans via OTLP. Operators still lack a documented
vocabulary for the subset of trace metadata that explains *why* a gateway call was allowed, denied, routed, or sent to a
specific backend.

This design proposes a small metadata convention for exported gateway traces. The fields intentionally map to concepts
that exist in the current codebase: listeners, routes, backends, MCP targets and tools, policies, authorization results,
and trace/span ids. The first iteration is documentation only.

## Background

Issue [#2141](https://github.com/agentgateway/agentgateway/issues/2141) asks whether agentgateway should document
metadata conventions for traces that capture:

- decision result for a call
- policy or rule that produced the decision
- selected route and backend target
- MCP target, method, or tool name
- stable id for later review

The data plane already records related facts in debug traces. For example, `MessageType::AuthorizationResult` exposes
allow/deny outcomes, `MessageType::Policy` records apply/skip decisions, and MCP paths emit tool-call counters. What is
missing is a **cross-export contract** so OTLP spans, JSONL `agctl proxy trace` output, and external audit systems use
consistent field names and semantics.

Without that contract, an operator may see that a request returned HTTP 200 but still have to reconstruct which route was
selected, which policy applied, and which MCP tool/backend was reached by reading several trace events manually.

## Goals

- Document a minimal, recommended set of governance fields for gateway spans.
- Map each field to existing `dtrace`, telemetry, or request-log concepts where possible.
- Provide JSON examples for an allowed MCP tool call and a denied authorization decision.
- Provide an OpenTelemetry attribute mapping table for OTLP exporters.
- Give operators enough structure to answer: *which route/backend/MCP target was selected, which policy decided the
  request, and what trace/span id can be used for follow-up inspection?*

## Non-Goals

- Implement new dataplane enforcement in this design iteration.
- Replace existing debug-trace JSONL format or `agctl proxy trace` output.
- Mandate a single SIEM schema; this doc recommends conventions, not a proprietary product format.
- Define approval, checkpoint, or orchestration fields that do not currently exist in the gateway runtime model.

## Recommended metadata fields

Each gateway span that represents a routed request or MCP call SHOULD include the following logical fields when the
source data exists. Names below use dot notation; OTel mapping follows in a later section.

| Field | Required | Description |
|-------|----------|-------------|
| `decision.result` | Yes, when evaluated | `allow`, `deny`, `apply`, or `skip` |
| `decision.policy_kind` | No | Policy kind that produced the decision, matching `Policy.kind` |
| `decision.policy_name` | No | Policy name when available from a selected attached policy |
| `route.listener` | Recommended | Listener or bind name associated with the request |
| `route.selected` | Recommended | Selected HTTP/TCP route key from `RouteSelection.selectedRoute` |
| `backend.target` | Recommended | Backend target from `BackendCallStart.target` |
| `mcp.target` | No | MCP target/server name when the call is routed to MCP |
| `mcp.method` | No | MCP method name, for example `tools/call` or `tools/list` |
| `mcp.tool.name` | No | MCP tool name for `tools/call` |
| `request.arguments_digest` | No | SHA-256 (or similar) of normalized MCP tool arguments, if captured |
| `audit.trace_id` | Recommended | Trace id used for cross-system correlation |
| `audit.span_id` | Recommended | Span id used for cross-system correlation |
| `identity.subject` | No | Caller identity propagated through the gateway (JWT sub, mTLS CN, etc.) |

### `decision.result` semantics

| Value | Meaning |
|-------|---------|
| `allow` | Authorization rule allowed the request |
| `deny` | Authorization rule denied the request |
| `apply` | A policy applied and changed request/response handling |
| `skip` | A policy was evaluated but did not apply |

### Mapping to existing `dtrace` events

| Governance field | Existing `MessageType` / event |
|------------------|--------------------------------|
| `decision.result` | `AuthorizationResult.result`, `Policy.result` (`Apply` / `Skip`) |
| `decision.policy_kind` | `Policy.kind`, `PolicyEvent.kind` |
| `decision.policy_name` | `PolicySelection.effectivePolicy`, where a named attached policy is available |
| `route.selected` | `RouteSelection.selectedRoute` |
| `backend.target` | `BackendCallStart.target` |
| `mcp.target` | MCP server/target name recorded in request logs and MCP metrics |
| `mcp.method` | MCP method name recorded on MCP request logs |
| `mcp.tool.name` | MCP tool name recorded on MCP request logs and `tool_calls_total` |
| `request.arguments_digest` | Derived from captured MCP tool arguments at export time |
| `audit.trace_id` / `audit.span_id` | Existing trace/span identifiers |
| `identity.subject` | JWT / RBAC context already augmented in telemetry examples |

## JSON examples

### Example A: allowed MCP tool call

```json
{
  "decision": {
    "result": "allow",
    "policy_kind": "mcp_authorization",
    "policy_name": "tools-read"
  },
  "route": {
    "listener": "bind/3000",
    "selected": "default/default"
  },
  "backend": {
    "target": "everything"
  },
  "mcp": {
    "target": "everything",
    "method": "tools/call",
    "tool": {
      "name": "echo"
    }
  },
  "audit": {
    "trace_id": "0af7651916cd43dd8448eb211c80319c",
    "span_id": "b9c7c989f97918e1"
  },
  "identity": {
    "subject": "user:alice@example.com"
  }
}
```

### Example B: denied MCP tool call

```json
{
  "decision": {
    "result": "deny",
    "policy_kind": "mcp_authorization",
    "policy_name": "tools-write-deny"
  },
  "route": {
    "listener": "bind/3000",
    "selected": "default/default"
  },
  "backend": {
    "target": "inventory"
  },
  "mcp": {
    "target": "inventory",
    "method": "tools/call",
    "tool": {
      "name": "update_asset"
    }
  },
  "request": {
    "arguments_digest": "sha256:b6d767d2f8ed5d21a44b0e5886680cb91f5851b2"
  },
  "audit": {
    "trace_id": "0af7651916cd43dd8448eb211c80319c",
    "span_id": "b9c7c989f97918e1"
  },
  "identity": {
    "subject": "user:bob@example.com"
  }
}
```

Deny reason remains in the underlying `AuthorizationResult.rules` / `Policy.result` details rather than a separate
metadata field.

## OpenTelemetry attribute mapping

When exporting via OTLP, map logical fields to span attributes under the `agentgateway.governance.*` namespace:

| Logical field | Suggested OTel attribute |
|---------------|--------------------------|
| `decision.result` | `agentgateway.governance.decision.result` |
| `decision.policy_kind` | `agentgateway.governance.decision.policy_kind` |
| `decision.policy_name` | `agentgateway.governance.decision.policy_name` |
| `route.listener` | `agentgateway.governance.route.listener` |
| `route.selected` | `agentgateway.governance.route.selected` |
| `backend.target` | `agentgateway.governance.backend.target` |
| `mcp.target` | `agentgateway.governance.mcp.target` |
| `mcp.method` | `agentgateway.governance.mcp.method` |
| `mcp.tool.name` | `agentgateway.governance.mcp.tool.name` |
| `request.arguments_digest` | `agentgateway.governance.request.arguments_digest` |
| `audit.trace_id` | `agentgateway.governance.audit.trace_id` |
| `audit.span_id` | `agentgateway.governance.audit.span_id` |
| `identity.subject` | `agentgateway.governance.identity.subject` |

Existing HTTP and MCP metrics (for example `tool_calls_total`, `agentgateway_requests_total`) remain unchanged. Governance
attributes complement metrics by making individual calls auditable.

## Runtime Design

**Phase 1 (this PR):** documentation only — design doc, examples, telemetry README cross-link.

**Phase 2 (follow-up implementation):** populate governance attributes on OTLP spans at export time by translating
existing debug-trace and request-log data:

```text
request
  -> route selection (route.selected)
  -> authorization / policy evaluation (decision.result, decision.policy_kind)
  -> backend / MCP call (backend.target, mcp.target, mcp.method, mcp.tool.name)
  -> trace context (audit.trace_id, audit.span_id)
  -> export span with governance.* attributes
```

Phase 2 should not duplicate logging; it should attach the same facts already emitted to debug traces.

## Compatibility and Migration

- No behavior change for existing deployments.
- Operators may adopt the attribute names incrementally in dashboards and SIEM rules.
- JSONL debug trace format remains the source of truth for deep inspection; governance metadata is a summarized export
  layer.

## Risks and Tradeoffs

- **Attribute cardinality:** `backend.target`, `mcp.tool.name`, and `identity.subject` can be high-cardinality; operators may need sampling
  or aggregation in metrics backends while retaining full attributes on trace spans.
- **Digest algorithm:** documenting `sha256` as recommended but not mandating a single canonical JSON normalization
  may cause inconsistent digests across exporters until a normative normalization spec is added.
- **Overlap with OTel semantic conventions:** some fields may eventually align with upstream semantic conventions; the
  `agentgateway.governance.*` prefix avoids collision until alignment is agreed.

## Test Plan

- Doc review only for Phase 1.
- Phase 2 implementation should add:
  - unit tests mapping `AuthorizationResult`, `Policy`, `RouteSelection`, and `BackendCallStart` messages to attributes
  - an integration test in `examples/telemetry` asserting expected attributes appear in exported spans
  - a fixture JSON golden file for Example A and Example B

## Open Questions

1. Should `request.arguments_digest` use a normative JSON canonicalization spec in-repo, or reference an external standard?
2. Do maintainers prefer governance attributes on the root request span only, or also on nested backend/MCP child spans?
3. Should `decision.policy_name` be omitted until the policy selection path consistently exposes a stable name?
