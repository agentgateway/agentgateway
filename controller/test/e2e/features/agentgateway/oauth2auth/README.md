# OAuth2 E2E Suite

This suite validates foundational OAuth2 policy behavior in controller e2e:

- Route- and Gateway-targeted OAuth2 policy attachment.
- OAuth2 provider back-channel routing via `providerBackendRef` (Service).
- API-client challenge behavior (`401` + `WWW-Authenticate`).
- Browser behavior (`302` redirect to authorization endpoint).

The suite uses the shared in-cluster `dummy-idp` plus a lightweight discovery
compatibility proxy (`testdata/common.yaml`) so behavior is deterministic and
aligned with other auth e2e suites.
