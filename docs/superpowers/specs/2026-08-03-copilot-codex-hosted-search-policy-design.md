# Copilot Codex Hosted Search Policy Design

## Context

Issue 10 defines provider-neutral OpenAI Responses to Anthropic Messages conversion. That converter intentionally rejects hosted `web_search` declarations because Messages cannot preserve either live-search or cache-only search semantics.

Codex CLI 0.146.0 includes `{"type":"web_search","external_web_access":false}` in its default request. GitHub Copilot's Claude Messages endpoint has no faithful equivalent for that Responses hosted tool. This compatibility rule belongs to the downstream Copilot provider, not to Issue 10's generic conversion contract.

## Scope

Issue 11 establishes a Copilot-only request policy entry point for Claude models reached through the Responses route. This issue adds only the captured hosted-search rule. It does not change the generic converter, other providers, or the existing `shell`, `local_shell`, and `apply_patch` mappings.

## Design

The Copilot provider will normalize a cloned Responses request immediately before it is rendered as Anthropic Messages. When tool choice is absent or automatic, the policy removes only hosted tool declarations whose type is `web_search` and whose `external_web_access` value is exactly `false`. The normalized request is then passed to the existing provider-neutral converter.

The policy identifies hosted declarations by their `type`, so an ordinary function tool named `web_search` is preserved. Hosted `web_search` declarations with `external_web_access:true`, a missing value, a malformed value, or an explicitly selected hosted-search tool remain in the request and receive the generic converter's existing specific error. This avoids silently weakening requests that ask for live or required search behavior.

The policy entry point lives with the Copilot provider. Its transformations are ordered and provider-scoped, so later confirmed Copilot quirks can be added there without adding provider flags or exceptions to generic converters. The gateway's Copilot Claude Responses rendering branch invokes it before generic translation. Anthropic, Vertex, Bedrock, Azure, custom providers, Copilot non-Claude models, and native Messages requests do not invoke it.

Issue 11 does not migrate unrelated existing Copilot quirks. Later focused changes can move the beta-header and Messages-field compatibility rules behind the same provider boundary without changing their behavior.

## Data Flow

1. The gateway parses the client's Responses request normally.
2. Routing selects Anthropic Messages for a Copilot `claude-*` model.
3. The Copilot policy clones and normalizes the Responses request.
4. The unchanged Responses-to-Messages converter translates the normalized request.
5. Existing Copilot Messages policies and upstream request handling continue unchanged.

## Errors

The compatibility policy introduces no new error type. Requests outside its exact safe case continue into the generic converter and retain its current validation and error messages.

## Verification

Automated tests will prove that:

- the captured Codex default request reaches a Copilot Claude Messages request without the cache-only hosted-search declaration;
- all captured ordinary Codex function tools remain present;
- an ordinary function named `web_search` remains present;
- live, ambiguous, malformed, and explicitly selected hosted search remain rejected;
- non-Copilot conversion retains Issue 10's strict hosted-search rejection;
- Copilot non-Claude and native Messages paths remain unchanged.

Each future policy transformation must have a focused isolation test as well as coverage for its interaction with the accumulated Copilot policy.

Live verification will run unmodified Codex CLI 0.146.0 and GitHub Copilot CLI 1.0.75 through a shell call, tool result, follow-up, and sentinel. Repeated Codex `apply_patch` sessions will check the end-to-end tool path and gateway records. The final gate will run formatting, linting, focused and full tests, diff checks, and `quorum:converge` until it reaches consecutive clean passes.
