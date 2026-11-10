# EP-1255: Priority-Based Backend Failover Across Service and Static Backends

- Issue: [#1255](https://github.com/agentgateway/agentgateway/issues/1255)
- Related:
  - AI backend `provider_groups` (existing reference implementation)
  - [PR #1808](https://github.com/agentgateway/agentgateway/pull/1808) (capacity-weighted P2C — recently merged; this design composes with it)
  - Prior work: [PR #1189](https://github.com/agentgateway/agentgateway/pull/1189)
  - Related discussion: [kgateway#13643](https://github.com/kgateway-dev/kgateway/issues/13643)
- Status: proposed
- Date: 5/21/2026

> **Note:** This design reflects the proposal as of the date above. The current implementation may differ as the design
> is implemented, reviewed, or revised.

## Summary

Today only AI backends support priority-based failover (via `provider_groups`). Service and Static (hostname) backends have no failover semantics — if a primary goes down the caller gets a 503. This proposal generalizes the existing AI failover machinery so that a single `AgentgatewayBackend` can express **priority-ordered groups whose members are a mix of Services and static hostnames**, with per-pod awareness across the constituent Services and the existing EWMA / eviction engine driving health-based transitions.

## Background

### Where each backend kind stands today

| Backend kind | Failover behavior today |
| --- | --- |
| **AI** | Multi-group failover via `spec.ai.groups[].providers[]`. Priority bucketed inside one `EndpointSet<NamedAIProvider>`. EWMA + eviction worker drives recovery. |
| **Service** | Pod-level traffic distribution within one Service via `EndpointSet<Endpoint>` (priority buckets used today only for locality). No mechanism to express "Service A then Service B." |
| **Static (`Opaque`)** | Single hostname/IP. No failover at all. |
| **MCP** | Targets are fanout, not failover. Sessions are stateful and pinned post-init. |
| **TCP** | Uses `SimpleBackend` (Service / Opaque / Aws). Inherits whatever the underlying backend supports — i.e. no failover. |

## Design Principles

The following principles shape the design. These are inherited from the original AI failover work and discussions with maintainers.

1. **Merged pool, not black boxes.** Combine all pods from all constituent Services — plus any static hosts — into a single shared `EndpointSet`. The gateway keeps full pod-level visibility instead of treating each constituent backend as an opaque unit. This unlocks locality-aware routing and gradual spill that aggregate-cluster designs cannot express.
2. **Priority outer, locality inner.** Within a priority tier, exhaust all locality sub-tiers before falling to the next priority. A remote primary pod is always preferred over a local fallback — failover only happens when the entire primary tier is exhausted.
3. **Reuse existing primitives.** `EndpointSet<T>`, EWMA health scoring, the eviction worker, `health::Policy` (CEL expressions, consecutive failures, thresholds), capacity-weighted P2C, and `Sampler::Drained` semantics already exist and are generic. Wire them up — do not introduce parallel mechanisms. See the [Appendix](#appendix-why-the-existing-primitives-are-sufficient) for the full inventory.
4. **Mirror the AI backend pattern.** AI groups providers into priority tiers within a single `EndpointSet<NamedAIProvider>` via `spec.ai.groups[].providers[]`. The new Backend variant uses the same shape (`spec.failover.groups[].members[]`) so the user-facing CRD vocabulary and the internal type model stay consistent across backend kinds.
5. **Mixed Service + Static members in the same group.** A failover group may contain Services, static hosts, or both. This is the load-bearing scope decision from the maintainer review — the design must enable adaptive load balancing between heterogeneous backend types, not just Service-to-Service failover.
6. **MCP is preferrably out of scope for v1.** Stateful MCP sessions would require a sub-sub-backend model (a session targets a member, and the member is itself a failover unit). Deferred to its own design.
7. **Three implementation challenges acknowledged up front.** Per-member backend policies (Service and Host typically need different TLS/auth), health-score configuration across heterogeneous member types, and weight normalization between a multi-pod Service and a single-endpoint host. Each is addressed explicitly in the [API](#api) and [Policy Attachment](#policy-attachment) sections rather than deferred.

## Goals

- Allow an `AgentgatewayBackend` to declare an ordered list of failover groups whose members may be Services *or* static hosts in any combination.
- Reuse the existing `EndpointSet<T>` priority + locality bucketing, EWMA health scoring, and eviction worker without forking new mechanisms.
- Surface a single merged endpoint pool across all members of all groups so the gateway has complete pod-level visibility.
- Apply health/eviction policy uniformly across the merged pool via the existing `AgentgatewayPolicy` resource.
- Work for HTTP routes and (in the same change, since they share `SimpleBackend`) TCP routes.
- Compose with capacity-weighted P2C from PR #1808 so capacity-based drain remains a usable operator tool.

## Non-Goals

- **MCP failover.** Stateful session pinning would require nested backend semantics ("session targets a member, member is itself a failover backend"). Deferred to its own RFC.
- **Active health checking.** Passive health (response-derived EWMA + eviction) is the only health source for v1. Active probing is a follow-up.
- **Proportional spillover** between priority tiers (e.g. 70/30 healthy split). v1 is binary at the bucket boundary: P0 takes 100% while it has any active endpoints. The architecture leaves this as a future enhancement that does not require structural change.
- **Per-group health policies.** Health policy attaches at the `AgentgatewayBackend` level for v1 and applies uniformly to all groups. Per-group policy is a future extension.
- **Cross-cluster failover.** Out of scope; constituent Services must be reachable within the same control-plane view.
- **AI backends inside failover groups.** AI backends already have their own priority groups; nesting them inside a generic failover backend is not addressed here.

## API

### Top-level shape

A new variant on the existing `Backend` proto (sibling of `Backend.Ai`, `Backend.Mcp`, `Backend.Aws`, `Backend.Static`) named `Backend.Failover`. The CRD surfaces it as `spec.failover` on `AgentgatewayBackend`. The structure mirrors `spec.ai.groups[].providers[]` from the AI failover backend.

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: resilient-api
  namespace: production
spec:
  failover:
    groups:
      # Group 0 — primary tier. Members are co-equal at this priority.
      - members:
          - service:
              name: api-primary-east
              port: 8080
          - service:
              name: api-primary-west
              port: 8080
      # Group 1 — fallback tier. Mixes a static host with a Service.
      - members:
          - host:
              hostname: api-backup.example.com
              port: 443
          - service:
              name: api-local-fallback
              port: 8080
```

`members[]` is a discriminated union; each entry is exactly one of `service` or `host`. List position carries priority — `groups[0]` is highest priority. Within a group, all members share the same priority bucket and compete for traffic via capacity-weighted P2C.

### Validation rules

- `groups` must contain at least one group, and each group at least one member.
- A given Service or hostname may appear in at most one group within a single `AgentgatewayBackend`. (Cross-Backend uniqueness is not enforced.)
- Service member `port` must match a declared service port; controller emits a status condition on mismatch.
- Host member `hostname` is required; `port` defaults to 80/443 based on the resolved TLS policy if omitted.
- The Backend's resolved connection policy (TLS, auth) must be representable per-member — see [Policy Attachment](#policy-attachment).

### Why mirror `spec.ai.groups`

This is similar to "two layers like AI": tiers as an outer list, peers as the inner list. Mirroring the AI shape literally (rather than adding `priority: N` per-member) makes tier-equality structural — you cannot accidentally have two members at the same numeric priority but treated differently. It also keeps the user-facing CRD vocabulary consistent across backend kinds.

## Runtime Design

### Internal types

Add a sibling to the existing `Backend` enum in `crates/agentgateway/src/types/agent.rs`:

```rust
pub enum Backend {
    Service(Arc<Service>, u16),
    Opaque(ResourceName, Target),
    MCP(ResourceName, McpBackend),
    AI(ResourceName, crate::llm::AIBackend),
    Aws(ResourceName, crate::aws::AwsBackendConfig),
    Failover(ResourceName, FailoverBackend),  // NEW
    Dynamic(ResourceName, ()),
    Invalid,
}

pub struct FailoverBackend {
    /// Merged endpoint pool. Buckets are priority-outer, locality-inner:
    /// bucket_index = priority_group * n_locality_buckets + locality_rank.
    pub endpoints: EndpointSet<Endpoint>,
    /// Per-group metadata used by the controller wiring and by per-member
    /// policy resolution. Order corresponds to priority (0 = highest).
    pub groups: Vec<FailoverGroup>,
}

pub struct FailoverGroup {
    pub members: Vec<FailoverMember>,
}

pub enum FailoverMember {
    Service(Arc<Service>, u16),
    Host(Target),  // resolved hostname/IP
}
```

`FailoverBackend` is also added to `SimpleBackend` (its members are themselves SimpleBackends) so TCP routes can reference it without a separate variant.

### Bucket layout

For `n` priority groups and `L` locality levels (derived from the `AgentgatewayBackend`-level LB config — see [Locality config](#locality-config-conflict-resolution)):

```text
bucket_index(member, locality_rank) = group_index_of(member) * L + locality_rank
```

A worked example (2 groups × 2 locality levels = 4 buckets). The CRD that produces this layout:

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: example
spec:
  failover:
    groups:
      # Group 0 — Priority 0 (primary tier)
      - members:
          - service: { name: svc-a, port: 8080 }
          - service: { name: svc-b, port: 8080 }
      # Group 1 — Priority 1 (fallback tier)
      - members:
          - host:    { hostname: host-c.example.com, port: 443 }
          - service: { name: svc-d, port: 8080 }
```

Assume `svc-a` and `svc-b` each have one pod in the Local locality and one in Remote (`/L`, `/R`); `svc-d` has the same; `host-c` is a single static endpoint with no locality. The merged pool's buckets are:

| Bucket | Priority | Locality | Members hashed here | When it serves traffic |
| --- | --- | --- | --- | --- |
| 0 | P0 (group 0) | Local | `[svc-a/L, svc-b/L]` | First |
| 1 | P0 (group 0) | Remote | `[svc-a/R, svc-b/R]` | When bucket 0 has no active endpoints |
| 2 | P1 (group 1) | Local | `[host-c, svc-d/L]` | When P0 is fully drained or evicted |
| 3 | P1 (group 1) | Remote | `[svc-d/R]` | Last resort |

Static-host members have no locality information, so they land at `locality_rank = 0` (the most-preferred locality bucket inside their group).

The existing `select_p2c` and `select_fallback` paths walk this index in order without modification.
### Discovery wiring

The merged pool subscribes to workload events rather than aggregating remote `EndpointSet` instances at request time. Concretely:

- For each Service member, the controller registers a watcher on that service's workload stream. Workload add/remove events translate into `EndpointSet::insert` / `remove` calls on the FailoverBackend's pool with `bucket = priority * L + locality_rank`. The same `Arc<EndpointInfo>` instance is used so capacity updates and health/eviction state remain coherent.
- For each Host member, the controller emits a single synthetic `Endpoint` into the FailoverBackend's pool at insertion time. The host's capacity defaults to 1 (overridable via the API — see [Weight normalization](#weight-normalization-mixed-member-types)).
- LB-config changes on the FailoverBackend trigger `EndpointSet::rebucket`, preserving per-endpoint health Arcs.

### Request flow

The proxy's request path does not change shape — it already dispatches on `Backend::*` in `httpproxy.rs::make_backend_call`. A new arm calls `FailoverBackend::endpoints.select_endpoint(...)` exactly as the existing Service path does. The merged pool's bucket walk produces the failover behavior.

### Health and eviction

The EWMA scoring, consecutive-failure counting, and eviction worker in `EndpointInfo` are generic and already run when callers invoke `ActiveHandle::finish_request(...)`. We do not introduce a new health engine. We do need to ensure the call site post-Service-request actually invokes `finish_request` with health-relevant info (already does for AI; Service path needs verification — see [Open Questions](#open-questions)).

For Host members specifically, the same passive-health path runs: 5xx counts as a failure, latency feeds the EWMA, eviction backs off multiplicatively. There is no Workload Discovery source for capacity on a Host, so capacity is purely a configuration value at the API surface.

## Controller and xDS

### Proto changes

- New oneof variant: `Backend.failover = FailoverBackend` carrying `repeated FailoverGroup groups`, where each group has `repeated FailoverMember members`, and each member is a oneof of `ServiceMember` or `HostMember`.
- No new XDS messages required. Workload discovery already streams per-Service workloads; the controller layer fans them into the FailoverBackend pool. Static hosts are encoded entirely in the `Backend` proto.

### Controller responsibilities

1. Resolve each Service member against the local Service cache; emit a `ResolvedRefs` status condition with per-member detail.
2. Resolve each Host member's hostname (no DNS resolution — the data plane handles that, same as today's Opaque backend).
3. Build the `FailoverBackend` runtime value with the LB config from the `AgentgatewayBackend` (or default if unset) and push it via the existing Backend xDS path.
4. Re-translate on member changes; rebucket on LB config changes.

### Locality config conflict resolution

The merged pool needs **one** locality dimensionality (number of locality buckets `L`) across all members. The FailoverBackend defines its own `loadBalancer` config that takes precedence over any per-Service LB config inside the constituent Services. Per-Service LB config is ignored within this backend's view. This is documented in the CRD and surfaced as a status condition if a constituent Service has a non-default LB config that is being overridden.

## Policy Attachment

### Health and eviction (per-Backend)

`AgentgatewayPolicy` with `targetRefs.kind: AgentgatewayBackend` carries `spec.backend.health.{unhealthyCondition, eviction.{duration, consecutiveFailures, restoreHealth}}`. Field names match the existing AI failover example in `controller/test/e2e/features/agentgateway/aibackend/testdata/failover_eviction.yaml` exactly — no new field names are introduced.

```yaml
apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: resilient-api-health
spec:
  targetRefs:
    - group: agentgateway.dev
      kind: AgentgatewayBackend
      name: resilient-api
  backend:
    health:
      unhealthyCondition: "response.code >= 500"
      eviction:
        duration: 30s
        consecutiveFailures: 3
        restoreHealth: 100
```

For initial phase this policy applies uniformly to every member of every group inside the targeted Backend.

### Per-member connection policy (TLS, auth)

a service member and a Host member typically need different TLS and auth — a cluster-local Service runs with mesh certs, a public Host needs system trust + an API key. We can resolve this by re-using the existing `BackendTrafficPolicy` inline-policy attachment that `RouteBackendReference` already supports, lifted to the member level:

```yaml
spec:
  failover:
    groups:
      - members:
          - service:
              name: api-primary
              port: 8080
          - host:
              hostname: backup.example.com
              port: 443
            policies:
              backendTLS: { name: public-tls }
              backendAuth: { key: backup-api-key }
```

`policies` on a member overrides the Backend-level defaults for that member only. Policy merging follows the existing `AgentgatewayPolicy < AgentgatewayBackend < member.policies` ordering documented on the CRD.

### Weight normalization (mixed member types)

A Service with 100 pods and a Host with one synthetic endpoint cannot be left to compete via raw P2C without normalization — the Host would receive ~1% of bucket traffic when an operator likely expects them weighted by intent, not pod count. We can use the existing per-endpoint `capacity` field as the normalization knob:

- Service members: each pod's `capacity` comes from WDS (defaults to 1 per pod) — no change.
- Host members: `capacity` defaults to 1 but is explicitly configurable on the member spec (`host.capacity: <u32>`). To give a Host equal weight to a 100-pod Service inside the same bucket, the operator sets `host.capacity: 100`.

This keeps a single weighting mechanism (capacity) end-to-end and avoids introducing a second weight dimension at the group level. Documentation will spell out the rule explicitly because the default behavior is non-obvious for users who haven't internalized P2C semantics.

## Compatibility and Migration

This is a pure addition. No existing `Backend` variant or route changes behavior by default. Users opt in by creating a new `AgentgatewayBackend` with `spec.failover` set, then pointing an `HTTPRoute` (or `TCPRoute`) `backendRefs` entry at it.

- Existing AI `provider_groups` is unaffected — same machinery, separate type.
- Existing Service-backed routes are unaffected — `Backend::Service` continues to work as today.
- A Service that's referenced both directly by Route A and as a member of a failover Backend used by Route B will run with two independent views (separate `EndpointSet` instances) unless we explicitly choose to share `Arc<EndpointInfo>`. v1 keeps them independent — see [Open Questions](#open-questions) for the sharing trade-off.

No migration is required for existing users.

## Risks and Tradeoffs

### Risk: binary failover at the bucket boundary

If P0 has 100 pods and 50 crash, the remaining 50 absorb all traffic until they too are evicted. P1 receives zero traffic until P0 is empty. This is intentional for v1 and matches the existing AI failover behavior. The merged-pool architecture is specifically designed so that proportional spillover can be added later by changing only the `best_bucket()` selection logic — no structural change required.
### Risk: heterogeneous member policies inside one group

Per-member inline policies (above) cover the main case. The remaining edge case is a member that needs *different* health policy than its siblings — v1 does not support this; per-group health policy is a future extension.

### Risk: status visibility into the merged pool

Operators may want to see "which member am I currently selecting from, and why?" via status or a debug endpoint. v1 ships standard `ResolvedRefs` conditions plus per-Backend metrics (active/rejected counts per bucket). A more detailed debug surface is a follow-up.

### Alternative considered: extend `HTTPRoute.backendRefs[].priority`

The kgateway #13643 approach adds `priority` and `fallback` fields to backend refs on the Route. We rejected this because:

- It can't be shared across multiple Routes (each Route restates the failover topology).
- It doesn't fit the "merged pool" architecture — refs from Route backendRefs resolve at request time, not at config time, so we lose pod-level unification.
- It diverges from the existing AI pattern.

## Test Plan

WIP
## Open Questions

1. **Arc sharing for Service members.** When a Service is referenced both directly by a Route and as a member of a FailoverBackend, should the two contexts share the same `Arc<EndpointInfo>` per pod? Sharing means health state is unified across views — a failure observed via the failover Backend also affects the direct-Service view. v1 keeps them independent for simplicity; revisit if operators report divergent health perception.
2. **Service backend `finish_request` wiring.** AI invokes `ActiveHandle::finish_request` after every request and so feeds EWMA / eviction. We need to confirm the Service request path (`httpproxy.rs::build_service_call`) does the same with response status, latency, and the policy-derived `eviction_time` argument. If it does not today, this is additional work inside v1's scope.
3. **DNS for Host members.** Today's `Backend::Opaque` does not handle DNS re-resolution. If a Host member's hostname changes IPs while the FailoverBackend is live, do we re-resolve or do we delegate to the existing Opaque connector? 
4. **Capacity defaults for Host members.** Default of 1 may be surprising next to a 100-pod Service. Should the CRD require explicit `capacity` on a Host that shares a group with a multi-pod Service, or is a documentation note sufficient?
5. **Per-group health policy.** Out of scope for v1 per the goals, but worth confirming this is a future extension rather than a permanent decision.

## Appendix: Why the existing primitives are sufficient

The `EndpointSet<T>` in `crates/agentgateway/src/types/loadbalancer.rs` is already generic over the endpoint type and already implements every primitive needed for cross-backend failover:

- Priority buckets (`Vec<Atomic<EndpointGroup<T>>>`) — already used for locality; we extend the bucket index.
- Per-endpoint EWMA health + consecutive-failure tracking + multiplicative-backoff eviction (`EndpointInfo`).
- Atomic `rebucket()` that preserves `Arc<EndpointInfo>` across re-distribution.
- Power-of-two-choices selection within the best non-empty bucket; locality-ordered fallback across buckets.
- As of [PR #1808](https://github.com/agentgateway/agentgateway/pull/1808): per-endpoint `capacity`, weighted P2C, and `Sampler::Drained` semantics that cause `select_fallback` to skip fully-drained buckets — i.e. **operator-driven graceful failover already works for free** when pods drain via capacity.

Two owners exist for `EndpointSet<T>` today: `Service.endpoints` and `AIBackend.providers`. This design adds a third.
