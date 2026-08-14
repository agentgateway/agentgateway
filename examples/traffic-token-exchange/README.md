## Token exchange

These examples show three ways to have agentgateway exchange an inbound user
credential for a per-upstream token at an OAuth authorization server before
forwarding the request:

| Example | Mechanism | Grant |
|---|---|---|
| [extauthz](extauthz/README.md) | `extAuthz` + CEL — builds the token request by hand in YAML | RFC 8693 token exchange, RFC 6749 client credentials |
| [oauth-rfc8693](oauth-rfc8693/README.md) | built-in `backendAuth.oauth` policy | RFC 8693 token exchange |
| [jwt-authz-grant](jwt-authz-grant/README.md) | built-in `backendAuth.oauth` policy | RFC 7523 JWT bearer (including the Microsoft Entra on-behalf-of shape) |

Each sub-example is self-contained with its own Keycloak stack. The stacks use
the same ports (`7080`, `18080`), so run one example's stack at a time.
