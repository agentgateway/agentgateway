# EP-1462: Dynamic Model Discovery for `/v1/models`

- Issue: [#1462](https://github.com/agentgateway/agentgateway/issues/1462)
- Status: proposed

## Background

OpenAI-compatible clients use `GET /v1/models` to discover the model IDs that
they can send to an endpoint. The response needs to answer:

> Which model IDs can this caller successfully send to this Gateway listener?

The original version of this proposal derived a catalog and its reachability
from `HTTPRoute`, `AgentgatewayPolicy`, and backend configuration. That made
HTTP routing configuration the source of truth for model discovery.

`AgentgatewayModel`, introduced by
[#2583](https://github.com/agentgateway/agentgateway/pull/2583), provides a
model-centric source of truth instead. An `AgentgatewayModel` attaches to a
Gateway listener, declares a client-facing model match, selects a concrete
provider or virtual model, and carries model policies. The controller
translates it to an xDS `ModelRoute`. The data plane groups `ModelRoute`
resources by listener and uses the resulting `ModelRouter` for both request
routing and the synthetic `/v1/models` response.

Known model inventories require no additional discovery machinery. An operator
declares each such model with an ordinary `AgentgatewayModel`, which remains
the source of truth for its client-facing name, attachment, backend, and
policies.

This proposal only addresses runtime-managed inventories that can change
without a Kubernetes configuration update. Dynamically loaded and removed
LoRA adapters are the motivating example. Gateway API Inference Extension
(GAIE) `InferencePool` is the first such source, and an OpenAI-compatible
runtime can expose its current inventory through `GET /v1/models`.

If the supported runtimes do not have a concrete need for inventory that
changes independently of Kubernetes configuration, runtime polling should not
be implemented. It is not a replacement for declaring known models.

## Decision

Dynamic discovery produces exact xDS `ModelRoute` entries. The per-listener
`ModelRouter` remains the catalog, routing table, and `/v1/models` publisher.

An `AgentgatewayModel` in explicit discovery mode acts as the discovery anchor:

- `parentRefs` select the Gateway listeners that receive discovered entries;
- a custom provider `backendRef` selects the `InferencePool`;
- visibility and policies are inherited by discovered entries; and
- `spec.discovery` opts the anchor into polling.

The anchor does not advertise its resource name or a wildcard. Each discovered
client-facing ID becomes an exact concrete `ModelRoute` targeting the pool.
The EPP remains responsible for selecting a capable endpoint within that pool.

Discovery is an explicit alternative to the existing match mode. For an
ordinary model, omitting `spec.match.model` continues to mean an exact match on
`metadata.name`. For a discovery anchor, `spec.discovery` is present,
`spec.match` must be absent, and the controller does not create the normal
`metadata.name` model route. Only IDs returned by discovery become
client-facing routes. This distinction must be represented by the API rather
than inferred from an omitted match or an annotation.

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayModel
metadata:
  name: llama-models
spec:
  parentRefs:
  - name: ai-gateway
    sectionName: llm
  discovery:
    type: OpenAI
    path: /v1/models
    interval: 30s
    staleAfter: 5m
  provider: Custom
  custom:
    backendRef:
      group: inference.networking.k8s.io
      kind: InferencePool
      name: llama-pool
    formats:
    - type: Completions
```

## Goals

- Discover model IDs from explicitly opted-in, runtime-managed
  `InferencePool` inventories.
- Feed discovered IDs into the existing per-listener `ModelRouter`.
- Keep model advertisement and request routing backed by the same entries.
- Preserve Gateway/listener scoping through `AgentgatewayModel.parentRefs`.
- Preserve per-model authorization and backend policies by inheritance.
- Keep the EPP responsible for endpoint scheduling.
- Retain last-known-good inventory during transient failures.
- Remove expired inventory atomically from routing and `/v1/models`.
- Provide status, metrics, and logs for freshness and failures.

## Non-Goals

- Poll an `InferencePool` whose model inventory is known declaratively.
- Replace ordinary `AgentgatewayModel` resources for static model inventories.
- Derive model inventory or reachability from `HTTPRoute`.
- Generate or mutate `HTTPRoute` or `AgentgatewayPolicy` resources.
- Replace the EPP or its model-aware endpoint selection.
- Require all GAIE runtimes to implement `/v1/models`.
- Support arbitrary provider-specific discovery APIs in the first milestone.
- Publish filesystem paths or other runtime-specific metadata to clients.
- Add a second Kubernetes catalog CRD.

## Architecture

```text
AgentgatewayModel discovery anchor
  parentRefs + InferencePool backend + inherited policies
                         |
                         v
InferencePool discovery controller
  selects ready pool Pods and polls /v1/models
                         |
                         v
normalized discovered model IDs
                         |
                         v
exact xDS ModelRoute resources per attached listener
                         |
                         v
listener ModelRouter
       |                                  |
       v                                  v
request routing                    GET /v1/models
```

Static and discovered models converge before publication. There is no separate
generated `/v1/models` route or model-to-header routing map. The existing data
plane creates the internal LLM route and direct response from its model table.

### Why the anchor is required

Annotating an `InferencePool` alone identifies an inventory source but not
where its models should be exposed. Deriving listeners from `HTTPRoute`
references would recreate the routing-centric design this proposal replaces.

The anchor provides explicit operator intent and gives dynamic entries the
same attachment and policy semantics as static `AgentgatewayModel` resources.

## Discovery Configuration

The proposed typed fields are:

| Field | Default | Meaning |
| --- | --- | --- |
| `spec.discovery.type` | none | Required discriminator. Initially only `OpenAI` is supported. |
| `spec.discovery.path` | `/v1/models` | Path polled on selected pool Pods. It must be an absolute path, not a URL. |
| `spec.discovery.interval` | `30s` | Successful polling interval. |
| `spec.discovery.staleAfter` | `5m` | Maximum age of last-known-good inventory after all polls fail. |

An enabled anchor must:

- use the `Custom` provider;
- reference a namespace-local `InferencePool`;
- set `spec.discovery`;
- omit `spec.match`, with API validation making `match` and `discovery`
  mutually exclusive; and
- attach only to listeners that allow `AgentgatewayModel`.

When `spec.discovery` is absent, all existing match behavior is unchanged,
including the `metadata.name` default. When it is present, `metadata.name`
identifies the anchor but is not advertised or accepted as a model ID.
Discovery is disabled by default. The controller never forwards client
credentials to discovery endpoints.

Future configuration may add:

- runtime profiles;
- include and exclude filters;
- discovery authentication;
- response size and timeout overrides within administrator limits; and
- aggregation behavior for schedulers that are not model-aware.

## Endpoint Selection and Polling

`InferencePool.spec.selector` selects Pods in the pool namespace. The
controller polls Pods that:

- match the selector;
- are in the Running phase;
- have a Pod IP; and
- have a Ready condition set to `True`.

The first `InferencePool.spec.targetPorts` entry is used initially. Multi-port
selection requires a typed discovery setting before it is supported.

Polls use bounded timeouts, response-size limits, and concurrency. Inventory
from successful endpoints is normalized and combined. Because the GAIE EPP is
model-aware, the initial aggregation mode is `union`: a model is routable when
at least one ready endpoint reports it and the EPP can select such an endpoint.

If support is added for a scheduler that cannot steer around endpoints lacking
a model, that integration must use `intersection` or another safe policy.

## Response Normalization

The first implementation accepts the OpenAI-compatible shape:

```json
{
  "object": "list",
  "data": [
    {
      "id": "tweet-summary",
      "object": "model",
      "created": 1710000000,
      "owned_by": "vllm"
    }
  ]
}
```

Only non-empty `data[].id` and a valid non-negative `created` value are needed
for routing. Unknown fields are ignored. Duplicate IDs from the same anchor are
deduplicated.

The initial client response remains the existing strictly OpenAI-compatible
model entry:

```json
{
  "id": "tweet-summary",
  "object": "model",
  "created": 1710000000,
  "owned_by": "openai"
}
```

Runtime-specific `root`, `parent`, endpoint addresses, and source metadata are
not copied into the client response. They may be retained internally for
status and metrics in a later milestone.

## Generated Model Routes

For every normalized ID and every accepted parent listener, the controller
generates an exact concrete `ModelRoute`:

```text
match.model: tweet-summary
listener: default/ai-gateway/llm
backend: InferencePool/default/llama-pool
visibility: inherited from the anchor
authorization: inherited from the anchor
policies: inherited from the anchor
```

The request model is preserved for the upstream runtime. There is no generated
base-model header or body-to-header mapping. The selected `InferencePool`
determines the scheduling domain and the EPP handles adapter-aware endpoint
selection.

The anchor also contributes a hidden internal route so its listener continues
to serve an empty `/v1/models` response before the first successful poll or
after confirmed expiry.

Updates are sent as one reconciled collection so a model is never advertised
without its route or left routable after it is removed from discovery.

## Visibility and Authorization

Discovered concrete models inherit the anchor's visibility:

- `Public` entries are directly routable and eligible for `/v1/models`.
- `Internal` entries can only be selected by virtual models and are not
  advertised.

The data plane evaluates concrete model authorization while building
`/v1/models`, so callers sharing a listener can receive different model lists
based on request credentials.

Virtual models currently cannot carry their own authorization policy and are
always included in the model list. That pre-existing limitation should be
addressed separately before virtual models are used as a tenant boundary.

## Conflicts and Filtering

Advertisement must be a subset of routability. Publication and routing filters
must not be independently configurable.

Conflict precedence within a listener is:

1. an exact user-authored, non-discovery `AgentgatewayModel`;
2. a user-authored wildcard model route;
3. a discovered exact model.

If multiple discovery anchors publish the same ID to the same listener:

- equivalent entries are deduplicated;
- conflicting backends or policies cause the discovered ID to be omitted; and
- status reports the contributing anchors.

Initial implementation may omit configurable filters. When filters are added,
one include/exclude decision controls both routing and `/v1/models`.

## Freshness and Failure Behavior

Discovery fails soft for transient control-plane errors:

- Any successful endpoint poll produces the current union for the anchor.
- If every endpoint poll fails, the last-known-good inventory remains active
  until `stale-after`.
- Once `stale-after` expires, all dynamic entries for the anchor are removed
  from routing and `/v1/models` together.
- A successful empty response is authoritative and removes existing entries.
- Disabling or deleting the anchor removes its dynamic entries immediately.
- User-authored routes and policies are never modified.

Confirmed expiry is different from a transient publisher failure: expired
models are hidden rather than advertised as unavailable.

## Status

The discovery anchor is the status owner. In addition to normal
`AgentgatewayModel` attachment conditions, discovery should expose:

| Condition | Meaning |
| --- | --- |
| `DiscoveryAccepted` | Discovery configuration and the pool reference are valid. |
| `DiscoveryReady` | At least one current model entry is available. |
| `DiscoveryStale` | Last-known-good inventory is serving after polling failures. |
| `DiscoveryConflict` | One or more IDs were suppressed because routing was ambiguous. |

The first implementation may begin with structured logs and metrics while the
experimental status shape is finalized, but invalid anchor configuration must
still set `ResolvedRefs=False`.

## Security

- Discovery is explicit and disabled by default.
- Targets are selected Pod IPs from the referenced `InferencePool`; users
  cannot configure an arbitrary discovery host.
- The path must begin with `/` and cannot contain a scheme or host.
- Polls have connection and request timeouts.
- Responses have byte, entry-count, and model-ID length limits.
- Redirects are not followed outside the selected Pod.
- Client or backend credentials are not sent during discovery.
- Error logs avoid response bodies and sensitive runtime metadata.
- NetworkPolicy must permit controller-to-Pod polling when discovery is used.

Authenticated or TLS-protected discovery requires a future typed credential
and transport configuration rather than credential-bearing annotations.

## Observability

Suggested metrics:

- `agentgateway_model_discovery_models`
- `agentgateway_model_discovery_polls_total`
- `agentgateway_model_discovery_poll_errors_total`
- `agentgateway_model_discovery_stale_anchors`
- `agentgateway_model_discovery_conflicts_total`
- `agentgateway_model_discovery_response_seconds`

Useful structured log fields include anchor, pool, endpoint count, successful
poll count, discovered model count, last success time, and stale state.
Endpoint addresses should only be logged at debug level.

## Milestones

### M1: OpenAI-compatible InferencePool discovery

- Add an explicit typed `AgentgatewayModel.spec.discovery` mode.
- Preserve the existing `metadata.name` match default for non-discovery
  models.
- Select ready Pods for the referenced pool.
- Poll standard OpenAI-compatible `/v1/models`.
- Use union aggregation for the model-aware EPP.
- Retain last-known-good state until expiry.
- Generate exact xDS `ModelRoute` entries.
- Preserve listener, backend, policy, visibility, and authorization semantics.
- Add unit, controller, and data-plane integration coverage.

### M2: Typed configuration and operational status

- Add typed status conditions.
- Add include/exclude filters shared by routing and publication.
- Add explicit port selection and administrator-bounded tuning.
- Resolve listener-scoped conflicts with detailed status.

### M3: Runtime profiles and external producers

- Add conservative profiles for runtime-specific metadata.
- Add authenticated and TLS-protected discovery.
- Evaluate other producers that can emit normalized model routes.
- Extend xDS catalog metadata only when a concrete client requirement exists.

## Test Plan

Unit tests:

- parse valid OpenAI-compatible model responses;
- reject malformed, oversized, and over-limit responses;
- deduplicate model IDs;
- select only matching Ready Pods;
- combine successful endpoint responses by union;
- retain last-known-good data during transient failures;
- expire stale data;
- validate discovery anchor configuration; and
- ensure deleting or disabling an anchor removes dynamic entries.

Controller tests:

- an anchor generates exact model routes for each discovered ID;
- generated routes use the anchor's listener and `InferencePool`;
- policies, authorization, and visibility are inherited;
- the anchor itself is not advertised;
- a successful empty inventory produces an empty model list;
- static exact models take precedence over discovered entries; and
- invalid configuration reports `ResolvedRefs=False`.

Integration tests:

- a fake runtime exposes `/v1/models`;
- the Gateway's `/v1/models` returns the discovered IDs;
- a request using a discovered ID reaches the referenced pool;
- changing runtime inventory updates routing and discovery atomically;
- authorization filters the response per caller; and
- stale expiry removes a model from both routing and discovery.

## Alternatives

### Derive the catalog from HTTPRoute

Rejected. HTTP routes describe transport reachability rather than the complete
client-facing model contract. Reconstructing aliases, dynamic adapters,
authorization, and body-based selection from routes and CEL is incomplete and
creates a second source of truth.

### Annotate InferencePool without an AgentgatewayModel anchor

Rejected. The pool does not identify the Gateway listener, visibility, or
model policies. Inferring those from references recreates route-derived
scoping.

### Put a static model list on InferencePool

Rejected. If an operator knows the inventory, ordinary `AgentgatewayModel`
resources express it with the required listener, visibility, authorization,
and policy semantics. A second list on `InferencePool` would duplicate that
source of truth. Runtime discovery is reserved for inventory that changes
independently of Kubernetes configuration.

### Generate AgentgatewayPolicy mappings

Rejected as the default. It exposes runtime inventory as mutable Kubernetes
configuration, creates ownership conflicts, and separates advertisement from
the actual model router.

### Add a ModelCatalog CRD

Deferred. `AgentgatewayModel` plus xDS `ModelRoute` already provides the needed
catalog and routing boundary. Another CRD would add synchronization and
conflict semantics without solving runtime discovery.

### Move discovery into the EPP

Deferred. The EPP schedules requests, while agentgateway owns listener
attachment, authorization, and the client-facing `/v1/models` response.
Inventory may be shared with the EPP in the future if the GAIE API defines an
appropriate contract.

## Open Questions

- Is runtime-managed inventory required by a supported `InferencePool`
  implementation, or are declarative `AgentgatewayModel` resources sufficient
  for the initial milestone?
- Should the initial `spec.discovery` shape be extensible to non-OpenAI
  protocols, or expose only the fields required by the first supported
  runtime?
- Should discovery status extend `AgentgatewayModelStatus.Parents` or add a
  top-level condition list?
- How should authenticated discovery reuse backend TLS and credential policy
  without forwarding inference credentials unnecessarily?
- Does multi-port discovery select a named port or an index?
- Should xDS add optional `owned_by` metadata, or keep the current minimal
  response?
- How should authorization be added to virtual models?
- When should non-GAIE dynamic producers be supported?
