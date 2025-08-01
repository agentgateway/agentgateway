## MCP Authentication Example (Stdio)

This example demonstrates how to protect an MCP server (running over `stdio`) with the MCP Authorization spec using agentgateway.

> NOTE: The MCP Authorization spec currently defines only the authentication part of the flow, meaning that it is able for a user to authenticate and allow an MCP Client to make requests on his behalf. However, the access control aspect, such as what type of permissions each user has (which is the Authorization part) is still under discussion.

### What this example shows

This example shows how the gateway will handle the server requirements for the MCP Authorization spec by:

- Exposing a resource metadata endpoint for OAuth-protected MCP resources
- Rejecting unauthenticated traffic with appropriate response headers
- Validating JWTs against the JWKS endpoint

### Running the stdio example

1. **Start the example Authorization Server:**

   ```bash
   python3 examples/mcp-authentication/auth_server.py
   ```

2. **Run agentgateway with the provided config:**

   ```bash
   cargo run -- -f examples/mcp-authentication/config.yaml
   ```

#### Resource Metadata Endpoint

The following config exposes the well-known OAuth-protected resource metadata for the stdio MCP server:

```yaml
- name: stdio-oauth-metadata
  matches:
    - path:
        exact: /.well-known/oauth-protected-resource/stdio/mcp
  policies:
    cors:
      allowOrigins:
        - '*'
    oauthProtectedResource:
      metadata:
        authorization_servers:
          - http://localhost:9000
        bearer_methods_supported:
          - header
          - body
          - query
        resource: http://localhost:3000/stdio/mcp
        resource_documentation: http://localhost:3000/stdio/docs
        resource_policy_uri: http://localhost:3000/stdio/policies
```

You can access this metadata at `http://localhost:3000/.well-known/oauth-protected-resource/stdio/mcp`, which will return the following:

```json
{
  "authorization_servers": [
    "http://localhost:9000"
  ],
  "bearer_methods_supported": [
    "header",
    "body",
    "query"
  ],
  "resource": "http://localhost:3000/stdio/mcp",
  "resource_documentation": "http://localhost:3000/stdio/docs",
  "resource_policy_uri": "http://localhost:3000/stdio/policies"
}
```

#### Protecting the stdio MCP server

The MCP server is protected with the following `mcpAuthentication` policy:

```yaml
mcpAuthentication:
  audience: http://localhost:3000/stdio/mcp
  issuer: http://localhost:9000
  provider:
    custom:
      jwks_url: http://localhost:9000/.well-known/jwks.json
  scopes:
    - read:all
```

Requests without a valid JWT will receive a `401 Unauthorized` response, including headers that direct clients to the metadata endpoint for authentication details (issuer, audience, scopes, etc). After obtaining a valid access token, clients retry the request and are authenticated.

To try it out run the following:

```bash
$ curl localhost:3000/stdio/mcp

{"error":"unauthorized","error_description":"JWT token required"}
```

---

### Remote MCP Option

The config also includes a remote MCP example (see the `remote-oauth-metadata` and `/remote/mcp` routes). You can try this out by following a similar pattern—see the config for details.

### Testing the Example

Run MCP Inspector to test the MCP server:

```bash
npx @modelcontextprotocol/inspector@0.16.1
```

Set the transport type to "Streamable" and the URL to `http://localhost:3000/stdio/mcp` to test the stdio MCP server, or to `http://localhost:3000/remote/mcp` to test the remote MCP server.