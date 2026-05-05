# EP-288: AI Policies for InferencePool-backed Custom Providers

- Issue: [#288](https://github.com/agentgateway/agentgateway/issues/288)
- Related: [#1714](https://github.com/agentgateway/agentgateway/issues/1714)
- Status: proposed

## Background

Issue [#288](https://github.com/agentgateway/agentgateway/issues/288) asks for AI policies such as token-based rate
limiting to apply when traffic ultimately routes to a Gateway API Inference Extension (GAIE) `InferencePool`.

Today, agentgateway already supports two important pieces of that story:

- It can route directly to `InferencePool` backends by lowering them to a synthetic `Service` hostname plus
  `inferenceRouting` policy.
- It can apply AI request and response processing for `AgentgatewayBackend.spec.ai` backends, including request
  parsing, prompt policies, token counting, and response translation.

The problem is that these features live in separate layers of the current execution flow.

At a high level, the current `make_backend_call` flow is:

```text
maybe inference routing
build concrete backend call
apply backend policies and auth
run LLM request processing
call upstream
run LLM response processing
```

This ordering works when a route points either:

- Directly at a `Service` or `InferencePool`, or
- Directly at a managed LLM provider such as OpenAI or Anthropic.

It does not work well when we need both:

- AI-aware processing on the request and response path, and
- `InferencePool` endpoint selection on the concrete upstream call.

That gap is the core of issue [#288](https://github.com/agentgateway/agentgateway/issues/288).

There is a second design problem behind the same issue. Current LLM provider types mix two distinct concerns:

1. Vendor identity, such as `openai` or `anthropic`.
2. Native API formats the upstream actually supports, such as `Completions`, `Responses`, or `Messages`.

This works well for managed providers, but it is awkward for self-hosted backends. Examples:

- A self-hosted OpenAI-compatible server may support `Completions` but not `Responses`.
- Ollama-style deployments may support both `Completions` and `Messages`.
- Future backends may speak additional transports such as gRPC while still fitting behind an `InferencePool`.

Today, users cannot express those capabilities precisely. Using `openai` for a self-hosted backend implicitly says more
than "it speaks something OpenAI-like", it also implies support for the output and translation behavior agentgateway
currently associates with the OpenAI provider type.

That ambiguity matters on the response path. Agentgateway already resolves the input format from the configured route
type and request path, but for self-hosted mixed capability backends it does not always know which native format the
upstream response will use.

Related work reinforces the same layering. Other GAIE-conformant implementations distinguish between:

- A route-layer AI abstraction that applies AI-aware processing, and
- A backend-layer `InferencePool` abstraction that performs endpoint selection.

This proposal adopts this separation of concerns in agentgateway, while keeping the existing `AgentgatewayBackend`
API model instead of introducing a new route CRD.

## Motivation

We want one request flow to support all of the following together:

- AI request parsing and route typing.
- Prompt policies and AI request/response transformations.
- Token-based rate limiting and telemetry.
- Provider selection and failover.
- Concrete backend dispatch to a `Service` or `InferencePool`.
- GAIE endpoint selection for the final upstream call.

That requires two changes:

1. A provider model that can express self-hosted native capabilities without pretending the upstream is a managed
   vendor.
2. An execution model that processes LLM semantics before `InferencePool` routing, while still using one mutable
   request instead of recursively calling the full backend pipeline.

### Goals

- Allow `AgentgatewayBackend.spec.ai` to select a provider and still route that provider to a `Service` or
  `InferencePool`.
- Add a `custom` LLM provider for self-hosted backends with explicit native format support.
- Make native upstream format selection deterministic so the request path and response path stay aligned.
- Refactor the LLM branch of backend execution into phases so AI processing happens before `InferencePool` routing.
- Reuse the existing controller-side `InferencePool` lowering to synthetic `Service` plus `inferenceRouting` policy.
- Keep existing managed providers such as `openai`, `anthropic`, `gemini`, `vertexai`, `azure`, and `bedrock`
  unchanged for users.
- Preserve the current MCP behavior.

### Non-Goals

- Redesign MCP request execution.
- Change the upstream GAIE `InferencePool` API or the EPP protocol.
- Introduce a new agentgateway route CRD solely for this feature.
- Implement a generic pluggable protocol parser framework in the first milestone.
- Support arbitrary gRPC-based custom providers in the first milestone.
- Allow recursive provider targets such as custom provider -> `AgentgatewayBackend` AI backend in the first milestone.
- Remove or deprecate existing managed provider types.

## Implementation Details

This feature has five separate pieces:

1. A `custom` LLM provider API with explicit native capabilities.
2. An internal provider capability and target model.
3. A phased LLM execution path in `make_backend_call`.
4. Controller and xDS support for provider backend targets.
5. Tests and migration guidance.

### Architecture

Conceptual flow:

```text
client request
  |
  v
route policies resolve input format
  |
  v
AI backend selects provider
  |
  v
provider chooses native upstream format
  |
  v
resolve concrete provider target identity
  - built-in provider default target
  - direct host + port
  - custom provider backendRef
  |
  v
collect LLM-affecting policy inputs
  - route and backend policies
  - provider inline policies
  - provider target policies, when applicable
  |
  v
LLM request processing
  - parse request
  - apply AI policies
  - tokenize
  - translate request to chosen upstream format
  |
  v
if target is InferencePool-backed service:
  run inference routing
  |
  v
call upstream
  |
  v
parse upstream response using chosen native format
  |
  v
translate response back to client format
  |
  v
apply response AI policies and logging
```

The key property is that the request stays one mutable `Request` object through the LLM path. We do not need the MCP
"outer request creates N inner requests" execution model here.

### API Design

The public API change is intentionally small.

The existing managed providers will be kept as-is:

- `openai`
- `azureopenai`
- `azure`
- `anthropic`
- `gemini`
- `vertexai`
- `bedrock`

A new `custom` provider will be added with the following shape:

```go
type LLMProvider struct {
    ...
    Custom      *CustomProvider    `json:"custom,omitempty"`
    
}

type CustomProvider struct {
    BackendRef       *gwv1.BackendObjectReference `json:"backendRef,omitempty"`
    SupportedFormats []ProviderFormat             `json:"supportedFormats"`
}
```

Validation rules:

- `custom` is added to the `ExactlyOneOf` provider list.
- `custom` must specify exactly one of `backendRef` or `host + port` (existing LLMProvider fields).
- `supportedFormats` is required and must contain at least one format.
- Direct `host + port` uses the existing shared LLMProvider fields for custom providers that do not refer to a
  Kubernetes backend.
- In M1, `backendRef` may target only namespace-local `Service` or `InferencePool`.

Example:

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: llm-failover
spec:
  ai:
    groups:
    - providers:
      - name: openai-primary
        openai:
          model: gpt-4o-mini
      - name: onprem-ollama
        custom:
          backendRef:
            group: inference.networking.k8s.io
            kind: InferencePool
            name: ollama-pool
          supportedFormats:
          - Completions
          - Messages
        policies:
          auth:
            secretRef:
              name: ollama-auth
      - name: plain-vllm
        custom:
          backendRef:
            group: ""
            kind: Service
            name: vllm
            port: 8000
          supportedFormats:
          - Completions
```

#### Supported Formats

This proposal deliberately uses `supportedFormats`, not `supportedRoutes`.

`RouteType` and `supportedFormats` are related but not the same thing:

- Route policy resolves how agentgateway interprets the incoming client request.
- Supported formats declare which native request and response formats the upstream backend can speak directly.

`ProviderFormat` should be a new enum, not a reuse of `RouteType`, because some existing route types are gateway
processing behaviors rather than native provider formats.

Initial `ProviderFormat` values:

- `Completions`
- `Messages`
- `Responses`
- `Embeddings`
- `AnthropicTokenCount`
- `Realtime`

Excluded from `ProviderFormat`:

- `Models`
  Agentgateway handles generated `/v1/models` itself.
- `Detect`
  This is a gateway mode for best-effort parsing and telemetry, not a native upstream wire format.
- `Passthrough`
  This is a gateway routing mode, not a provider capability declaration.

### Why `custom` Instead of Extending Existing Providers

Adding `backendRef` directly to current providers such as `openai` was considered and would help with the `InferencePool` use case,
but it would not fix the deeper modeling issue:

- Existing provider types mix vendor identity and native capability assumptions.
- Self-hosted backends often need different native format sets than the managed vendor they resemble.
- Response parsing becomes ambiguous when a self-hosted provider is "OpenAI-like" but does not actually support the
  full OpenAI format set.

The `custom` provider addresses both concerns:

- It can target `InferencePool`, `Service`, or direct `host` + `port`.
- It explicitly declares native supported formats.

Existing providers remain the ergonomic shorthand for managed backends. `custom` becomes the precise tool for
self-hosted and mixed-capability backends.

### Internal Provider Model

The runtime should move toward a unified internal provider shape.

Conceptual model:

```rust
struct ResolvedProvider {
    name: Strng,
    native_formats: BTreeSet<ProviderFormat>,
    target: ProviderTarget,
    path_override: Option<Strng>,
    path_prefix: Option<Strng>,
    tokenize: bool,
    inline_policies: Vec<BackendPolicy>,
    implementation: ProviderImplementation,
}

enum ProviderTarget {
    BuiltInDefaultTarget,
    HostOverride(Target),
    BackendRef(SimpleBackendReference),
}

enum ProviderImplementation {
    Managed(AIProvider),
    Custom(CustomProviderRuntime),
}
```

Externally, only `custom` needs explicit `supportedFormats` in M1. Internally, both built-ins and custom providers
should lower into the same execution model:

- Built-ins infer `native_formats` from current provider behavior.
- Custom providers get `native_formats` from configuration.

This keeps the phased refactor focused on one runtime shape instead of introducing a second AI execution path.

### Native Format Selection

Current managed providers infer response format from provider identity. That is not sufficient for self-hosted mixed
capability providers.

For the LLM path, agentgateway should choose a concrete native upstream format before request translation and before
response parsing.

Inputs:

- `input_format`
  resolved from route policies and request path.
- `supported_formats`
  declared by `custom` or inferred for built-ins.

Selection rules for structured AI requests:

| Input format | Allowed native targets, in order |
| --- | --- |
| `Completions` | `Completions`, `Messages` |
| `Messages` | `Messages`, `Completions` |
| `Responses` | `Responses`, `Completions` |
| `Embeddings` | `Embeddings` |
| `AnthropicTokenCount` | `AnthropicTokenCount` |
| `Realtime` | `Realtime` |

Behavior:

- If `input_format` is supported natively, use it.
- Otherwise, choose the first supported fallback from the table above.
- If no supported fallback exists, reject the request with a clear configuration error.

Examples:

```text
input=Messages, supported=[Messages, Completions]
  => use Messages upstream
```

```text
input=Responses, supported=[Completions]
  => translate Responses -> Completions upstream
```

```text
input=Responses, supported=[Messages]
  => reject; no supported conversion path in M1
```

This selected native format drives both:

- request translation to the upstream wire format;
- response parsing and translation back to the client-visible format.

That resolves the "input format was known, output format was ambiguous" problem for self-hosted providers.

### `make_backend_call` Refactor

This proposal does not call the full `make_backend_call` twice for the LLM path.

Instead, it refactors the LLM branch into phases while preserving one mutable request.
That feasibility distinction is important. For the LLM path, agentgateway is still operating on one logical request and
one mutable `Request` object. The phased refactor is therefore mostly a reordering problem: select the provider, choose
the native upstream format, resolve the provider target identity, collect any LLM-affecting target policies, mutate the
request, and then optionally perform inference routing.

Current conceptual order:

```text
inference routing
build concrete backend call
apply backend auth and backend policies
run LLM request pipeline
upstream call
run LLM response pipeline
```

Proposed conceptual order:

```text
select backend
if AI:
  select provider
  resolve client input format
  choose native upstream format
  resolve provider target identity
  collect target-scoped LLM-affecting policies
  apply early auth
  run LLM request pipeline
  if target uses inference routing:
    call EPP via ext-proc for upstream endpoint selection
  apply target-bound backend policies
  upstream call
  run LLM response pipeline using chosen native format
  inference response hook
else:
  existing non-AI behavior
```

Suggested helper boundaries:

```rust
async fn select_provider_plan(...)
async fn apply_early_backend_auth(...)
async fn prepare_llm_request(...)
async fn finalize_provider_backend_call(...)
async fn process_llm_response(...)
```

#### Early, Target-Bound, and Late Auth

Auth currently mixes concerns that happen at different points in the flow.

For this refactor, split backend auth into three stages:

1. Early auth
   Apply before LLM request setup.
   Used for auth that providers may inspect or rewrite.
   Examples: `Passthrough`, `Key`.

2. Target-bound auth
   Apply after the final `BackendCall` target exists.
   Needed for auth that depends on the concrete call target.
   Examples: `Gcp`, `Azure`.

3. Late auth
   Apply after all request mutation is complete.
   Used for request signing.
   Example: `Aws`.

This preserves current behaviors such as:

- Anthropic auth header rewriting expectations.
- GCP and Azure token acquisition against the final upstream destination.
- AWS signing at the very end of request mutation.

### Provider Target Resolution

The selected provider can resolve to one of three target modes:

1. Built-in provider default target
   Existing behavior for built-in providers.

2. Direct host + port
   Existing behavior when `host` and `port` are set.

3. Backend reference
   New behavior for `custom.backendRef`.

In M1, provider backend refs may target:

- `Service`
- `InferencePool`

Recursive targets such as provider -> `AgentgatewayBackend` are intentionally deferred.

### Controller and xDS Changes

The controller should follow the same high-level pattern used by route backend refs today.

#### `InferencePool` Lowering

Do not add a new dataplane `InferencePool` backend kind.

Keep the existing controller behavior:

- Route or backend references to `InferencePool` resolve to the synthetic inference service hostname.
- `inferenceRouting` policy is attached to that synthetic service target.

This is already the right abstraction boundary for endpoint picking. The new feature should reuse it.

#### AI Provider Translation

Extend the AI provider xDS/proto model so a provider can optionally carry a backend target.

Conceptual proto change:

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
    repeated ProviderFormat supported_formats = 1;
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

Notes:

- `provider_backend` is general in the IR, but in M1 it is populated only for `custom`.
- This follows the existing `provider_backend` naming pattern already used in telemetry-related resources.
- `provider_backend` should lower to the same runtime `SimpleBackendReference` shape used elsewhere.

#### Translation Rules

When translating `custom.backendRef`:

- `Service`
  resolve to a `BackendReference_Service`.
- `InferencePool`
  resolve exactly as route backend refs do today, which yields the synthetic service hostname and canonical pool port.

This gives the runtime a normal service target. Any attached `inferenceRouting` policy is then discovered through the
same service-target policy merge used elsewhere.

### LLM Request and Response Processing

For built-in providers, current request and response translation behavior remains the source of truth.

For `custom`, request and response handling should be format-driven:

- `Completions`
  parse and serialize OpenAI chat completions-compatible structures.
- `Responses`
  parse and serialize OpenAI responses-compatible structures.
- `Messages`
  parse and serialize Anthropic messages-compatible structures.
- `Embeddings`
  parse and serialize embeddings-compatible structures.
- `AnthropicTokenCount`
  parse and serialize Anthropic token count-compatible structures.
- `Realtime`
  keep current realtime handling model; defer richer custom transport support.

The selected native format decides which request codec and response codec run for the upstream leg.

Client-visible format remains determined by route policy and request path.

### Policy Merging

The current AI merge order should be preserved conceptually:

```text
provider defaults
  < backend-level AI/backend policies
  < provider inline policies
```

For provider backend targets, service-targeted policies discovered through the existing bind and policy lookup model
continue to apply when the concrete backend call is finalized.

This is especially important for `InferencePool`, because the inference routing policy is attached to the synthetic
service target rather than embedded inside the AI provider.

Provider target policy collection should happen before LLM request processing, even though target-bound request
mutations should be deferred until the final backend call is known. In other words, resolving the provider target
identity is not the same as running inference routing. This ordering gives the LLM phase access to any LLM-affecting
policies that are attached to the concrete provider target, while still ensuring endpoint picking happens after the
request has been parsed, counted, and translated.

That distinction is the important connection to issue [#1714](https://github.com/agentgateway/agentgateway/issues/1714).
If `InferencePool` policy target refs resolve to the same synthetic service target used by inference routing, then
pool-targeted backend AI policies can be merged into the LLM request plan before token counting and prompt processing.
Non-LLM target-bound policies, such as backend TLS, backend auth, and `inferenceRouting`, should still run later during
final backend call construction.

Token rate limiting is currently a traffic policy that is lifted into the LLM request path. For the first milestone, the
most straightforward and least surprising configuration is to attach token rate limits to the `HTTPRoute`, `GRPCRoute`,
`Gateway`, or `ListenerSet` that selects the AI backend. If issue [#1714](https://github.com/agentgateway/agentgateway/issues/1714)
also allows pool-targeted traffic policies, then provider target policy collection must explicitly include those
target-scoped token rate limits in the LLM request plan.

#### InferencePool Policy Attachment

Issue [#1714](https://github.com/agentgateway/agentgateway/issues/1714) is the policy attachment companion to this
design. Today, `AgentgatewayPolicy.targetRefs` does not allow `InferencePool`, so users cannot directly attach traffic
or backend policies to an `InferencePool` by name.

Supporting `InferencePool` as a policy target should resolve the target to the same synthetic service identity used for
`InferencePool` routing. That gives users a first-class way to say "this policy applies to the pool", while still
letting the dataplane consume normal service-targeted policies after the controller lowers the pool.

This proposal still needs the LLM execution refactor even after [#1714](https://github.com/agentgateway/agentgateway/issues/1714)
is implemented. Policy attachment makes the policy selectable for an `InferencePool`, while the execution refactor makes that
policy meaningful for AI workloads by ensuring requests enter the LLM pipeline before `InferencePool` endpoint
selection.

### User Experience and Migration

This feature intentionally does not change the behavior of:

- routes that directly reference `InferencePool`;
- existing managed providers;
- MCP.

Instead, it adds a new way to express "AI-aware traffic that ultimately uses an `InferencePool` backend."

Recommended migration:

1. Create an `AgentgatewayBackend` with `spec.ai`.
2. Use one or more `custom` providers with `supportedFormats`.
3. Point the `custom` provider at a `Service` or `InferencePool`.
4. Update the route to reference the AI backend instead of referencing the `InferencePool` directly.

Example:

```yaml
apiVersion: gateway.networking.k8s.io/v1
kind: HTTPRoute
metadata:
  name: chat
spec:
  parentRefs:
  - name: gw
  rules:
  - backendRefs:
    - group: agentgateway.dev
      kind: AgentgatewayBackend
      name: chat-backend
---
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: chat-backend
spec:
  ai:
    groups:
    - providers:
      - name: onprem
        custom:
          backendRef:
            group: inference.networking.k8s.io
            kind: InferencePool
            name: llama-pool
          supportedFormats:
          - Completions
```

Direct route -> `InferencePool` remains valid, but it continues to bypass AI policies by design.

### MCP

MCP is out of scope for the behavioral change in this proposal.

The MCP path is different in a more fundamental way than the LLM path. The first MCP phase handles one incoming client
request, but later MCP processing may create N independent upstream `Request` objects, each of which performs its own
concrete backend execution. That is materially different from the LLM case, where agentgateway keeps modifying one
mutable `Request` all the way through provider selection, translation, and the final upstream call.

Because of that difference, MCP does not naturally fit the same "phase reorder one request" model used for LLM. Its
current layered execution architecture is still appropriate even if the helper boundaries become cleaner.

An optional follow-up cleanup may split execution helpers into:

- `make_full_backend_call`
- `make_simple_backend_call`

That would improve code clarity, but it is not required to fix [#288](https://github.com/agentgateway/agentgateway/issues/288).

### Milestones

M1: `custom` provider plus phased LLM execution for HTTP self-hosted backends.

- Add `custom` provider to the Kubernetes API.
- Add `supportedFormats` and `backendRef`.
- Add `provider_backend` and custom provider support to the IR/xDS model.
- Refactor the LLM path in `make_backend_call` into phases.
- Support `Service` and `InferencePool` provider targets.
- Keep existing managed providers unchanged externally.
- Keep MCP behavior unchanged.

M2: unify built-ins on the same internal capability model.

- Lower built-in providers into the same internal provider capability shape used by `custom`.
- Reduce provider-specific branching in request and response planning where practical.
- Improve validation and observability for format selection failures.

M3: future transport and parser extensibility.

- Evaluate custom gRPC provider support.
- Integrate with broader parser framework work if needed.
- Revisit whether built-ins should optionally expose explicit native capability overrides.

### Test Plan

Unit tests:

- `custom` provider validation rejects empty `supportedFormats`.
- `custom` provider validation rejects `backendRef` plus `host`/`port`.
- Native format selection chooses the direct native match when available.
- Native format selection chooses the correct fallback when direct support is absent.
- Unsupported input/native format combinations fail clearly.
- `custom` provider backed by `InferencePool` still resolves to the synthetic service target.
- Early, target-bound, and late auth each run in the intended phase.

Runtime tests:

- Existing managed providers behave unchanged.
- `custom` provider backed by `Service` applies token counting and AI policies.
- `custom` provider backed by `InferencePool` applies token counting and AI policies.
- `custom` provider backed by `InferencePool` still performs inference routing.
- `Messages` input with `supportedFormats=[Messages,Completions]` uses `Messages` upstream.
- `Responses` input with `supportedFormats=[Completions]` translates request and response through
  `Completions`.
- `Responses` input with `supportedFormats=[Messages]` fails deterministically.

Controller tests:

- `custom.backendRef` to `Service` translates to the expected provider backend reference.
- `custom.backendRef` to `InferencePool` translates to the synthetic service hostname and pool port.
- Invalid backend kinds are rejected in status.
- Cross-namespace provider backend refs are rejected in M1.

End-to-end tests:

- `HTTPRoute` -> AI backend -> `custom` provider -> `InferencePool` applies token RL.
- `HTTPRoute` -> AI backend -> `custom` provider -> `InferencePool` applies prompt guard and response processing.
- Direct `HTTPRoute` -> `InferencePool` remains unchanged and does not get AI policy behavior unless wrapped in an AI
  backend.
- MCP regression tests continue to pass unchanged.

## Alternatives

### Add `backendRef` to Existing Providers Only

Add `backendRef` to `openai`, `anthropic`, and the other current provider types without introducing `custom`.

Pros:

- Small API change.
- Directly solves the `InferencePool` targeting problem.

Cons:

- Does not solve the capability-modeling problem for self-hosted backends.
- Still conflates vendor identity with supported formats.
- Leaves response-format ambiguity for mixed-capability providers.

### Special-case `InferencePool` Inside the AI Runtime

Teach the dataplane a new `InferencePool` backend kind specifically for AI providers.

Pros:

- Could avoid one layer of controller lowering.

Cons:

- Duplicates logic the controller already has.
- Makes `InferencePool` a dataplane concern instead of a controller-lowered service abstraction.
- Works against the current architecture and the desired layering.

### Call `make_backend_call` Twice for LLM

Use a recursive outer AI call and inner concrete backend call, similar to the MCP shape.

Pros:

- Conceptually simple.

Cons:

- Unnecessary for the LLM path because the request stays one mutable object.
- Risks duplicating auth, inference, and response processing in the wrong phase.
- Makes the LLM path look like MCP even though their execution models differ materially.

### OpenAI-compatible Self-hosted Wrapper Only

Introduce a narrow self-hosted OpenAI-compatible provider wrapper and defer `custom`.

Pros:

- Smaller first implementation.

Cons:

- Does not cover `Messages`-only or mixed-format providers.
- Does not solve the capability-modeling concern John raised.
- Likely leads to a second API redesign later.

## Open Questions

- Which exact native format conversion pairs should be supported in M1 beyond the table in this proposal?
- Should `Realtime` remain built-in only until transport extensibility is designed more fully?
- Do we want to add user-facing status conditions to `AgentgatewayBackend`, or is logging and metrics sufficient for the
  first milestone?
- After `custom` lands, do we want built-in providers to stay vendor-centric forever, or should they eventually compile
  fully into the same capability declaration model?
