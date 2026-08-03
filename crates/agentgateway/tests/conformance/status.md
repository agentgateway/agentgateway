# MCP Conformance Status

## 2025-11-25 (upstream active)

Direct: 32/32 pass.<br>
Gateway: 27/32 pass, 5/32 gap.

| Scenario | Direct | Gateway | Details | Rationale |
| --- | --- | --- | --- | --- |
| `completion-complete` | pass (2 checks) | pass (2 checks) | — | — |
| `dns-rebinding-protection` | pass (2 checks) | gap (1/2 checks) | dns-rebinding-protection:localhost-host-rebinding-rejected (gap; gateway-attributed) | — |
| `elicitation-sep1034-defaults` | pass (6 checks) | gap (1/2 checks) | elicitation-sep1034-defaults:elicitation-sep1034-general (gap; investigate) | — |
| `elicitation-sep1330-enums` | pass (6 checks) | gap (1/2 checks) | elicitation-sep1330-enums:elicitation-sep1330-general (gap; investigate) | — |
| `json-schema-2020-12` | pass (8 checks) | pass (8 checks) | — | — |
| `logging-set-level` | pass (2 checks) | pass (2 checks) | — | — |
| `ping` | pass (2 checks) | pass (2 checks) | — | — |
| `prompts-get-embedded-resource` | pass (2 checks) | pass (2 checks) | — | — |
| `prompts-get-simple` | pass (2 checks) | pass (2 checks) | — | — |
| `prompts-get-with-args` | pass (2 checks) | pass (2 checks) | — | — |
| `prompts-get-with-image` | pass (2 checks) | pass (2 checks) | — | — |
| `prompts-list` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-list` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-read-binary` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-read-text` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-subscribe` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-templates-read` | pass (2 checks) | pass (2 checks) | — | — |
| `resources-unsubscribe` | pass (2 checks) | pass (2 checks) | — | — |
| `server-initialize` | pass (3 checks) | pass (3 checks) | — | — |
| `server-session-lifecycle` | pass (3 checks) | pass (3 checks) | — | — |
| `server-sse-multiple-streams` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-audio` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-elicitation` | pass (2 checks) | gap (1/2 checks) | tools-call-elicitation:tools-call-elicitation (gap; gateway-attributed) | — |
| `tools-call-embedded-resource` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-error` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-image` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-mixed-content` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-sampling` | pass (2 checks) | gap (1/2 checks) | tools-call-sampling:tools-call-sampling (gap; gateway-attributed) | — |
| `tools-call-simple-text` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-with-logging` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-call-with-progress` | pass (2 checks) | pass (2 checks) | — | — |
| `tools-list` | pass (3 checks) | pass (3 checks) | — | — |

## 2026-07-28

Direct: 19/19 pass.<br>
Gateway: 18/19 pass, 1/19 gap.

| Scenario | Direct | Gateway | Details | Rationale |
| --- | --- | --- | --- | --- |
| `caching` | pass (8 checks) | pass (8 checks) | — | — |
| `http-custom-header-server-validation` | pass (5 checks) | pass (5 checks) | — | — |
| `http-header-validation` | pass (5 checks) | pass (5 checks) | — | — |
| `input-required-result-basic-elicitation` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-basic-list-roots` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-basic-sampling` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-capability-check` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-ignore-extra-params` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-missing-input-response` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-multi-round` | pass (4 checks) | pass (4 checks) | — | — |
| `input-required-result-multiple-input-requests` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-non-tool-request` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-request-state` | pass (3 checks) | pass (3 checks) | — | — |
| `input-required-result-result-type` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-tampered-state` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-unsupported-methods` | pass (2 checks) | pass (2 checks) | — | — |
| `input-required-result-validate-input` | pass (3 checks) | pass (3 checks) | — | — |
| `sep-2164-resource-not-found` | pass (4 checks) | pass (4 checks) | — | — |
| `server-stateless` | pass (28 checks) | gap (1/28 checks) | server-stateless:sep-2575-server-unsupported-version-error (gap; gateway-attributed) | intentional proxy behavior (#2417): The gateway rejects unsupported versions before opening upstream connections, so it returns gateway-supported versions rather than probing every target in the 400 path. |
