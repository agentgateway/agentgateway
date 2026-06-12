# EP-1462: Extensible Model Catalog for /v1/models

- Issue: [#1462](https://github.com/agentgateway/agentgateway/issues/1462)
- Status: proposed

## Background

Many OpenAI-compatible clients call `GET /v1/models` to discover the model IDs they can send in request bodies.
Agentgateway already knows about models from several places, but it does not currently expose a scoped synthetic
`/v1/models` response that reflects those configured or discovered models.

Issue [#1462](https://github.com/agentgateway/agentgateway/issues/1462) describes this broader need across multiple
agentgateway modes and backend types:

- Standalone config can define models through `llm.models[].name`.
- Kubernetes config can define models through `AgentgatewayBackend` AI provider settings.
- AI policies can define user-facing aliases through `modelAliases`.
- Gateway API routes can make only a subset of backends reachable from a given Gateway, listener, hostname, or path.

The common requirement is not specific to one backend type. Agentgateway needs an internal model catalog that can be
populated by multiple sources and then published as a scoped OpenAI-compatible `/v1/models` response.

Gateway API Inference Extension (GAIE) `InferencePool` support adds an important dynamic source. A common GAIE
deployment uses one `Gateway` and multiple `InferencePool` backends to serve multiple base models behind a single
endpoint. Each `InferencePool` represents endpoints that share a common base model, and LoRA adapters associated with
the base model are served by the same backend inference servers as the base model.

An `InferencePool` is not tied to one model server implementation. At the time of writing, GAIE documentation lists
vLLM, Triton with the TensorRT-LLM backend, SGLang, and custom protocol-compatible engines as supported model-server
options. This means agentgateway should treat `/v1/models` discovery as a runtime capability, not as a vLLM-specific
assumption.

With agentgateway, body-based routing can be implemented natively with an `AgentgatewayPolicy` in the `PreRouting`
phase. The policy reads the OpenAI request body model name, maps it to a base model, and sets the
`X-Gateway-Base-Model-Name` request header. `HTTPRoute` rules then match that header and forward the request to the
appropriate `InferencePool`.

For example, a client may send:

```json
{
  "model": "tweet-summary",
  "messages": []
}
```

The effective model routing map may be:

```text
tweet-summary -> meta-llama/Llama-2-7b-hf
```

Agentgateway sets:

```http
X-Gateway-Base-Model-Name: meta-llama/Llama-2-7b-hf
```

The matching `HTTPRoute` forwards the request to the `InferencePool` for `meta-llama/Llama-2-7b-hf`.

Route-only discovery can find the base model route key from `HTTPRoute` header matches, but it cannot discover
user-facing LoRA adapter IDs or aliases unless those names are also represented in static route or policy config.
For some OpenAI-compatible model servers, the backend can expose this information through `GET /v1/models`:

```json
{
  "id": "tweet-summary",
  "object": "model",
  "owned_by": "vllm",
  "root": "/adapters/vineetsharma/qlora-adapter-Llama-2-7b-hf-TweetSumm_4",
  "parent": "meta-llama/Llama-2-7b-hf"
}
```

This endpoint contains the user-facing model ID (`tweet-summary`) and, when the runtime reports it, the parent/base
model (`meta-llama/Llama-2-7b-hf`). A GAIE `InferencePool` publisher can contribute that dynamic inventory to the same
internal catalog used by configured model publishers. When the runtime does not expose enough lineage in
`/v1/models`, the publisher should use explicit profile configuration, pool metadata, or static alias sources instead
of assuming vLLM response extensions are available.

The EPP remains responsible for endpoint scheduling. It discovers endpoints and uses runtime signals, such as metrics
and readiness, to choose an endpoint for a request. This proposal keeps model catalog publication in agentgateway
because the gateway owns the client-facing API surface, Gateway API scoping, and `AgentgatewayPolicy` integration.

## Motivation

A `/v1/models` implementation should answer this client-facing question:

> Which model IDs can I send to this Gateway endpoint?

It should not only answer implementation-specific questions such as:

> Which base model headers appear in attached `HTTPRoute`s?

or:

> Which provider model is configured on one backend?

Different sources can contribute valid answers:

- Standalone `llm.models[]` can publish statically configured model IDs.
- `AgentgatewayBackend` can publish configured provider models.
- `modelAliases` can publish user-facing aliases.
- GAIE `InferencePool` discovery can publish dynamically loaded LoRA adapters.
- A future CRD or external integration can publish additional catalog entries.

A source-specific implementation would solve only one aspect of
[#1462](https://github.com/agentgateway/agentgateway/issues/1462) and would likely duplicate scoping, conflict
resolution, generated route, and status logic. An extensible internal catalog lets agentgateway add new publishers over
time while keeping `/v1/models` behavior consistent.

GAIE with dynamically loaded LoRA serving is the first high-value dynamic publisher. The model server may know about
adapters that are loaded, unloaded, or updated independently of Gateway API configuration. Requiring users to duplicate
those adapter IDs in `AgentgatewayPolicy` mappings is operationally fragile and can drift quickly.

### Goals

- Introduce an internal model catalog that can be populated by multiple publisher implementations.
- Generate scoped OpenAI-compatible `GET /v1/models` responses from the internal catalog.
- Support a GAIE `InferencePool` publisher as the first dynamic publisher.
- Poll opted-in `InferencePool` endpoints for `GET /v1/models` when the selected runtime profile supports it.
- Normalize returned model entries through runtime profiles for vLLM, SGLang, Triton OpenAI-compatible frontends, and
  custom protocol-compatible engines.
- Derive LoRA/user-facing model IDs and base model route keys from runtime profile rules or explicit user
  configuration.
- Preserve the existing GAIE routing model based on `X-Gateway-Base-Model-Name` and `HTTPRoute` header matches.
- Scope published models to the Gateway/listener/route context that can actually reach the associated backend.
- Allow users to opt in or out of dynamic discovery per `InferencePool`.
- Allow operators to filter which discovered model IDs are published or included in generated routing maps.
- Avoid mutating user-authored `AgentgatewayPolicy` resources by default.
- Provide status and observability for catalog sources, freshness, conflicts, and generated resources.

### Non-Goals

- Replace the EPP or change EPP request scheduling behavior.
- Require the GAIE community to add `/v1/models` scraping to the EPP.
- Make dynamic discovery mandatory for all `InferencePool`s.
- Implement every possible model publisher in the first milestone.
- Support arbitrary provider-specific model catalog APIs in the initial implementation.
- Reverse-engineer arbitrary CEL expressions from existing `AgentgatewayPolicy` resources.
- Require every GAIE-compatible model server to implement `GET /v1/models`.
- Guarantee that every OpenAI-compatible server returns the same schema extensions as vLLM, such as `parent` and
  `root`.
- Delete or overwrite user-authored `AgentgatewayPolicy` resources.

## Implementation Details

This feature has five separate pieces:

1. Internal model catalog and publisher interface.
2. Source-specific model publishers.
3. Catalog aggregation, conflict handling, and reachability scoping.
4. Generated `/v1/models` publication.
5. Optional generated routing maps for sources that need model-to-route-key translation.

### Architecture

Conceptual flow:

```text
Standalone config publisher
  reads llm.models[]
        |
AgentgatewayBackend publisher
  reads configured AI provider models and aliases
        |
AgentgatewayPolicy alias publisher
  reads explicit modelAliases or simple routing maps
        |
GAIE InferencePool publisher
  polls runtime-supported /v1/models from model-server endpoints
        |
Future publishers
  read ModelCatalog CRDs or external inventory
        |
        v
Internal Model Catalog
        |
        v
Scoped GET /v1/models direct response
        |
        v
Optional model ID -> route key transformation
```

The catalog is the shared boundary. Publishers should not generate `/v1/models` routes directly. They should emit
normalized catalog entries with enough source and reachability metadata for the central catalog reconciler to publish
models consistently.

### Model Catalog Entries

Conceptual shape:

```go
type ModelCatalogEntry struct {
    ID            string
    RouteKey      string
    SourceKind    string
    SourceName    types.NamespacedName
    SourceSection string
    Parent        string
    OwnedBy       string
    Metadata      map[string]string
    Freshness     ModelFreshness
    Reachability  []RouteScope
}

type RouteScope struct {
    Gateway     types.NamespacedName
    Listener    string
    Hostnames   []string
    PathContext string
}
```

Field meanings:

| Field | Meaning |
| --- | --- |
| `ID` | User-facing model ID to publish in `/v1/models`. |
| `RouteKey` | Value used for routing. For GAIE this is the base model used in `X-Gateway-Base-Model-Name`. |
| `SourceKind` | Publisher source type, such as `InferencePool`, `AgentgatewayBackend`, or `StandaloneConfig`. |
| `SourceName` | Namespace/name of the source when applicable. |
| `SourceSection` | Optional section or sub-backend identifier. |
| `Parent` | Parent/base model metadata to include when safe. Usually equal to `RouteKey` for LoRA entries. |
| `OwnedBy` | Optional OpenAI-compatible owner metadata. |
| `Metadata` | Optional source metadata. Sensitive fields should not be published by default. |
| `Freshness` | Last-seen and stale state for dynamic publishers. |
| `Reachability` | Gateway/listener/path contexts where this model is available. |

Examples:

```text
GAIE runtime-discovered LoRA:
  ID: tweet-summary
  RouteKey: meta-llama/Llama-2-7b-hf
  SourceKind: InferencePool
  SourceName: default/llama2-pool
  Parent: meta-llama/Llama-2-7b-hf
```

```text
AgentgatewayBackend configured model:
  ID: gpt-4o
  RouteKey: gpt-4o
  SourceKind: AgentgatewayBackend
  SourceName: tenant-a/openai-backend
```

```text
Alias:
  ID: friendly-model
  RouteKey: provider-model-name
  SourceKind: AgentgatewayBackend
  SourceName: tenant-a/openai-backend
```

### Model Publisher Interface

Each publisher should answer two questions:

1. Which model catalog entries does this source contribute?
2. Where are those entries reachable?

Conceptual interface:

```go
type ModelPublisher interface {
    Name() string
    Entries(ctx krt.HandlerContext) []ModelCatalogEntry
}
```

Publisher implementations can use existing collections and reference indexes. The central catalog reconciler should be
responsible for deduplication, conflict handling, generated route behavior, and final response shaping.

Initial publishers:

| Publisher | Source | Milestone | Notes |
| --- | --- | --- | --- |
| `InferencePoolModelPublisher` | Opted-in GAIE `InferencePool` endpoints | M1 | First dynamic publisher and primary motivation. |
| `AgentgatewayBackendModelPublisher` | `AgentgatewayBackend` AI provider config | M2 | Covers Kubernetes configured backend models. |
| `StandaloneModelPublisher` | standalone `llm.models[]` | M2/M3 | Covers standalone config mode. |
| `PolicyAliasPublisher` | explicit `modelAliases` or simple static maps | M2/M3 | Covers user-facing aliases when statically configured. |
| `ExternalModelCatalogPublisher` | future CRD or external inventory | Future | Lets users integrate non-standard discovery systems. |

### User Configuration

The extensible catalog should not require every source to use the same configuration mechanism. Each publisher can own
its source-specific opt-in and tuning knobs while emitting the same normalized entries.

For the initial GAIE `InferencePool` publisher, annotations avoid adding a new CRD before the behavior is proven.

Example:

```yaml
apiVersion: inference.networking.k8s.io/v1
kind: InferencePool
metadata:
  name: llama2-pool
  namespace: default
  annotations:
    agentgateway.dev/model-discovery: "enabled"
    agentgateway.dev/model-discovery-path: "/v1/models"
    agentgateway.dev/model-discovery-port: "8000"
    agentgateway.dev/model-discovery-runtime: "auto"
    agentgateway.dev/model-discovery-mode: "intersection"
    agentgateway.dev/model-discovery-publish-exclude: "internal-*,experimental-*"
spec:
  selector:
    matchLabels:
      app: llama2-pool
```

Proposed GAIE publisher annotations:

| Annotation | Default | Description |
| --- | --- | --- |
| `agentgateway.dev/model-discovery` | `disabled` | Enables model discovery for the `InferencePool` when set to `enabled`. |
| `agentgateway.dev/model-discovery-path` | `/v1/models` | HTTP path to poll on model-server endpoints. |
| `agentgateway.dev/model-discovery-port` | first target port | Endpoint port used for discovery. |
| `agentgateway.dev/model-discovery-scheme` | `http` | Discovery scheme. Initial support is `http`; `https` can be added with backend TLS settings. |
| `agentgateway.dev/model-discovery-runtime` | `auto` | Runtime profile used to parse responses and derive routing keys. Supported values include `auto`, `openai`, `vllm`, `sglang`, `triton-openai`, and `custom`. |
| `agentgateway.dev/model-discovery-route-key-source` | profile default | How to derive `RouteKey`. Supported values include `profile`, `response.parent`, `response.id`, `inferencepool`, `annotation`, `separator`, and `custom`. |
| `agentgateway.dev/model-discovery-base-model` | empty | Explicit base model route key for runtimes that do not report parent/base model lineage. |
| `agentgateway.dev/model-discovery-id-jsonpath` | profile default | Custom JSON path for model IDs when `runtime=custom`. |
| `agentgateway.dev/model-discovery-parent-jsonpath` | profile default | Custom JSON path for parent/base model metadata when `runtime=custom`. |
| `agentgateway.dev/model-discovery-mode` | `intersection` | Aggregation mode across endpoints. Supported values: `intersection`, `union`, `firstHealthy`. |
| `agentgateway.dev/model-discovery-interval` | controller default | Optional per-pool polling interval. |
| `agentgateway.dev/model-discovery-publish` | `enabled` | Controls whether discovered models are published through generated `/v1/models`. |
| `agentgateway.dev/model-discovery-publish-include` | empty | Optional comma-separated glob list of model IDs to publish. Empty means all discovered IDs are eligible. |
| `agentgateway.dev/model-discovery-publish-exclude` | empty | Optional comma-separated glob list of model IDs to hide from generated `/v1/models`. Exclude wins over include. |
| `agentgateway.dev/model-discovery-routing` | `internal` | Controls routing map generation. Supported values: `internal`, `agentgatewayPolicy`, `disabled`. |
| `agentgateway.dev/model-discovery-routing-include` | empty | Optional comma-separated glob list of model IDs to include in generated routing maps. Empty means all discovered IDs are eligible. |
| `agentgateway.dev/model-discovery-routing-exclude` | empty | Optional comma-separated glob list of model IDs to exclude from generated routing maps. Exclude wins over include. |

If this feature graduates, these annotations can become typed fields on an agentgateway-owned policy or parameters
resource. The discovery behavior should not require changes to the upstream GAIE `InferencePool` API.

### GAIE InferencePool Publisher

The `InferencePoolModelPublisher` is the first dynamic publisher. It discovers live model inventory from opted-in
model servers behind a GAIE `InferencePool`.

Agentgateway already watches `InferencePool`s when GAIE support is enabled. For an opted-in pool, agentgateway should
discover candidate model-server endpoints using the pool's selector and target port information, following the same
Kubernetes source of truth used for backend endpoint discovery.

The publisher should:

- Watch opted-in `InferencePool`s.
- Resolve selected pods or endpoint slices for each pool.
- Filter to ready endpoints.
- Select a runtime profile from annotations, endpoint labels, or the default profile.
- Construct a polling URL from scheme, endpoint address, port, and path.
- Poll endpoints on a bounded interval with timeout and jitter.
- Cache the last successful model response per endpoint.
- Emit normalized catalog entries.
- Expose discovery errors without immediately deleting the last known-good catalog.

The implementation can run in the controller process or in a dedicated sidecar process colocated with the controller.

Controller process advantages:

- Direct access to existing informers and reference indexes.
- Simpler deployment and fewer moving parts.
- Easier integration with generated xDS resources and status.

Sidecar process advantages:

- Isolates outbound polling and parser bugs from the main controller.
- Can be independently tuned for concurrency, timeouts, and network policy.
- May be reused by future non-controller deployments.

The recommended initial implementation is in the controller process unless polling volume or isolation requirements
force a sidecar split.

### Runtime Profiles

The GAIE publisher should use runtime profiles so discovery behavior is explicit and extensible. A profile defines:

- Whether `GET /v1/models` discovery is supported.
- Which response fields identify user-facing model IDs.
- Which response fields, annotations, or static settings identify the route key.
- Which metadata fields are safe to preserve in the catalog.
- Whether LoRA adapter lineage can be inferred from the response.

Initial profiles:

| Profile | Discovery behavior | Route key default | LoRA/alias support |
| --- | --- | --- | --- |
| `openai` | Parse standard OpenAI-style `data[].id` entries. | `data[].id` | Publishes listed IDs but does not infer base-model lineage. |
| `vllm` | Parse OpenAI-style entries plus vLLM extensions such as `parent` and `root` when present. | `data[].parent`, else `data[].id` | Supports dynamically reported LoRA adapter IDs when `parent` is present. |
| `sglang` | Parse OpenAI-style entries and SGLang-reported LoRA entries when enabled by the runtime. | `data[].parent`, else `data[].id` | Supports dynamically reported LoRA adapter IDs when `parent` is present. |
| `triton-openai` | Parse Triton OpenAI-compatible model IDs. | `data[].id` or explicit base-model config | Conservative default because Triton does not currently provide the same LoRA metrics support GAIE uses for affinity. |
| `custom` | Parse fields selected by user-provided JSON paths or future CEL expressions. | explicit config | Allows protocol-compatible engines to integrate without agentgateway code changes. |

`auto` should select a profile using explicit annotations first, then endpoint or pod engine labels such as
`inference.networking.k8s.io/engine-type`, then the controller default. Auto-detection must be conservative: if a
runtime does not report parent/base model lineage, agentgateway should not invent aliases. Operators can still provide
`agentgateway.dev/model-discovery-base-model` or `agentgateway.dev/model-discovery-route-key-source` when they know how
the discovered model IDs should map to a route key.

This keeps the first implementation useful for vLLM and SGLang LoRA discovery, while still allowing Triton and custom
OpenAI-compatible model servers to publish catalog entries safely.

### Model Response Normalization

When a runtime profile supports OpenAI-compatible model listing, the GAIE publisher should accept standard
OpenAI-style responses:

```json
{
  "object": "list",
  "data": [
    {
      "id": "tweet-summary",
      "object": "model",
      "parent": "meta-llama/Llama-2-7b-hf"
    }
  ]
}
```

For each entry, normalize:

| Field | Source | Required | Meaning |
| --- | --- | --- | --- |
| `id` | `data[].id` | yes | User-facing model ID. |
| `routeKey` | runtime profile or explicit config | yes | Base model route key or provider route key. |
| `parent` | `data[].parent` | no | Parent/base model metadata. |
| `ownedBy` | `data[].owned_by` | no | Provider metadata to preserve in `/v1/models`. |
| `root` | `data[].root` | no | Adapter/root metadata to preserve when safe. |
| `sourceName` | `InferencePool` namespace/name | yes | Pool that discovered the model. |
| `endpoint` | endpoint address | yes | Endpoint that reported the model. |
| `lastSeen` | poll timestamp | yes | Freshness indicator. |

If the selected profile derives `routeKey` from `parent` and `parent` is absent, `routeKey` should default to `id`.
This preserves behavior for non-LoRA base models. Other profiles may require an explicit base-model annotation or custom
route-key rule before they can generate alias-to-base-model routing maps.

For the example response above:

```text
ID: tweet-summary
RouteKey: meta-llama/Llama-2-7b-hf
SourceKind: InferencePool
SourceName: default/llama2-pool
```

The publisher should also include an implicit base model entry when appropriate:

```text
meta-llama/Llama-2-7b-hf -> meta-llama/Llama-2-7b-hf
tweet-summary -> meta-llama/Llama-2-7b-hf
```

### Catalog Filters

Operators may need to hide or disable individual model IDs even when an `InferencePool` is opted in. Examples include
experimental adapters, tenant-private LoRAs, canary models, or aliases that should remain routable for known clients
but should not be advertised through `/v1/models`.

The GAIE publisher should support separate filters for publication and generated routing:

```yaml
metadata:
  annotations:
    agentgateway.dev/model-discovery-publish-include: "meta-llama/*,tweet-summary-*"
    agentgateway.dev/model-discovery-publish-exclude: "internal-*,experimental-*"
    agentgateway.dev/model-discovery-routing-exclude: "disabled-*"
```

Filter behavior:

- Filters apply after runtime profile normalization and before catalog aggregation.
- Include filters are allow lists. Empty include filters mean all discovered model IDs are eligible.
- Exclude filters are deny lists. Exclude filters win over include filters.
- Publication filters affect generated `GET /v1/models` responses.
- Routing filters affect generated model ID to route-key maps.
- Filtering a model from publication should not automatically remove it from generated routing maps.
- Filtering a model from routing should not automatically remove it from generated `/v1/models`.
- If a model is published but excluded from generated routing, status should warn that the model may not be routable
  through agentgateway-generated policy.

The initial filter language should be simple comma-separated globs matched against normalized model IDs. A future typed
configuration surface can add label selectors or CEL expressions if model servers or external catalog publishers expose
structured metadata suitable for richer filtering.

### Dynamic Aggregation Modes

Dynamic publishers may see different model inventories across endpoints. The default should be safe.

`intersection` mode:

- Publish only models reported by all currently ready endpoints in the source.
- Best default for correctness when the EPP may choose any endpoint in the pool.
- Avoids publishing a LoRA ID that only some endpoints can serve.

`union` mode:

- Publish any model reported by at least one ready endpoint.
- Useful when the scheduler is model-aware and can route a request only to endpoints that serve the requested model.
- Risky if endpoint selection is not constrained by model availability.

`firstHealthy` mode:

- Publish the response from the first successful ready endpoint.
- Operationally simple.
- Useful for homogeneous pools.
- Can be misleading if endpoints drift.

Initial recommendation: implement `intersection` first and leave `union` behind an explicit annotation.

### Catalog Aggregation and Scoping

The internal catalog should not be global-only. It must be scoped by reachability. A model should appear in a generated
`/v1/models` response only when the associated source is reachable through that Gateway/listener/path context.

For route-based Kubernetes sources, scoping should account for:

- which `HTTPRoute`s reference the backend or `InferencePool`;
- which Gateway/listener those routes are accepted by;
- route hostnames and listener hostnames;
- `sectionName`, allowed routes, protocol, and accepted status;
- path context when a Gateway exposes separate model surfaces such as `/cheap-llms` and `/expensive-llms`.

This prevents one tenant or listener from seeing models that are only reachable through another route.

Duplicate handling:

- If the same `ID` and `RouteKey` are published by multiple sources in the same scope, deduplicate the response entry.
- If the same `ID` maps to different `RouteKey` values in the same scope, report a catalog conflict and do not generate
  an ambiguous routing map for that ID.
- If two sources publish the same `ID` with different metadata but equivalent routing, prefer deterministic metadata
  precedence and surface the source list in status.

### Generated `/v1/models`

For each Gateway/listener/path context with a non-empty scoped catalog, agentgateway can generate an internal route:

```http
GET /v1/models
```

The response should be OpenAI-compatible:

```json
{
  "object": "list",
  "data": [
    {
      "id": "tweet-summary",
      "object": "model",
      "owned_by": "vllm",
      "parent": "meta-llama/Llama-2-7b-hf"
    }
  ]
}
```

Generated route behavior:

- only emit routes for HTTP-capable listeners;
- do not emit a generated route when a user-authored `GET /v1/models` route already exists for the same listener unless
  an explicit override is configured;
- return `Content-Type: application/json`;
- prefer last known-good catalog data when a dynamic publisher temporarily fails;
- include freshness metadata in controller status or metrics, not necessarily in the OpenAI response body.

### Generated Routing Map

Some publishers only need to publish model IDs. Others also need a generated request routing map. GAIE `InferencePool`
discovery needs this when the user-facing model ID differs from the base model route key.

For each scoped model catalog, agentgateway can generate the request body model map:

```text
tweet-summary -> meta-llama/Llama-2-7b-hf
meta-llama/Llama-2-7b-hf -> meta-llama/Llama-2-7b-hf
```

The recommended default is to generate this internally as xDS/IR policy attached to the relevant Gateway/listener/path
context. This avoids mutating user-owned `AgentgatewayPolicy` resources.

Equivalent generated transformation:

```yaml
traffic:
  phase: PreRouting
  transformation:
    request:
      set:
      - name: X-Gateway-Base-Model-Name
        value: |
          {
            "meta-llama/Llama-2-7b-hf": "meta-llama/Llama-2-7b-hf",
            "tweet-summary": "meta-llama/Llama-2-7b-hf"
          }[string(json(request.body).model)]
```

If `agentgateway.dev/model-discovery-routing: agentgatewayPolicy` is set, agentgateway may create a Kubernetes
`AgentgatewayPolicy` instead. Generated policies must:

- Use deterministic names.
- Use owner references where possible.
- Carry labels and annotations that identify them as generated.
- Use server-side apply with a stable field manager.
- Refuse to overwrite user-authored policies.
- Be deleted or disabled when discovery is disabled.
- Report conflicts through status.

Generated `AgentgatewayPolicy` is useful for transparency but should not be the default because it turns runtime
discovery state into user-visible configuration and creates ownership conflicts.

### Failure Behavior

Dynamic discovery should fail soft for data-plane availability:

- If a poll fails, keep using the last known-good catalog until a configurable TTL expires.
- If the TTL expires, remove stale models from generated `/v1/models` and generated routing maps.
- If all endpoints fail discovery but the backend is still routable, do not remove user-authored routes or policies.
- If catalog generation produces an invalid or ambiguous map, fail closed for generated routing and surface a status
  error.

Example status conditions:

| Condition | Meaning |
| --- | --- |
| `ModelCatalogAccepted` | Catalog source config is valid. |
| `ModelCatalogReady` | At least one publisher contributed usable entries. |
| `ModelCatalogStale` | Catalog is using last known-good data after recent dynamic publisher failures. |
| `ModelCatalogConflict` | Duplicate or generated route/policy conflict requires user action. |

Status can initially be exposed through logs and metrics if no appropriate status target exists, then promoted to a
typed status surface later.

### Security Considerations

The model catalog can reveal adapter names, tenant workloads, provider names, or filesystem-like `root` metadata.
Dynamic discovery must be opt-in and scoped.

Security requirements:

- Do not enable dynamic discovery by default.
- Do not publish models from sources that are not reachable from the requesting Gateway/listener/path context.
- Allow operators to exclude sensitive model IDs, such as private LoRAs, from generated `/v1/models` responses.
- Allow operators to disable metadata passthrough fields such as `root`.
- Support network policy patterns where the controller is allowed to reach model-server pods only when discovery is
  explicitly enabled.
- Add timeouts, response size limits, and JSON parse limits for polling.
- Avoid forwarding user credentials to model discovery endpoints.
- Consider future support for service-account, mTLS, or configured headers if `/v1/models` requires authentication.

### Observability

Add metrics such as:

- `agentgateway_model_catalog_entries`
- `agentgateway_model_catalog_filtered_entries`
- `agentgateway_model_catalog_conflicts_total`
- `agentgateway_model_catalog_publish_errors_total`
- `agentgateway_model_discovery_polls_total`
- `agentgateway_model_discovery_poll_errors_total`
- `agentgateway_model_discovery_stale_catalogs`
- `agentgateway_model_discovery_response_seconds`

Useful log fields:

- publisher name
- source kind and namespace/name
- Gateway/listener/path scope
- endpoint address for dynamic publishers
- discovered model count
- filtered model count and filter reason
- aggregation mode
- catalog generation
- last successful poll time
- generated route or policy conflict reason

### Milestones

M1: internal catalog plus GAIE `InferencePool` publisher.

- Implement catalog entry normalization, scoping, conflict handling, and generated `/v1/models`.
- Implement opt-in `InferencePool` polling for runtime-supported `/v1/models`.
- Implement initial runtime profiles for `openai`, `vllm`, `sglang`, `triton-openai`, and `custom`.
- Implement per-pool publication and routing filters for discovered model IDs.
- Support internal generated routing maps for GAIE base-model header routing.

M2: configured Kubernetes model publishers.

- Publish models from `AgentgatewayBackend` AI provider config.
- Publish explicit aliases from supported `modelAliases` fields.
- Share the same generated `/v1/models` and conflict handling logic.

M3: standalone and external publishers.

- Publish standalone `llm.models[]` entries.
- Evaluate a typed `ModelCatalog` CRD or external publisher API if needed.

### Test Plan

Unit tests:

- Normalize entries from each publisher type.
- Parse `/v1/models` responses through `openai`, `vllm`, `sglang`, `triton-openai`, and `custom` profiles.
- Default `routeKey` to `id` when the selected profile uses `parent` but `parent` is absent.
- Use explicit base-model and custom route-key configuration for runtimes that do not report parent lineage.
- Reject malformed responses and oversized payloads.
- Aggregate endpoint catalogs in `intersection`, `union`, and `firstHealthy` modes.
- Apply publication and routing include/exclude filters after normalization.
- Verify exclude filters win over include filters.
- Deduplicate equivalent catalog entries.
- Detect duplicate IDs with conflicting route keys.
- Scope catalogs to accepted route/listener/Gateway/path contexts.
- Detect user-defined `GET /v1/models` route collisions.
- Generate deterministic direct response JSON.

Controller tests:

- Opted-in `InferencePool` creates scoped catalog entries.
- Opted-out `InferencePool` is ignored.
- Stale poll keeps last known-good catalog until TTL.
- Discovery disablement removes generated dynamic entries.
- Publication filters remove matching models from generated `/v1/models`.
- Routing filters remove matching models from generated routing maps.
- Generated resources are updated when model-server inventory changes.
- Configured `AgentgatewayBackend` entries and discovered `InferencePool` entries can coexist in one catalog.

Integration tests:

- Run fake OpenAI-compatible model servers for standard, vLLM-style, SGLang-style, and Triton-style responses.
- Verify `GET /v1/models` returns discovered LoRA IDs.
- Verify a request with `model: <lora-id>` sets the base-model header and routes to the correct `InferencePool`.
- Verify runtimes without parent lineage require explicit base-model or route-key configuration before generating alias
  routing maps.
- Verify configured backend models also appear when reachable.
- Verify a listener or tenant only sees models reachable from that listener's routes.

## Alternatives

### GAIE-Only Model Discovery

Build the feature specifically around `InferencePool` polling and generated GAIE routing maps.

Pros:

- Simplest initial implementation.
- Directly solves the dynamic LoRA model discovery gap for GAIE.
- Avoids designing a general abstraction before the first use case is proven.

Cons:

- Does not address the broader scope of [#1462](https://github.com/agentgateway/agentgateway/issues/1462).
- Duplicates future `/v1/models` logic for other backend types.
- Risks making `InferencePool` the wrong long-term abstraction boundary.

### Route-Only Model Discovery

Discover models from `HTTPRoute` rules that match `X-Gateway-Base-Model-Name` and reference `InferencePool`s.

Pros:

- Simple.
- No outbound polling.
- No new runtime model catalog.

Cons:

- Only discovers base model route keys.
- Misses LoRA IDs and aliases known only to the model server or body-routing policy.
- Can accidentally aggregate models too broadly if not carefully scoped.

### Static AgentgatewayPolicy Alias Extraction

Scan `AgentgatewayPolicy` resources for simple literal maps from `json(request.body).model` to base model names.

Pros:

- Avoids polling.
- Can recover user-facing aliases when they are explicitly configured.
- Aligns with current agentgateway body-based routing.

Cons:

- Only works for simple static CEL maps.
- Cannot discover dynamically loaded LoRAs.
- Still requires users to maintain alias config manually.

### EPP-Published Model Inventory

Extend the EPP to scrape or publish `/v1/models` inventory and have agentgateway consume that state.

Pros:

- Co-locates endpoint inventory with the scheduler.
- Avoids duplicate endpoint watches.

Cons:

- Expands the EPP beyond request scheduling and metrics-driven endpoint selection.
- May not be accepted by the GAIE community.
- Couples agentgateway model publication to EPP feature availability.

### User-Managed ModelCatalog CRD

Add an agentgateway-owned `ModelCatalog` CRD where users or external automation publish model-to-route-key mappings.

Pros:

- Explicit.
- Auditable.
- Avoids controller outbound polling.

Cons:

- Introduces another API.
- Does not remove the operational burden of keeping model inventory in sync.
- Less compelling for dynamically loaded LoRA adapters.

## Open Questions

- Should the initial dynamic discovery implementation run in the controller process or a sidecar process?
- Where should catalog status live for source objects that agentgateway should not modify, such as `InferencePool`?
- Should generated `/v1/models` include vLLM extension fields such as `root` and `parent`, or only OpenAI core fields?
- Should stale discovered models be hidden immediately after TTL expiry or marked unavailable in a future extended
  response?
- Is `intersection` too conservative for pools where the EPP can already avoid endpoints that do not serve a requested
  LoRA?
- How should discovery authentication be configured for model servers that protect `/v1/models`?
- Should generated model-routing maps coexist with user-authored `AgentgatewayPolicy` mappings, or should one take
  precedence?
- Do we need a typed agentgateway API before supporting generated Kubernetes `AgentgatewayPolicy` resources?
- Which configured model sources should be implemented immediately after the GAIE `InferencePool` publisher?
