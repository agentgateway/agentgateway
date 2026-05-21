# Debugging E2E Tests

E2E tests are normal Go tests in `controller/test/e2e` guarded by the
`e2e` build tag. Each top-level `Test*` creates a shared agentgateway
installation with `New(tt)` and uses Go subtests for feature cases.

## Running Tests

Use a local Kind cluster with the needed images loaded, then run a focused test:

```bash
go test -tags=e2e -v ./controller/test/e2e -run '^TestAIBackend$/^Routing$' -agw.persist=true
```

Common flags and environment variables:

- `-agw.persist=true` (`PERSIST_INSTALL=true`): reuse the shared installation and skip uninstall.
- `-agw.fail-fast-persist=true` (`FAIL_FAST_AND_PERSIST=true`): reuse setup and keep resources only after a failure.
- `-agw.skip-install=true` (`SKIP_INSTALL=true`): assume the installation already exists.
- `-agw.skip-bug-report=true` (`SKIP_BUG_REPORT=true`): skip failure dump collection.
- `-agw.trace=true` / `-agw.verbose=true` (`AGW_E2E_TRACE=true` or `E2E_VERBOSE=true`): emit timing traces for apply/wait/setup steps.

Subtests are standard Go subtests, so `-run` filtering works naturally:

```bash
go test -tags=e2e -v ./controller/test/e2e -run '^TestMCP$/^DynamicAdminRouting$'
```

## IDE Debugging

Configure your IDE to run package `controller/test/e2e` with build flag
`-tags=e2e` and a focused `-test.run` regex. For VS Code or GoLand, set the
same environment variables you would use on the command line, usually
`-agw.persist=true` or `-agw.fail-fast-persist=true`.

## Failure Artifacts

On failure, tests dump relevant cluster state under:

```text
controller/_test/bug_report/<cluster>/<test-name>
```

The dump excludes Kubernetes system namespaces and includes agentgateway logs
and cluster resources for the namespaces used by the e2e suite.
