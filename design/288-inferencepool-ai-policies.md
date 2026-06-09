# EP-288: AI Policies for InferencePool-backed Custom Providers

- Issue: [#288](https://github.com/agentgateway/agentgateway/issues/288)
- Related: [#1714](https://github.com/agentgateway/agentgateway/issues/1714)
- Implementation: [#1932](https://github.com/agentgateway/agentgateway/pull/1932)
- Status: proposed
- Date: 5/18/2026

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

`InferencePool` traffic can be routed today, and `AgentgatewayBackend.spec.ai` traffic can use AI policies today, but
the two paths do not compose cleanly. The current backend flow runs inference routing before LLM provider handling, so a
request that ultimately needs an `InferencePool` endpoint cannot first go through provider selection, request parsing,
token counting, prompt policies, and response parsing.

This proposal fixes that by adding a `custom` LLM provider that can target a `Service`, `InferencePool`, or direct
`host + port`, and by refactoring the LLM path so provider selection happens before optional inference routing.

The design is intentionally narrow:

- Only `custom` providers get explicit backend targets.
- Existing managed providers stay unchanged.
- `InferencePool` remains a controller-lowered synthetic service plus `inferenceRouting` policy.
- MCP behavior is unchanged.

## Goals

- Let an AI backend select a `custom` provider whose concrete target is a `Service` or `InferencePool`.
- Let `custom` declare the provider-native request/response formats it supports.
- Select the provider and concrete provider target before optional inference routing.
- Preserve the existing LLM request pipeline for AI policy application, token counting, and upstream serialization.
- Reuse the existing `build_service_call` path for `Service` and lowered `InferencePool` provider targets.
- Keep managed providers such as `openai`, `anthropic`, `gemini`, `vertexai`, `azure`, and `bedrock` as they are.

## Non-Goals

- Add `backendRef` support to managed providers.
- Redesign MCP execution.
- Change the upstream GAIE `InferencePool` API or EPP protocol.
- Add a dataplane-native `InferencePool` backend kind.
- Support arbitrary gRPC custom providers.
- Allow recursive provider targets such as custom provider -> `AgentgatewayBackend`.

## API

Add a `custom` provider to `LLMProvider`:

```go
type LLMProvider struct {
    ...
    Custom *CustomProvider `json:"custom,omitempty"`
}

type CustomProvider struct {
    BackendRef *gwv1.BackendObjectReference `json:"backendRef,omitempty"`
    Model      *ShortString                  `json:"model,omitempty"`
    Formats    []ProviderFormatConfig        `json:"formats"`
}

type ProviderFormatConfig struct {
    Type ProviderFormat `json:"type"`
    Path LongString     `json:"path,omitempty"`
}
```

Validation:

- `custom` is added to the `ExactlyOneOf` provider list.
- `custom` must specify exactly one of `backendRef` or direct `host + port`.
- `formats` is required and must contain at least one format.
- `backendRef` may target only namespace-local `Service` or `InferencePool`.
- `path`, `pathPrefix`, and custom format `path` overrides are mutually exclusive.

Example:

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: chat-backend
spec:
  ai:
    groups:
    - providers:
      - name: openai-primary
        openai:
          model: gpt-4o-mini
      - name: onprem
        custom:
          backendRef:
            group: inference.networking.k8s.io
            kind: InferencePool
            name: llama-pool
          formats:
          - type: Completions
            path: /v1/chat/completions
```

### Provider Formats

`formats` declares provider-native wire formats, not route matching behavior. Each entry may optionally set `path` to
override the default upstream path for that provider-native format. The format type should be a new enum rather than a
reuse of `RouteType`.

Initial values:

- `Completions`
- `Messages`
- `Responses`
- `Embeddings`
- `AnthropicTokenCount`
- `Realtime`

Excluded values:

- `Models`: agentgateway handles generated `/v1/models` itself.
- `Detect`: this is gateway parsing/telemetry behavior.
- `Passthrough`: this is gateway routing behavior.

## Runtime Design

Add an explicit custom provider runtime variant. Do not try to unify all provider internals; existing providers
have provider-specific request, response, auth, path, and model handling that should stay on their current paths.

Conceptually:

```rust
enum AIProvider {
    OpenAI(openai::Provider),
    Anthropic(anthropic::Provider),
    Gemini(gemini::Provider),
    Vertex(vertex::Provider),
    Bedrock(bedrock::Provider),
    Azure(azure::Provider),
    Custom(CustomProviderRuntime),
}

struct CustomProviderRuntime {
    model: Option<Strng>,
    formats: Vec<ProviderFormatConfig>,
    target: CustomProviderTarget,
}

enum CustomProviderTarget {
    HostOverride(Target),
    BackendRef(SimpleBackendReference),
}
```

The selected provider target can resolve to:

- Built-in provider default target for existing managed providers.
- Direct `host + port`, including custom direct targets.
- `custom.backendRef`, targeting `Service` or `InferencePool`.

`custom.backendRef` targets are service-backed at runtime:

- `Service` targets call `build_service_call` directly.
- `InferencePool` targets lower to the synthetic inference service, discover `inferenceRouting`, and call
  `build_service_call` with the EPP-selected override when inference routing succeeds.

## Request Flow

The key change is to move inference routing into the concrete backend branches instead of running it before matching the
backend kind.

```text
match backend:
  Service:
    maybe run inference routing
    build_service_call with inference override, when present

  AI:
    select provider
    resolve provider target identity

    match selected provider target:
      custom backendRef Service or lowered InferencePool:
        collect target-bound backend policies
        maybe run inference routing
        build_service_call with inference override, when present

      built-in default target or direct host + port:
        use existing AI provider target construction

    merge route, backend, provider, and target-bound policies
    run the existing LLM request pipeline:
      resolve input format from backend/route AI policy and request path
      choose native upstream format
      parse request, apply AI policies, count tokens, and serialize upstream request

    apply backend policies and late auth according to existing ordering
    call upstream
    parse response using the chosen native upstream format

  MCP:
    existing behavior
```

### EPP Ordering

For `InferencePool` targets, EPP endpoint selection happens after provider and provider-target selection, but before the
existing LLM request pipeline applies AI policies and mutates the request into the final provider-native upstream form.

This design intentionally does not split AI policy application from provider-native serialization. In the current implementation,
those steps are part of the same LLM request pipeline. That means EPP may be called for a request that is later rejected
by AI policy or rate limiting, and EPP will not see prompt mutation or model aliasing performed by AI policy. This is an
acceptable tradeoff because it avoids a larger LLM pipeline refactor.

EPP sees the client/input API shape. If the EPP parser only supports OpenAI chat completions, then `custom` +
`InferencePool` is guaranteed only for input shapes that EPP can parse. Other client formats can work once EPP supports
those protocols. Direct custom targets that do not use EPP can still translate to any supported native format.

## Native Format Selection

Agentgateway should choose one native upstream format before sending the upstream request. The selected format drives
upstream request serialization and upstream response parsing.

Inputs:

- `input_format`: resolved from backend/route AI policy and request path after the top-level backend is selected.
- `formats`: declared by `custom` or inferred from the existing managed provider behavior.

Initial conversion table:

| Input format | Native target preference |
| --- | --- |
| `Completions` | `Completions`, `Messages` |
| `Messages` | `Messages`, `Completions` |
| `Responses` | `Responses`, `Completions` |
| `Embeddings` | `Embeddings` |
| `AnthropicTokenCount` | `AnthropicTokenCount` |
| `Realtime` | `Realtime` |

If no supported native target exists, reject the request with a clear configuration error.

## Controller and xDS

Do not add a dataplane `InferencePool` backend kind. Continue lowering `InferencePool` references to the synthetic
service hostname with an attached `inferenceRouting` policy.

Extend the AI provider xDS/proto model so `custom` can carry a backend target:

```proto
message AIBackend {
  enum ProviderFormat {
    COMPLETIONS = 0;
    MESSAGES = 1;
    RESPONSES = 2;
    EMBEDDINGS = 3;
    ANTHROPIC_TOKEN_COUNT = 4;
    REALTIME = 5;
  }

  message Custom {
    repeated ProviderFormatConfig formats = 1;
    optional string model = 2;
  }

  message ProviderFormatConfig {
    ProviderFormat format = 1;
    optional string path = 2;
  }

  message Provider {
    string name = 1;
    HostOverride host_override = 2;
    optional string path_override = 3;
    optional string path_prefix = 12;
    BackendReference provider_backend = 14;
    oneof provider {
      OpenAI openai = 4;
      Gemini gemini = 5;
      Vertex vertex = 6;
      Anthropic anthropic = 7;
      Bedrock bedrock = 8;
      AzureOpenAI azureopenai = 11;
      Azure azure = 13;
      Custom custom = 15;
    }
    repeated BackendPolicySpec inline_policies = 10;
  }
}
```

`provider_backend` is populated only for `custom`. A `custom.backendRef` to `InferencePool` should translate the
same way route backend refs do today: synthetic service hostname plus canonical pool port.

`dynamicForwardProxy` and `inferenceRouting` are orthogonal and should be rejected when configured together.

## Policy Attachment

Issue [#1714](https://github.com/agentgateway/agentgateway/issues/1714) added support for targeting policies at
`InferencePool` backends. That means policies attached to an `InferencePool` can be resolved through the same synthetic
service identity agentgateway already uses for inference routing.

This proposal builds on that behavior. When a `custom` AI provider targets an `InferencePool`, agentgateway should:

- Select the AI backend and provider.
- Resolve the provider target to the lowered `InferencePool` service identity.
- Collect policies attached to that target, including policies attached directly to the `InferencePool`.
- Run EPP before the existing LLM request pipeline applies LLM-affecting policies, such as token rate limits.
- Apply LLM-affecting policies before token counting and before the upstream call.
- Apply target-bound backend policies and auth after the concrete backend call target is known.

The clearest token-rate-limit configuration is to attach the policy to the `HTTPRoute`, `GRPCRoute`, `Gateway`,
or `ListenerSet` that selects the AI backend. Pool-targeted policies are also valid, but they must be collected through
the selected custom provider target before the LLM request pipeline runs.

## Migration

Direct route -> `InferencePool` remains valid and unchanged, but it continues to bypass AI policy behavior.

To apply AI policies to pool-backed traffic:

1. Create an `AgentgatewayBackend` with `spec.ai`.
2. Configure one or more `custom` providers with `formats`.
3. Point the selected `custom` provider at a `Service` or `InferencePool`.
4. Update the route to reference the AI backend.

## Test Plan

- API validation rejects empty `custom.formats`.
- API validation rejects `custom.backendRef` plus direct `host + port`.
- API validation rejects conflicting path overrides.
- `custom.backendRef` to `Service` translates to the expected provider backend reference.
- `custom.backendRef` to `InferencePool` translates to the synthetic service hostname and pool port.
- Custom format path overrides are honored.
- Service-backed custom targets use `build_service_call`.
- `custom` + `InferencePool` performs token counting and inference routing.
- `custom` + `InferencePool` sends the input request shape to EPP before upstream serialization.
- Native format selection rejects unsupported input/native format combinations.
- Dynamic forward proxy with inference routing is rejected.
- Existing managed providers and MCP behavior remain unchanged.
