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
go test -tags=e2e -v ./controller/test/e2e -run '^TestAIBackend$/^Routing$' -agw.persist=true
```

Useful flags, with their legacy environment variable fallback:

- `-agw.persist=true` (`PERSIST_INSTALL=true`): reuse the installation and skip uninstall.
- `-agw.fail-fast-persist=true` (`FAIL_FAST_AND_PERSIST=true`): keep resources only after failure.
- `-agw.skip-install=true` (`SKIP_INSTALL=true`): do not install or uninstall.
- `-agw.skip-all-teardown=true` (`SKIP_ALL_TEARDOWN=true`): skip shared and per-test cleanup.
- `-agw.skip-bug-report=true` (`SKIP_BUG_REPORT=true`): skip failure dump collection.
- `-agw.skip-dump=true` (`SKIP_DUMP=true`): skip Kubernetes state dumping.
- `-agw.port-forward=true` (`USE_PORTFORWARD` set): send Gateway traffic through port-forwarding.
- `-agw.trace=true` / `-agw.verbose=true` (`AGW_E2E_TRACE=true` or `E2E_VERBOSE=true`): log setup/apply/wait timings.
- `-agw.install-namespace=<namespace>` (`INSTALL_NAMESPACE`): override the install namespace.
- `-agw.cluster-name=<name>` (`CLUSTER_NAME`): select the Kind cluster name.
- `-agw.kube-context=<context>` (`KUBE_CTX`): select the kube context.
- `-agw.default-namespace=<namespace>` (`DEFAULT_NAMESPACE`): default namespace for resources without one.
- `-agw.version=<tag>` (`VERSION`): use locally-built controller/proxy images with this tag.

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
