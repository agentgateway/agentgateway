# Schema doc-comment plan (agentgateway/website#160, bullet 1)

Scratch/working spec — not for commit. Adds `///` doc comments to config struct
fields that currently produce no `description` in `schema/config.json`. `schemars`
turns each `///` into the field's schema `description`, which the docs site renders
in the config schema explorer.

Workflow: add the `///` lines below, run `make gen` (or `make generate-schema`),
diff `schema/config.json`, then the website picks it up on its next build.

**174 fields** across 56 types (2 `Duration` fields excluded — see caveats).
All descriptions are grounded in code by the research agents; confidence noted only
where below `high`.

## Caveats / decisions
- **`Duration.secs` / `Duration.nanos` — SKIP.** Duration serializes as a string
  (`"10s"`) via `core/src/serdes.rs`; the `secs`/`nanos` object `$def` is referenced
  only once in the schema and is a schemars fallback, not user-facing. Fix the stray
  reference upstream instead of documenting these.
- **`Config.url`** (log-store database) — med confidence; the `Config` type is
  `telemetry::log_store::Config` used by `RawLogging.database`. Verify the field is a
  DB connection URL before finalizing wording.
- Match existing doc-comment style in each struct (some LLM structs lead with the
  field name, e.g. `/// name is referenced from ...`); keep one line where possible.

---

## crates/agentgateway/src/lib.rs

### RawConfig (:147)
- enableIpv6 — Enable IPv6 address resolution and binding. Defaults to true.
- caAddress — Address of the Certificate Authority used to issue SPIFFE certificates.
- caAuthToken — Authentication token for communicating with the Certificate Authority.
- xdsAddress — Address of the xDS control plane used for dynamic configuration.
- xdsAuthToken — Authentication token for communicating with the xDS control plane.
- namespace — Kubernetes namespace for this gateway instance.
- gateway — Name of this gateway. Required when xDS is configured.
- trustDomain — SPIFFE trust domain for this gateway.
- serviceAccount — Kubernetes service account for this gateway, used in its SPIFFE identity.
- clusterId — Identifier for the cluster this gateway runs in. Defaults to "Kubernetes".
- network — Network name for this gateway, used for locality-aware routing.
- connectionTerminationDeadline — Maximum time to wait for connections to close gracefully during shutdown.
- connectionMinTerminationDeadline — Minimum time to allow for graceful connection termination. Defaults to zero.
- workerThreads — Number of worker threads for the async runtime. Accepts a number or a string such as "auto".
- tracing — Distributed tracing configuration.
- logging — Logging configuration, including filter, level, format, and custom fields.
- metrics — Metrics configuration, including metric removal and custom fields.
- backend — Configuration for upstream connections, including keepalives, timeouts, and pooling.
- hbone — HBONE (HTTP/2 CONNECT tunnel) protocol configuration.

### RawLogging (:363)
- filter — CEL expression that selects which requests are logged.
- fields — Custom fields to add to or remove from log entries.
- level — Log level, or a comma-separated list of per-module levels (e.g. "info" or "info,agent_core=trace").
- format — Log output format: "text" or "json".
- database — Log-store database configuration; enables request logging to a database backend.

### RawHBONE (:313)
- windowSize — HTTP/2 per-stream flow-control window size in bytes. Defaults to 4 MiB.
- connectionWindowSize — HTTP/2 connection-level flow-control window size in bytes. Defaults to 16 MiB.
- frameSize — HTTP/2 maximum frame size in bytes. Defaults to 1 MiB.
- poolMaxStreamsPerConn — Maximum concurrent streams per pooled connection. Defaults to 100.
- poolUnusedReleaseTimeout — Duration after which unused pooled connections are released.

### RawTracing (:341)
- otlpEndpoint — OTLP collector endpoint URL for exporting traces.
- headers — HTTP headers to include on OTLP trace exports, such as authentication headers.
- otlpProtocol — OTLP transport protocol: "grpc", "grpc-web", or "http/protobuf".
- fields — Custom fields to add to or remove from trace spans.

### RawLoggingFields (:404)
- remove — Field names to remove from log entries.
- add — Map of field name to a CEL expression that computes the value to add to logs.

### RawMetrics (:387)
- remove — Metric names to exclude from collection.
- fields — Custom fields to add to all metrics.

### RawMetricFields (:394)
- add — Map of field name to a CEL expression that computes the value to add to metrics.

### RawMcpConfig (:334)
- sessionTtl — Time to live for MCP sessions before they are closed automatically. Defaults to 30 minutes.

### BackendConfig (:241)
- keepalives — TCP keepalive configuration for upstream connections.
- connectTimeout — Maximum time to wait when establishing a connection to an upstream. Defaults to 10 seconds.

---

## crates/agentgateway/src/types/agent.rs

### KeepaliveConfig (:2968)
- enabled — Enable TCP keepalive probes on backend connections. Defaults to true.
- time — Idle time before the first keepalive probe is sent.
- interval — Time between successive keepalive probes.
- retries — Number of unacknowledged probes before the connection is considered dead.

### RouteMatch (:1034)
- headers — HTTP headers that must match for this route to apply.
- path — Path match rule (exact, prefix, or regex). Defaults to a "/" prefix match.
- method — HTTP method that must match for this route to apply.
- query — Query parameters that must match for this route to apply.

### RouteName (:740)
- name — Name identifying this route.
- namespace — Namespace scoping this route, used in fully qualified `namespace/name` references.
- ruleName — Specific rule within the route, for targeted policy references.
- kind — Resource kind used in policy target references.

### ResourceName (:877)
- name — Name identifying this resource.
- namespace — Namespace scoping this resource, used in fully qualified `namespace/name` references.

### ListenerTarget (:848)
- gatewayName — Name of the gateway this target references.
- gatewayNamespace — Namespace of the gateway this target references.
- listenerName — Specific listener within the gateway; if unset, targets the gateway itself.
- port — Port to target, as an alternative to listenerName.

### ListenerSetTarget (:2495)
- name — Name of the listener set resource.
- namespace — Namespace of the listener set resource.
- section — Specific listener within the listener set to target.

### HeaderMatch (:1055)
- name — HTTP header or pseudo-header name (such as `:method`) to match.
- value — Exact or regex pattern the header value must match.

### QueryMatch (:1063)
- name — Query parameter name to match.
- value — Exact or regex pattern the query parameter value must match.

---

## crates/agentgateway/src/types/local.rs

### LocalBind (:1201)
- listeners — Named listeners bound on this port, which may use different protocols and TLS.
- tunnelProtocol — Protocol used to tunnel backend connections, such as Direct or HBONE.

### LocalListener (:1225)
- name — Name identifying this listener, referenced by `gateways: gateway-name/listener-name`.
- namespace — Namespace scoping this listener.
- protocol — Protocol this listener accepts: HTTP, HTTPS, TCP, TLS, or HBONE.
- tls — TLS configuration, used with the HTTPS and TLS protocols.
- routes — HTTP routes attached directly to this listener.
- tcpRoutes — TCP routes attached directly to this listener.
- policies — Gateway-level policies applied to all traffic on this listener.

### LocalRoute (:1313) and LocalAttachedRoute (:1143) — shared wording
- name — Name identifying this route.
- namespace — Namespace scoping this route.
- ruleName — Specific rule within this route.
- matches — Conditions (path, method, headers, query) that select this route.
- policies — Route-level policies applied before backend selection.
- backends — Weighted backends this route forwards traffic to.

### LocalTCPRoute (:2002) and LocalAttachedTCPRoute (:1157) — shared wording
- name — Name identifying this TCP route.
- namespace — Namespace scoping this TCP route.
- ruleName — Specific rule within this TCP route.
- policies — TCP-level policies applied to traffic on this route.
- backends — Weighted backends this TCP route forwards traffic to.

### LocalRouteBackend (:1328)
- weight — Relative weight for load balancing across backends. Defaults to 1.
- policies — Backend-level policies such as TLS, authentication, and transformations.

### LocalTCPRouteBackend (:2015)
- weight — Relative weight for load balancing across TCP backends. Defaults to 1.
- policies — Backend-level policies for TCP backends, such as TLS, authentication, and tunneling.

### LocalRouteGroup (:1307)
- name — Identifier for this route group, referenced by delegating routes.
- routes — HTTP routes grouped together for delegation and reuse.

### FullLocalBackend (:1342)
- name — Identifier for this backend, referenced by routes.
- policies — Backend-level policies such as TLS, authentication, transformations, and health checks.

### LocalTLSServerConfig (:1254)
- cert — Path to the TLS certificate file (leaf certificate, or CA certificate in dynamic CA mode).
- key — Path to the TLS private key file.
- root — Path to a root CA certificate file used to validate client certificates.

### LocalLLMProviderDefaults (:423)
- defaults — Request payload fields to set when not already present in the request.
- overrides — Request payload fields to set, overriding any existing values in the request.
- transformation — CEL expressions that compute request payload fields, overriding existing values.
- requestHeaders — Headers to add, set, or remove on requests to the LLM provider.
- responseHeaders — Headers to add, set, or remove on responses from the LLM provider.
- tls (alias backendTLS) — TLS configuration for connecting to the LLM provider.
- auth — Authentication configuration for connecting to the LLM provider.
- health — Outlier detection and health checking for this provider backend.
- backendTunnel — Tunneling configuration for connecting to the LLM provider.
- promptCaching — Cache-point insertion for LLM providers that support prompt caching.

### LocalLLMParams (:855)
- awsRegion — AWS region to use for the Bedrock provider.
- vertexRegion — Google Cloud region to use for the Vertex AI provider.
- vertexProject — Google Cloud project ID to use for the Vertex AI provider.

### LocalNamedAIProvider (:1476)
- name — Name identifying this provider, referenced by `llm.models[].provider`.
- provider — The upstream LLM provider type and its configuration.
- policies — Backend policies applied to traffic to this provider.

### LocalAIProviders (:1471)
- providers — LLM providers in this group, load balanced together.

### LLMRouteMatch (:807)
- headers — Request headers to match for conditional model routing.

### LocalLLMWeightedTarget (:479)
- weight — Relative proportion of traffic sent to this target model. Defaults to 1.

### LocalSimpleMcpConfig (:516) and LocalMcpBackend (:1763) — shared wording
- targets — MCP server targets to multiplex together.
- statefulMode — Whether to keep a persistent session across requests (Stateful) or create one per request (Stateless).
- prefixMode — How to namespace tool names when multiplexing: always prefix with the target name, or only when needed (Conditional).
- policies — Policies applied to MCP requests (LocalSimpleMcpConfig only).

### LocalMcpTarget (:1776)
- name — Name identifying this MCP target, used to prefix tool and resource names when multiplexing.
- policies — Policies applied to this MCP target.

### LocalAgentCoreBackend (:1391)
- agentRuntimeArn — ARN of the Bedrock AgentCore runtime (arn:aws:bedrock-agentcore:REGION:ACCOUNT:runtime/ID).
- qualifier — Endpoint qualifier (version or alias) for the AgentCore runtime invocation.

### TCPFilterOrPolicy (:2706)
- backendTLS — TLS configuration for connections to the TCP route's backend.

---

## crates/agentgateway/src/http/auth/oauth/client_auth.rs

### RawDefaultClientSecretBasicAuth (:108)
- clientSecret — OAuth 2.0 client secret sent via HTTP Basic auth to the authorization server.

---

## crates/agentgateway/src/mcp/guardrails/mod.rs

### HeaderFilter (:147)
- allowed — Headers to forward; an empty list forwards all headers.
- disallowed — Headers to drop; takes precedence over the allow list.

---

## crates/agentgateway/src/http/buffer.rs

### BufferBody (:19)
- failureMode — Behavior when the body exceeds maxBytes: FailClosed (reject) or FailOpen (continue).

---

## crates/agentgateway/src/llm/cost/catalog.rs

Rates are per 1,000,000 tokens (TOKENS_PER_UNIT = 1e6, catalog.rs:169).

### Rates (:79)
- input — Cost per 1M input (prompt) tokens.
- output — Cost per 1M output (completion) tokens.
- cacheRead — Cost per 1M tokens read from cache.
- cacheWrite — Cost per 1M tokens written to cache.
- reasoning — Cost per 1M reasoning tokens. Falls back to the output rate if unset.
- inputAudio — Cost per 1M input audio tokens. Falls back to the input rate if unset.
- outputAudio — Cost per 1M output audio tokens. Falls back to the output rate if unset.

### Model (:70)
- rates — Base pricing rates for this model.
- tiers — Context-length pricing tiers that override the base rates.

### Tier (:117)
- contextOver — Context-token threshold above which this tier's rates apply.
- rates — Pricing rates for this tier, overlaid on the base model rates.

### Provider (:63)
- models — Map of model ID to its pricing rates and tiers.

### Catalog (:13)
- providers — Map of provider name to its supported models and pricing.

### Config (log-store) — VERIFY
- url — Connection URL for the log-store database. [confidence: med]

---

## crates/llm/src/*.rs (provider model fields)

All `model` fields: "Model ID to send upstream, overriding the model in the client request."
Apply per provider:
- bedrock.rs BedrockProvider: model, region (Required AWS region for the Bedrock endpoint),
  guardrailIdentifier (Bedrock guardrail to apply), guardrailVersion (version of that guardrail).
- vertex.rs VertexProvider: model, projectId (Google Cloud project ID for Vertex AI).
- azure.rs AzureProvider: model, apiVersion (Azure API version query parameter).
- custom.rs CustomProvider: model, formats (supported API payload formats and optional path overrides).
- openai.rs OpenAIProvider: model
- gemini.rs GeminiProvider: model
- anthropic.rs AnthropicProvider: model
- copilot.rs CopilotProvider: model

### crates/llm/src/types/mod.rs SimpleChatCompletionMessage (:52)
- role — Message role, such as "system", "user", or "assistant".
- content — Message text content.
