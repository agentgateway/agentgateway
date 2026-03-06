# OAuth2 E2E Suite

This suite validates foundational OAuth2 policy behavior in controller e2e:

- Route- and Gateway-targeted OAuth2 policy attachment.
- OAuth2 provider back-channel routing via `backendRef` (Service).
- API-client challenge behavior (`401` + `WWW-Authenticate`).
- Browser OAuth2 flow (`302` redirect, callback completion, session cookie issuance, and authenticated replay).

The suite uses the shared `testbox` dummy-idp (`dummy-idp.default:8443`) plus
`BackendTLSPolicy` wiring from `testdata/common.yaml` to exercise OIDC discovery
and the full controller-managed browser flow without a separate Keycloak deployment.
