## Examples

This directory contains examples of how to use agentgateway. Each example covers a slightly different use-case, if you are unsure where to start, basic is the way to go. The examples increase in complexity, so we recommend following them in order.

### [Basic](basic/README.md)

The basic example is the simplest way to get started with agentgateway.
This example exposes a single MCP server over Stdio.

### [Multiplex](multiplex/README.md)

The multiplex example shows how to use agentgateway to multiplex multiple targets on a single listener.

### [Authorization](authorization/README.md)

The authorization example shows how to use agentgateway to apply authorization policies to incoming requests. It uses JWT authentication (static JWKS or dynamic OIDC discovery) and authorization policies to authenticate and authorize incoming requests.

### [OAuth2](oauth2/README.md)

The oauth2 example shows how to use agentgateway's built-in OAuth2/OIDC policy to protect routes with browser login, configure local/IdP logout behavior, and route private IdP back-channel calls with `providerBackend`.

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
