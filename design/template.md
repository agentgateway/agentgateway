# EP-XXXX: Proposal Title

- Issue: [#XXXX](https://github.com/agentgateway/agentgateway/issues/XXXX)
- Related: N/A
- Status: proposed
- Date: M/D/YYYY

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

Briefly describe the problem and the proposed solution. Focus on what changes for users and maintainers.

## Background

Explain the current behavior, why it is insufficient, and any relevant prior work or related issues.

## Goals

- State the concrete outcomes this design is intended to deliver.
- Prefer user or operator-based outcomes over implementation tasks.
- Include enough detail that reviewers can tell whether the design solves the problem.

## Non-Goals

- List decisions that are intentionally out of scope.
- Call out tempting follow-up work that should not block this proposal.
- Be explicit about compatibility or migration work that is deferred.

## API

Describe user-facing API changes, configuration shape, validation rules, and examples.

```yaml
apiVersion: example.agentgateway.dev/v1alpha1
kind: Example
metadata:
  name: example
spec:
  field: value
```

## Runtime Design

Describe how the data plane, control plane, or both will implement the design.

Include important internal types or pseudocode when it makes the design easier to review:

```text
request
  -> select route
  -> select backend
  -> apply policy
  -> call upstream
```

## Controller and xDS

Describe controller translation, generated resources, xDS/proto changes, or schema changes. If none are needed, say so.

## Policy Attachment

Describe how policies attach to the new or changed resources. Include policy ordering if it matters.

## Compatibility and Migration

Explain how existing users are affected, whether behavior changes by default, and how users migrate to the new behavior.

## Risks and Tradeoffs

List known risks, implementation tradeoffs, and alternatives that were considered.

## Test Plan

- Add API validation tests for new fields or validation rules.
- Add controller translation tests for generated resources.
- Add data-plane tests for request and response behavior.
- Add end-to-end tests for the primary user flow, when practical.

## Open Questions

- List unresolved questions that should be answered before implementation or merge.
