## MCP Authentication Example

This example shows how to protect MCP servers with agentgateway using the MCP Authorization spec.

> Note: The current MCP Authorization spec focuses on authentication; m

### Running the example

```bash
cargo run -- -f examples/mcp-authentication/config.yaml
```

Let's look at the config to understand what's going on.

### Demo dependencies
For the demo, start Keycloak and the mock authorization server on `http://localhost:9000`:

```bash
make run-validation-deps
```

> You can stop the dependencies with the command `make stop-validation-deps`

---

### Scenario A: External Authorization Server (spec-compliant)

Agentgateway acts as the resource server. Point to your Authorization Server’s issuer and JWKS, set the expected audience, and expose resource metadata. No provider adaptation is required.

Taken from `examples/mcp-authentication/config.yaml`:

```yaml
- backends:
  - mcp:
      targets:
      - name: everything
        stdio:
          cmd: npx
          args:
          - '@modelcontextprotocol/server-everything'
  matches:
  - path:
      exact: /stdio/mcp
  - path:
      exact: /.well-known/oauth-protected-resource/stdio/mcp
  policies:
    cors:
      allowHeaders:
      - mcp-protocol-version
      - content-type
      allowOrigins:
      - '*'
    mcpAuthentication:
      issuer: http://localhost:9000
      jwksUrl: http://localhost:9000/.well-known/jwks.json
      audience: http://localhost:3000/stdio/mcp
      resourceMetadata:
        resource: http://localhost:3000/stdio/mcp
        scopesSupported:
        - read:all
        bearerMethodsSupported:
        - header
        - body
        - query
        resourceDocumentation: http://localhost:3000/stdio/docs
        resourcePolicyUri: http://localhost:3000/stdio/policies
```

- Resource metadata: `GET http://localhost:3000/.well-known/oauth-protected-resource/stdio/mcp`
- MCP server entry point: `POST/GET http://localhost:3000/stdio/mcp`

Unauthenticated requests receive `401 Unauthorized` with `WWW-Authenticate` and a link to the resource metadata.

---

### Scenario B: Remote MCP + External Authorization Server

Also in `examples/mcp-authentication/config.yaml`:

```yaml
- backends:
  - mcp:
      targets:
      - name: mcpbin
        mcp:
          host: https://mcpbin.is.solo.io/remote/mcp
  matches:
  - path:
      exact: /remote/mcp
  - path:
      exact: /.well-known/oauth-protected-resource/remote/mcp
  policies:
    backendTLS: {}
    cors:
      allowHeaders:
      - mcp-protocol-version
      - content-type
      allowOrigins:
      - '*'
    mcpAuthentication:
      issuer: http://localhost:9000
      jwksUrl: http://localhost:9000/.well-known/jwks.json
      audience: http://localhost:3000/remote/mcp
      resourceMetadata:
        resource: http://localhost:3000/remote/mcp
        scopesSupported:
        - offline_access
        bearerMethodsSupported:
        - header
        - body
        - query
        resourceDocumentation: http://localhost:3000/remote/docs
        resourcePolicyUri: http://localhost:3000/remote/policies
```

---

### Scenario C: Adapting a vendor Authorization Server (e.g., Keycloak)

When your Authorization Server doesn’t implement the spec as-is, agentgateway can fill in the gaps.
Currently, only two providers are supported: Keycloak and Auth0.

Excerpt from `examples/mcp-authentication/config.yaml`:

```yaml
- backends:
  - mcp:
      targets:
      - name: everything
        stdio:
          cmd: npx
          args:
          - '@modelcontextprotocol/server-everything'
  matches:
  - path: { exact: /keycloak/mcp }
  - path: { exact: /.well-known/oauth-protected-resource/keycloak/mcp }
  - path: { exact: /.well-known/oauth-authorization-server/keycloak/mcp }
  - path: { exact: /.well-known/oauth-authorization-server/keycloak/mcp/client-registration }
  - path: { exact: /realms/mcp/protocol/openid-connect/certs }
  policies:
    cors:
      allowHeaders: [mcp-protocol-version, content-type]
      allowOrigins: ['*']
    mcpAuthentication:
      issuer: http://localhost:7080/realms/mcp
      jwksUrl: http://localhost:7080/realms/mcp/protocol/openid-connect/certs
      audience: mcp_proxy
      provider:
        keycloak: {}
      resourceMetadata:
        resource: http://localhost:3000/keycloak/mcp
        scopesSupported: [profile, offline_access, openid]
        bearerMethodsSupported: [header, body, query]
        resourceDocumentation: http://localhost:3000/keycloak/docs
        resourcePolicyUri: http://localhost:3000/keycloak/policies
```

What setting a provider does (high level):
- Agentgateway acts as an Authorization Server facade for the MCP client and exposes the well-known endpoints itself:
  - Resource metadata at `/.well-known/oauth-protected-resource/...`
  - Authorization Server metadata at `/.well-known/oauth-authorization-server/...`
- In the resource metadata it returns, the `authorization_servers` value is set to the gateway’s own URL (not the upstream issuer) so clients talk to the gateway, and the gateway adapts things as needed.
- The AS metadata is fetched from your configured `issuer` and minimally rewritten per provider to smooth over incompatibilities.
- If `jwksUrl` is omitted, the gateway derives it from the provider:
  - Auth0 → `<issuer>/.well-known/jwks.json`
  - Keycloak → `<issuer>/protocol/openid-connect/certs`

Auth0-specific notes:
- Gateway appends `?audience=...` to the authorization endpoint it exposes.

Keycloak-specific notes:
- No RFC 8707 support; use a fixed audience in config.
- Client registration is proxied by the gateway at `.../client-registration` to forward to Keycloak’s `clients-registrations/openid-connect`.

Notes:
- Omit the `provider` block for spec-compliant servers. Use it only when adaptation is needed.

---

### Quick test

- Without a token:
  ```bash
  curl -i http://localhost:3000/stdio/mcp
  ```
  Expect `401 Unauthorized` and a `WWW-Authenticate` header referencing the well-known resource metadata.

- With MCP Inspector:
  ```bash
  npx @modelcontextprotocol/inspector
  ```
  Set transport to "Streamable" and URL to `http://localhost:3000/stdio/mcp` (`/remote/mcp` or `/keycloak/mcp`).

  The MCP Authorization flow starts after the initial unauthorized request. The mock server redirects back to the MCP client automatically, meanwhile for Keycloak use the credentials `testuser` and `testpass` to authenticate.

---

### Troubleshooting
- Ensure `issuer` and `jwksUrl` match your Authorization Server.
- Ensure `audience` equals the resource URL clients will request.
- Verify resource metadata is reachable at `/.well-known/oauth-protected-resource/...` and reflects the same `resource` value used in `audience`.