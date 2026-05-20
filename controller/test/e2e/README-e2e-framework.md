# E2E Test Framework

E2E tests are ordinary Go tests with a small helper layer for the common path:
apply YAML, send requests, assert status, and clean up.

```go
func TestExample(tt *testing.T) {
    t := New(tt)

    t.Run("Policy", func(t base.Test) {
        t.Apply(manifest("example", "policy.yaml"))
        t.Send("example.com/get", base.ExpectOK())
    })
}
```

Top-level tests live directly in `controller/test/e2e`. Related cases should
share a file and use standard Go subtests for filterable groups. YAML lives in
`testdata/<feature>/`.

## Running

```bash
go test -tags=e2e -v ./controller/test/e2e -run '^TestRBAC$'
PERSIST_INSTALL=true go test -tags=e2e -v ./controller/test/e2e -run '^TestAIBackend$/^Routing$'
```

Useful environment variables:

- `PERSIST_INSTALL=true`: reuse the installation and skip uninstall.
- `FAIL_FAST_AND_PERSIST=true`: keep the installation only after failure.
- `SKIP_INSTALL=true`: do not install or uninstall.
- `E2E_VERBOSE=true`: log setup/apply/wait timings.

## Helpers

- `New(tt)`: returns the e2e test handle.
- `t.Apply(manifest(...))`: applies YAML and registers cleanup.
- `t.Send("host/path", base.ExpectOK(), ...)`: sends through the shared gateway.
- `assertions.Eventually...`: shared Kubernetes status assertions.
- `base.WithMinGwApiVersion(...)`: only for tests that require a Gateway API
  version above the supported baseline.

Prefer standard `testing`, Istio `assert`, and Istio `retry` helpers. Avoid
adding broader test frameworks.

See [debugging.md](./debugging.md) for local debugging workflows.
