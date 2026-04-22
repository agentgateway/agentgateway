## Examples

This directory contains examples of how to use agentgateway. Each example covers a slightly different use-case, if you are unsure where to start, basic is the way to go. The examples increase in complexity, so we recommend following them in order.

### [Basic](basic/README.md)

The basic example is the simplest way to get started with agentgateway.
This example exposes a single MCP server over Stdio.

### [Multiplex](multiplex/README.md)

The multiplex example shows how to use agentgateway to multiplex multiple targets on a single listener.

### [Authorization](authorization/README.md)

The authorization example shows how to use agentgateway to apply authorization policies to incoming requests. It uses JWT Authentication and authorizaton policies to authenticate and authorize incoming requests.

### [OIDC Browser Auth](oidc/README.md)

The oidc example shows the built-in `oidc` flow using a local Keycloak issuer.

### [OAuth2 Proxy Integration](oauth2-proxy/README.md)

The oauth2-proxy example shows how to integrate with an external `oauth2-proxy` deployment. Most new browser-auth setups should prefer the built-in `oidc` flow instead.

### [TLS](tls/README.md)

The tls example shows how to use agentgateway to terminate TLS connections.

### [OpenAPI](openapi/README.md)

The openapi example shows how to use agentgateway to serve an OpenAPI specification for a given target.

### [A2A](a2a/README.md)

The `a2a` example shows how to use agentgateway to serve an [A2A](https://a2aproject.github.io/A2A/) agent.

### [HTTP](http/README.md)

The `http` example shows how to use agentgateway to serve generic HTTP traffic.

### [Prompt Enrichment](prompt-enrichment/README.md)

The `prompt-enrichment` example shows how to append or preprend prompts to agentgateway AI requests.

### [Standalone EPP](standalone-epp/README.md)

The `standalone-epp` example shows the v1 local config shape for running agentgateway as the sidecar proxy
next to a standalone EPP deployment on Kubernetes.
