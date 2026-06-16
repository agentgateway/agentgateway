# EP-2141: Recommended Governance Metadata for Gateway Traces

- Issue: [#2141](https://github.com/agentgateway/agentgateway/issues/2141)
- Related: [examples/telemetry](../examples/telemetry/README.md), [`dtrace`](../crates/agentgateway/src/proxy/dtrace.rs)
- Status: proposed
- Date: 6/16/2026

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

agentgateway already emits rich debug-trace events (`Policy`, `AuthorizationResult`, `RouteSelection`, MCP tool
calls, and backend lifecycle events) through the `dtrace` pipeline and can export spans via OTLP. Operators still lack a
documented, portable vocabulary for **governance metadata**: fields that answer whether a call succeeded through a
scoped, reviewable path rather than only whether it returned HTTP 200.

This design proposes a **recommended metadata convention** for gateway traces. It does not require new runtime behavior
in the first iteration. The goal is a stable doc + JSON examples + OpenTelemetry attribute mapping that implementors,
SIEM exporters, and downstream audit tools can rely on.

## Background

Issue [#2141](https://github.com/agentgateway/agentgateway/issues/2141) asks whether agentgateway should document
metadata conventions for traces that capture:

- decision result for a call
- risk class of the action
- selected capability or tool
- whether output was bounded before returning upstream
- audit id for later review

The data plane already records related facts in debug traces. For example, `MessageType::AuthorizationResult` exposes
allow/deny outcomes, `MessageType::Policy` records apply/skip decisions, and MCP paths emit tool-call counters. What is
missing is a **cross-export contract** so OTLP spans, JSONL `agctl proxy trace` output, and external audit systems use
consistent field names and semantics.

Multi-step workflows fail in subtle ways when traces only show success/failure:

- a session goes stale but the gateway returns an empty success payload
- an earlier human approval is reused after tool arguments change
- a phase gate passes but a later destructive action proceeds without a fresh authorization binding

Governance metadata makes those cases visible in the execution history.

## Goals

- Document a minimal, recommended set of governance fields for gateway spans.
- Map each field to existing `dtrace` concepts where possible.
- Provide JSON examples for an allowed read call and a denied write call.
- Provide an OpenTelemetry attribute mapping table for OTLP exporters.
- Give operators enough structure to answer: *was this call allowed, under which policy, for which target, with which
  arguments, and was the response bounded before return?*

## Non-Goals

- Implement new dataplane enforcement in this design iteration.
- Define a global workflow orchestration standard (see AAIF Workflows working group for adjacent work).
- Replace existing debug-trace JSONL format or `agctl proxy trace` output.
- Mandate a single SIEM schema; this doc recommends conventions, not a proprietary product format.

## Recommended metadata fields

Each gateway span that represents an enforced tool/API call SHOULD include the following logical fields. Names below
use dot notation; OTel mapping follows in a later section.

| Field | Required | Description |
|-------|----------|-------------|
| `decision.result` | Yes | `allow`, `deny`, `redact`, or `require_confirmation` |
| `decision.policy_id` | No | Stable id or name of the guardrail, route rule, or CEL policy that fired |
| `action.risk_class` | Yes | `read`, `write`, `destructive`, or `external_publish` |
| `action.target` | Yes | Tool name, resource URI, route target, or backend identifier |
| `action.arguments_digest` | Recommended | SHA-256 (or similar) of normalized arguments at enforcement time |
| `capability.selected` | No | Resolved capability, tool version, or MCP server name |
| `output.bounded` | No | `true` if response was truncated, redacted, or schema-limited before return |
| `workflow.phase` | No | Optional phase label when the call sits inside a multi-step process |
| `workflow.checkpoint_ref` | No | Link to a prior human gate or approval record, if any |
| `audit.id` | Recommended | Stable id for cross-system correlation (span id, trace id, or operator-assigned id) |
| `identity.subject` | Recommended | Caller identity propagated through the gateway (JWT sub, mTLS CN, etc.) |

### `decision.result` semantics

| Value | Meaning |
|-------|---------|
| `allow` | Call proceeded under policy |
| `deny` | Call blocked; upstream must not execute |
| `redact` | Call proceeded but response was redacted or bounded |
| `require_confirmation` | Call deferred pending explicit confirmation binding |

`require_confirmation` is reserved for integrations that bind human approval to a specific action envelope (see related
discussion in [agentskills#413](https://github.com/agentskills/agentskills/issues/413)).

### Mapping to existing `dtrace` events

| Governance field | Existing `MessageType` / event |
|------------------|--------------------------------|
| `decision.result` | `AuthorizationResult.result` (`Allow` / `Deny`); extend for `redact` / `require_confirmation` at export |
| `decision.policy_id` | `Policy.kind`, `PolicySelection.effectivePolicy` |
| `action.target` | `BackendCallStart.target`, MCP tool name metrics, `RouteSelection.selectedRoute` |
| `action.arguments_digest` | Derived from `RequestSnapshot` / MCP tool call args at enforcement time (export-time hash) |
| `capability.selected` | MCP server name, `LlmRouteResolved.provider` |
| `output.bounded` | Policy apply with redaction / response snapshot limits |
| `audit.id` | Trace id + span id, or operator-provided correlation id |
| `identity.subject` | JWT / RBAC context already augmented in telemetry examples |

## JSON examples

### Example A: allowed MCP read

```json
{
  "decision": {
    "result": "allow",
    "policy_id": "mcp-rbac/tools-read"
  },
  "action": {
    "risk_class": "read",
    "target": "mcp://everything/tools/list",
    "arguments_digest": "sha256:8f14e45fceea167a5a36dedd4bea2543ad36b3c6"
  },
  "capability": {
    "selected": "everything"
  },
  "output": {
    "bounded": false
  },
  "audit": {
    "id": "01JXYZ9ABCDEFGHJKMNPQRSTVW"
  },
  "identity": {
    "subject": "user:alice@example.com"
  }
}
```

### Example B: denied write (arguments changed after checkpoint)

```json
{
  "decision": {
    "result": "deny",
    "policy_id": "mcp-guardrails/write-envelope"
  },
  "action": {
    "risk_class": "write",
    "target": "mcp://inventory/tools/update_asset",
    "arguments_digest": "sha256:b6d767d2f8ed5d21a44b0e5886680cb91f5851b2"
  },
  "capability": {
    "selected": "inventory"
  },
  "workflow": {
    "phase": "before_state_transition",
    "checkpoint_ref": "approval:01JXYZ8ZYXWVUTSRQPNMLKJIHG"
  },
  "audit": {
    "id": "01JXYZ9ABCDEFGHJKMNPQRSTVW"
  },
  "identity": {
    "subject": "user:bob@example.com"
  }
}
```

Deny reason (companion field, optional): `arguments_digest mismatch vs checkpoint_ref; re-confirmation required`.

## OpenTelemetry attribute mapping

When exporting via OTLP, map logical fields to span attributes under the `agentgateway.governance.*` namespace:

| Logical field | Suggested OTel attribute |
|---------------|--------------------------|
| `decision.result` | `agentgateway.governance.decision.result` |
| `decision.policy_id` | `agentgateway.governance.decision.policy_id` |
| `action.risk_class` | `agentgateway.governance.action.risk_class` |
| `action.target` | `agentgateway.governance.action.target` |
| `action.arguments_digest` | `agentgateway.governance.action.arguments_digest` |
| `capability.selected` | `agentgateway.governance.capability.selected` |
| `output.bounded` | `agentgateway.governance.output.bounded` |
| `workflow.phase` | `agentgateway.governance.workflow.phase` |
| `workflow.checkpoint_ref` | `agentgateway.governance.workflow.checkpoint_ref` |
| `audit.id` | `agentgateway.governance.audit.id` |
| `identity.subject` | `agentgateway.governance.identity.subject` |

Existing HTTP and MCP metrics (for example `tool_calls_total`, `agentgateway_requests_total`) remain unchanged. Governance
attributes complement metrics by making individual calls auditable.

## Runtime Design

**Phase 1 (this PR):** documentation only — design doc, examples, telemetry README cross-link.

**Phase 2 (follow-up implementation):** populate governance attributes on OTLP spans at export time by translating
existing `dtrace` messages:

```text
request
  -> route selection (action.target)
  -> authorization (decision.result, identity.subject)
  -> policy evaluation (decision.policy_id, output.bounded)
  -> backend / MCP call (capability.selected, action.arguments_digest)
  -> export span with governance.* attributes
```

Phase 2 should not duplicate logging; it should attach the same facts already emitted to debug traces.

## Compatibility and Migration

- No behavior change for existing deployments.
- Operators may adopt the attribute names incrementally in dashboards and SIEM rules.
- JSONL debug trace format remains the source of truth for deep inspection; governance metadata is a summarized export
  layer.

## Risks and Tradeoffs

- **Attribute cardinality:** `action.target` and `identity.subject` can be high-cardinality; operators may need sampling
  or aggregation in metrics backends while retaining full attributes on trace spans.
- **Digest algorithm:** documenting `sha256` as recommended but not mandating a single canonical JSON normalization
  may cause inconsistent digests across exporters until a normative normalization spec is added.
- **Overlap with OTel GenAI conventions:** some fields may eventually align with upstream semantic conventions; the
  `agentgateway.governance.*` prefix avoids collision until alignment is agreed.

## Test Plan

- Doc review only for Phase 1.
- Phase 2 implementation should add:
  - unit tests mapping `AuthorizationResult` / `Policy` messages to governance attributes
  - an integration test in `examples/telemetry` asserting expected attributes appear in exported spans
  - a fixture JSON golden file for Example A and Example B

## Open Questions

1. Should `action.arguments_digest` use a normative JSON canonicalization spec in-repo, or reference an external standard?
2. Should `require_confirmation` integrate with an external approval store, or remain gateway-local in v1?
3. Do maintainers prefer governance attributes on the root request span only, or also on nested backend/MCP child spans?
4. Should this convention be proposed to the AAIF Observability working group as a reference profile?
