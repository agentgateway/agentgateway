# OAuth2 E2E Suite

This suite validates foundational OAuth2 policy behavior in controller e2e:

- Route- and Gateway-targeted OAuth2 policy attachment.
- OAuth2 provider back-channel routing via `backendRef` (Service).
- API-client challenge behavior (`401` + `WWW-Authenticate`).
- Browser behavior (`302` redirect to authorization endpoint).

The suite uses an in-cluster HTTPS Keycloak instance (`testdata/common.yaml`)
to exercise real OIDC discovery and redirect endpoint behavior.
