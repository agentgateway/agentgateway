# End-to-End Testing Framework

## How do I run a test?

### Quick Start (Recommended)

The easiest way to run any test (e2e or unit) is using the `hack/run-test.sh` script:

```shell
# Run an e2e test suite
./hack/run-test.sh SessionPersistence

# Run a unit test
./hack/run-test.sh TestShouldUseDefaultGatewayParameters

# Run all tests in a package
./hack/run-test.sh --package ./pkg/utils/helmutils

# Skip setup if cluster exists (faster iteration for local development with e2e tests)
PERSIST_INSTALL=true ./hack/run-test.sh SessionPersistence

# List all available tests
./hack/run-test.sh --list
```

For e2e tests specifically, you can also use `hack/run-e2e-test.sh`:

```shell
# Run an entire test suite
./hack/run-e2e-test.sh SessionPersistence

# Run a specific test method within a suite
./hack/run-e2e-test.sh TestCookieSessionPersistence

# Run a top-level test function
./hack/run-e2e-test.sh TestAgentgatewayIntegration
```

The scripts will automatically:
- Detect whether it's an e2e or unit test
- Find the test case using git grep
- Generate the most specific `go test -run` pattern
- Run `make setup` if needed for e2e tests (or skip if `PERSIST_INSTALL=true` and cluster exists)
- Execute the test with proper flags

### Manual Approach

If you prefer to run tests manually:

1. Make sure you have a kind cluster running with the images loaded. You can do this by running `./hack/kind/setup-kind.sh`
2. The `make unit` command will not run e2e tests; `make e2e-test` does. To run a specific e2e test, you can use `go test -tags=e2e` directly. This is accomplished via go build tags, so when you add a new test, be sure to make the first line of each go source file read `//go:build e2e`.

To run a specific test suite directly (everything that starts with `TestAgentgatewayIntegration`):
```shell
go test -tags=e2e -v -timeout 600s ./test/e2e/tests -run ^TestAgentgatewayIntegration
```
Here the regex matches any test whose name starts with `TestAgentgateway` (e.g. `TestAgentgatewayIntegration` would also run).

You can also run a specific match (only run the suite that starts with `TestAgentgatewayIntegration`):
```shell
go test -tags=e2e -v -timeout 600s ./test/e2e/tests -run ^TestAgentgatewayIntegration$
```

Here the `$` anchors the regex to the end of the string, so it would only match exactly `TestAgentgatewayIntegration`.

To run a specific e2e test, you can use regex to select a specific sub-suite or test:
```shell
go test -tags=e2e -v -timeout 600s ./test/e2e/tests -run ^TestAgentgatewayIntegration$$/^BasicRouting$$
```

You can find more information on running tests in the [e2e test debugging guide](debugging.md#step-2-running-tests).

## Test Structure

E2E tests are normal Go tests with standard subtests. A typical test creates an
e2e test handle with `New`, applies YAML, sends requests, performs assertions,
and lets `t.Cleanup` remove applied resources.

```go
func TestExample(tt *testing.T) {
    t := New(tt)

    t.Run("Policy", func(t base.Test) {
        t.Apply(manifest("example", "policy.yaml"))
        t.Send("example.com/get", base.ExpectOK())
    })
}
```

Use `istio.io/istio/pkg/test/util/assert`, `istio.io/istio/pkg/test/util/retry`,
and focused request/status helpers. Avoid adding broader test frameworks; the
common path should stay close to: apply manifests, run requests and assertions,
optionally check status, then clean up.

Use `New(tt, base.WithMinGwApiVersion(...))` only when the whole test requires a
newer Gateway API version than the supported baseline.

## TestCluster

A [TestCluster](./test.go) is the structure that manages tests running against a single Kubernetes Cluster.

Its sole responsibility is to create [TestInstallations](#testinstallation).

## TestInstallation

A [TestInstallation](./test.go) is the structure that manages a group of tests that run against an installation within a Kubernetes Cluster.

We try to define a single `TestInstallation` per file in a `TestCluster`. This way, it is easy to identify what behaviors are expected for that installation.

## Test Files

Top-level `Test*` functions live directly in this package. Keep related cases
in one file, use subtests for filterable groups, and place YAML under
`testdata/<feature>/`.

See [Load balancing tests](./load_balancing_tests.md) for more information about
how these tests are run in CI.

## Adding Tests to CI

When writing new tests, they should be added to the the [`Kubernetes Tests` that run on all PRs](/.github/workflows/pr-kubernetes-tests.yaml) if they are not already covered by an existing regex. This way we ensure parity between PR runs and nightlies.

When adding it to the list, ensure that the tests are load balanced to allow quick iteration on PRs and update the date and the duration of corresponding test.
The only exception to this is the Upgrade tests that are not run on the main branch but all LTS branches.

## Environment Variables

Some tests may require environment variables to be set. Some commonly used env vars are:

- `ISTIO_VERSION`: Required for Istio features. The tests running in CI use `ISTIO_VERSION="${ISTIO_VERSION:-1.19.9}"` to default to a specific version of Istio.

### Local Development Variables

These variables speed up local test development by controlling installation and
teardown behavior.

When you are done debugging an e2e test on your local Kind cluster, and you
want a clean slate, you might find it simplest and fastest to delete your Kind
cluster entirely.

NOTE: Teardown of specific 't.Cleanup()' functions is likely not affected, so
you may need to alter or comment out those in order to reproduce test behavior
after the test.

#### PERSIST_INSTALL (Recommended for Most Developers)

**Quick Start:**
```shell
PERSIST_INSTALL=true ./hack/run-test.sh SessionPersistence
```

**What it does:**
- Installs kgateway if not present, but will not overwrite existing installations
- Skips teardown completely (caveat t.Cleanup() functions)
- After tests: Leaves installation intact (no teardown)
- **Allows you to manually set up the environment but does not require it**

**Why use it:**
- **"Just handle it" mode** - automatically manages your test environment
- **Fast iteration** - run tests repeatedly without reinstalling, and debug
  with command-line tools after the test ends to better understand test
  failures

Set to `true`/`1`/`yes`/`y` to enable.

#### FAIL_FAST_AND_PERSIST (Debugging Test Failures)

**Quick Start:**
```shell
FAIL_FAST_AND_PERSIST=true go test -failfast -tags=e2e ./test/e2e/tests -run ^TestAgentgatewayIntegration$
```

**What it does:**
- Installs kgateway if not present, but will not overwrite existing installations (same as PERSIST_INSTALL)
- After tests pass: Runs teardown normally
- After tests fail: Skips teardown to preserve resources for debugging
- **Best combined with `go test -failfast` to stop after first failure**

**Why use it:**
- **Debugging mode** - automatically preserves failed test state for inspection
- **Fast setup** - reuses existing installations like PERSIST_INSTALL
- **Clean on success** - automatically cleans up when tests pass
- **Inspect on failure** - resources remain for debugging with kubectl/logs

**Example workflow:**
```shell
# First run - installs kgateway, test fails, resources preserved
FAIL_FAST_AND_PERSIST=true go test -failfast -tags=e2e ./test/e2e/tests -run ^TestAgentgatewayIntegration$

# Inspect the failure state
kubectl get pods -n agentgateway-system
kubectl logs -n agentgateway-system deployment/kgateway

# Fix the issue and re-run - reuses installation, cleans up on success
FAIL_FAST_AND_PERSIST=true go test -failfast -tags=e2e ./test/e2e/tests -run ^TestAgentgatewayIntegration$
```

Set to `true`/`1`/`yes`/`y` to enable.

#### SKIP_INSTALL (Full Control Desired)

**What it does:**
- Skips installation completely
- Skips teardown completely (caveat t.Cleanup() functions)
- **Assumes you've manually set up the environment**

**When to use it:**
- You need precise control over installation parameters
- You're debugging a specific cluster state
- You're working with a custom installation

## Debugging

Refer to the [Debugging guide](./debugging.md) for more information on how to debug tests.

## Thanks

### Inspiration

This framework was inspired by the following projects:

- [Kubernetes Gateway API](https://github.com/kubernetes-sigs/gateway-api/tree/main/conformance)

### Areas of Improvement
>
> **Help Wanted:**
> This framework is not feature complete, and we welcome any improvements to it.

Below are a set of known areas of improvement. The goal is to provide a starting point for developers looking to contribute. There are likely other improvements that are not currently captured, so please add/remove entries to this list as you see fit:

- **Debug Improvements**: On test failure, we should emit a report about the entire state of the cluster. This should be a CLI utility as well.
- **Curl assertion**: We need a re-usable way to execute Curl requests against a Pod, and assert properties of the response.
- **Cluster provisioning**: We rely on the [setup-kind](/hack/kind/setup-kind.sh) script to provision a cluster. We should make this more flexible by providing a configurable, declarative way to do this.
- **Istio action**: We need a way to perform Istio actions against a cluster.
- **Argo action**: We need an easy utility to perform ArgoCD commands against a cluster.
