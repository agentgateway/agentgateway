## Examples

This directory contains examples of how to use agentgateway. Each example is named `<type>-<name>`, where `type` is one of `mcp`, `llm`, or `traffic`.

### MCP

* [mcp-basic](mcp-basic/README.md): the simplest way to get started with agentgateway, exposing a single MCP server over Stdio.
* [mcp-multiplex](mcp-multiplex/README.md): multiplex multiple MCP targets on a single listener.
* [mcp-authorization](mcp-authorization/README.md): apply JWT authentication and MCP authorization policies to incoming requests.
* [mcp-authentication](mcp-authentication/README.md): authenticate MCP clients and protect MCP traffic.
* [mcp-tls](mcp-tls/README.md): terminate TLS connections.
* [mcp-openapi](mcp-openapi/README.md): serve an OpenAPI specification as MCP tools.
* [mcp-apps](mcp-apps/README.md): proxy MCP Apps (interactive tool UIs) across multiplexed targets.
* [mcp-telemetry](mcp-telemetry/README.md): visualize traces and metrics for MCP calls.

### LLM

* [llm-basic](llm-basic/README.md): proxy LLM requests to OpenAI and Anthropic with provider-specific model prefixes.
* [llm-ollama-postgres](llm-ollama-postgres/README.md): proxy local Ollama models and store request logs in Postgres.
* [llm-prompt-enrichment](llm-prompt-enrichment/README.md): append or prepend prompts to agentgateway AI requests.
* [llm-prompt-guard](llm-prompt-guard/README.md): configure prompt guards for LLM requests and responses.
* [llm-standalone-epp](llm-standalone-epp/README.md): run agentgateway as the sidecar proxy next to a standalone EPP deployment on Kubernetes.
* [llm-telemetry](llm-telemetry/README.md): export traces for LLM backend calls.

### Traffic

* [traffic-http](traffic-http/README.md): use agentgateway as a standard HTTP proxy.
* [traffic-a2a](traffic-a2a/README.md): proxy [A2A](https://a2aproject.github.io/A2A/) traffic.
* [traffic-aws-agentcore](traffic-aws-agentcore/README.md): proxy AWS AgentCore traffic with JWT auth and user-id header forwarding.
* [traffic-token-exchange](traffic-token-exchange/README.md): exchange inbound user credentials for per-upstream tokens — via `extAuthz` + CEL, the `backendAuth.oauth` RFC 8693 token-exchange grant, or the RFC 7523 JWT bearer grant.
* [traffic-cross-app-access](traffic-cross-app-access/README.md): use Cross App Access (OAuth Identity Assertion Authorization Grant / ID-JAG) to exchange an authenticated user's identity for a backend-scoped access token — with local Keycloak, xaa.dev, or Okta+Auth0 demos.
* [traffic-oidc](traffic-oidc/README.md): use the built-in `oidc` browser auth flow with a local Keycloak issuer.
* [traffic-oauth2-proxy](traffic-oauth2-proxy/README.md): integrate with an external `oauth2-proxy` deployment.
* [fault-injection](fault-injection/README.md): inject synthetic latency and aborts into a subset of traffic for fault-injection testing.
* [traffic-ratelimiting-local](traffic-ratelimiting-local/README.md): apply local rate limiting to HTTP traffic.
* [traffic-ratelimiting-global](traffic-ratelimiting-global/README.md): apply global rate limiting with Envoy's ratelimit service.
* [traffic-tailscale-auth](traffic-tailscale-auth/README.md): authenticate HTTP requests with Tailscale identity headers.
* [traffic-unified-gateway](traffic-unified-gateway/README.md): expose LLM, MCP, and the UI on one shared gateway listener.
