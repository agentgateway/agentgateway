# Debugging E2e Tests

This document describes workflows that may be useful when debugging e2e tests with an IDE's debugger.

## Overview

The entry point for an e2e test is a Go test function of the form `func TestXyz(t *testing.T)` which represents a top level suite against an installation mode of kgateway. For example, the `TestKgateway` function in [kgateway_test.go](/test/e2e/tests/kgateway_test.go) is a top-level suite comprising multiple feature specific suites that are invoked as subtests.

Each feature suite is invoked as a subtest of the top level suite. The subtests use [testify](https://github.com/stretchr/testify) to structure the tests in the feature's test suite and make use of the library's assertions.

## Step 1: Setting Up A Cluster
### Using a previously released version
It is possible to run these tests against a previously released version of kgateway. This is useful for testing a release candidate, or a nightly build.

There is no setup required for this option, as the test suite will download the helm chart archive from the specified release. You will use the `RELEASED_VERSION` environment variable when running the tests. See the [variable definition](/test/testutils/env.go) for more details.

### Using a locally built version
For these tests to run, we require the following conditions:
- kgateway helm chart archive present in the `_test` folder
- running kind cluster loaded with the images (with correct tags) referenced in the helm chart

#### Option 1: Using setup-kind.sh script

[hack/kind/setup-kind.sh](/hack/kind/setup-kind.sh) gets run in CI to setup the test environment for the above requirements.
The default settings should be sufficient for a working local environment.
However, the setup script accepts a number of environment variables to control the creation of a kind cluster and deployment of kgateway resources.
Please refer to the script itself to see what variables are available if you need customization.
Additionally, when running on apple silicon architectures, uncheck `Use Rosetta for x86_64/amd64 emulation on Apple Silicon` in your docker settings.

Basic Example:
```bash
./hack/kind/setup-kind.sh
```

#### Option 2: Using Tilt for development workflow

Tilt provides an excellent development workflow for e2e testing as it automatically rebuilds and redeploys images when you make code changes. This is particularly useful for iterative debugging.

**Prerequisites:**

- [Tilt](https://tilt.dev/) installed locally
- [ctlptl](https://github.com/tilt-dev/ctlptl) installed for cluster management

**Setup Steps:**

1. Create a kind cluster with a local registry:

```bash
ctlptl create cluster kind --name kind-kind --registry=ctlptl-registry
```

You can see the status of the cluster with:

```bash
kubectl cluster-info --context kind-kind
```

2. Build and load the initial images:

```bash
VERSION=1.0.0-ci1 CLUSTER_NAME=kind make kind-build-and-load
```

3. Start Tilt to enable live reloading:

```bash
tilt up
```

**Benefits of using Tilt:**

- Automatic image rebuilding and redeployment when code changes are detected
- Live updates without needing to restart the entire test environment
- Web UI for monitoring resource status and logs
- Faster iteration cycles during debugging

For more detailed instructions on using Tilt, see [devel/debugging/tilt.md](/devel/debugging/tilt.md).

## Step 2: Running Tests

_To run the regression tests, your kubeconfig file must point to a running Kubernetes cluster:_
```bash
kubectl config current-context
```
_should run `kind-<CLUSTER_NAME>`_

> Note: If you are running tests against a previously released version, you must set RELEASED_VERSION when invoking the tests

**Tip for Local Development:**
- For faster iteration during development, use `FAIL_FAST_AND_PERSIST=true`
  with the test scripts (e.g., `FAIL_FAST_AND_PERSIST=true ./hack/run-test.sh
  TestName`). This automatically manages installation and skips teardown upon
  failure (but see `SKIP_ALL_TEARDOWN=true` regarding skipping teardown even
  upon success), so you can run tests repeatedly without reinstalling.
- For debugging test failures, use `FAIL_FAST_AND_PERSIST=true` which reuses
  existing installations but only skips cleanup when tests fail, allowing you
  to inspect the failed state. Combine with `-failfast` to stop at the first
  failure.
- For IDE debugging, see below.

### Using Test Runner Scripts (Recommended)

We provide two convenient scripts for running tests:

#### `hack/run-test.sh` - Universal Test Runner

A unified script that auto-detects whether you're running e2e or unit tests and handles them appropriately.

**Key Features:**
- **Auto-detection**: Automatically determines if a test is e2e or unit
- **Intelligent search**: Uses git grep to find tests by name
- **Minimal setup**: Handles all the complexity for you

**Common Examples:**
```bash
# Run an e2e test suite (auto-detected)
./hack/run-test.sh SessionPersistence

# Run a unit test (auto-detected)
./hack/run-test.sh TestShouldUseDefaultGatewayParameters

# Run all tests in a package
./hack/run-test.sh --package ./pkg/utils/helmutils

# Skip setup if cluster exists (faster iteration for e2e tests)
PERSIST_INSTALL=true ./hack/run-test.sh SessionPersistence

# List all available tests
./hack/run-test.sh --list

# Print the command without running it
./hack/run-test.sh --dry-run TestName
```

**Options:**
- `--list, -l`: List all available tests (both e2e and unit)
- `--unit, -u`: Force unit test mode
- `--e2e, -e`: Force e2e test mode
- `--package PKG`: Run tests in a specific package
- `--rebuild, -r`: Delete cluster and rebuild from scratch (e2e only), useful when you previously skipped cleanup/teardown to debug, or when you cannot seem to reproduce what you see on CI
- `--dry-run, -n`: Print command without executing

#### `hack/run-e2e-test.sh` - E2E-Specific Runner

A specialized script for running e2e tests with advanced features.

**Key Features:**
- **Smart pattern matching**: Automatically builds the correct `-run` regex
- **Setup management**: Integrates with `make setup` and respects `PERSIST_INSTALL`
- **Auto-cleanup**: Can automatically clean up conflicting Helm releases with `AUTO_SETUP=true`
- **Eager to reproduce in a debuggable fashion**: Uses
  `FAIL_FAST_AND_PERSIST=true` and `go test -failfast` by default, leaving you
  with a local Kind cluster on failure begging for forensic analysis.

**Common Examples:**
```bash
# Run an entire test suite
./hack/run-e2e-test.sh SessionPersistence

# Run a specific test method within a suite
./hack/run-e2e-test.sh TestCookieSessionPersistence

# Run a top-level test function
./hack/run-e2e-test.sh TestKgateway

# Skip setup if cluster exists (faster iteration)
PERSIST_INSTALL=true ./hack/run-e2e-test.sh SessionPersistence

# Auto-cleanup conflicting Helm releases
AUTO_SETUP=true ./hack/run-e2e-test.sh SessionPersistence

# Delete cluster and rebuild everything from scratch
./hack/run-e2e-test.sh --rebuild SessionPersistence

# List all available e2e tests
./hack/run-e2e-test.sh --list

# See what command would run without executing
./hack/run-e2e-test.sh --dry-run TestCookieSessionPersistence
```

**Options:**
- `--dry-run, -n`: Print the test command without executing it
- `--list, -l`: List all available test suites and top-level tests
- `--rebuild, -r`: Delete kind cluster, rebuild images, and create fresh cluster
- `--persist, -p`: Skip 'make setup' if kind cluster exists (faster iteration).
- `--cleanup-on-failure, -c`: Always cleanup resources even if test fails (unless you have set `SKIP_ALL_TEARDOWN`

**Environment Variables:**
- `FAIL_FAST_AND_PERSIST`: Skip setup if cluster exists, but only skip teardown (all teardown, per suite, per test case, CRDs, etc.) on failure (recommended for local dev and debugging)
- `PERSIST_INSTALL`: Skip setup if cluster exists, skip Kind cluster and kgateway teardown but runs AfterTest,TearDownSuite,t.Cleanup, etc.
- `SKIP_INSTALL`: Like `PERSIST_INSTALL` except that it does not set up the environment
- `SKIP_ALL_TEARDOWN`: Skip teardown/cleanup of Kind cluster, kgateway, even the test case itself, even on test success
- `AUTO_SETUP`: Automatically clean up conflicting Helm releases
- `CLUSTER_NAME`: Name of the kind cluster (default: `kind`)
- `TEST_PKG`: Go test package to run (default: `./test/e2e/tests`)

**How Pattern Matching Works:**

The script intelligently finds tests and generates the correct regex:

1. **Suite name** (e.g., `SessionPersistence`) → Finds the suite registration and parent test → Generates `^TestKgateway$/^SessionPersistence$`

2. **Test method** (e.g., `TestCookieSessionPersistence`) → Finds the method, its suite, and parent test → Generates `^TestKgateway$/^SessionPersistence$/^TestCookieSessionPersistence$`

3. **Top-level test** (e.g., `TestKgateway`) → Generates `^TestKgateway$`

This saves you from having to manually construct complex regex patterns.

### Running Tests Manually (Advanced)

If you need more control or prefer to run tests manually, you can use `go test` directly. Since each feature suite is a subtest of the top level suite, you can run a single feature suite by running the top level suite with the `-run` flag.

**Note:** The test runner scripts (above) are the recommended approach as they handle pattern matching automatically. This section is for advanced use cases.

For example, to run the `Deployer` feature suite in the `TestKgateway` test:

You can either set environment variables inline with the command:

```bash
PERSIST_INSTALL=true CLUSTER_NAME=kind INSTALL_NAMESPACE=kgateway-system go test -v -timeout 600s -tags e2e ./test/e2e/tests -run ^TestKgateway$/^Deployer$
```

Or export the environment variables first and then run the test:

```bash
export PERSIST_INSTALL=true
export CLUSTER_NAME=kind
export INSTALL_NAMESPACE=kgateway-system
go test -v -timeout 600s -tags e2e ./test/e2e/tests -run ^TestKgateway$/^Deployer$
```

Note that the `-run` flag takes a sequence of regular expressions, and that each part may match a substring of a suite/test name. See https://pkg.go.dev/cmd/go#hdr-Testing_flags for details. To match only exact suite/test names, use the `^` and `$` characters as shown.

**Additional Environment Variables:**
For a complete list of available environment variables that can be used to configure the test behavior, see [test/testutils/env.go](/test/testutils/env.go). This file contains all the environment variable definitions used by the e2e test suite.

#### VSCode

**Tip:** Use `./hack/run-e2e-test.sh --dry-run TestName` to see the exact regex pattern to use in your IDE config.

You can use a custom debugger launch config that sets the `test.run` flag to run a specific test:

```json
{
  "name": "e2e",
  "type": "go",
  "request": "launch",
  "mode": "test",
  "buildFlags": "-tags=e2e",
  "program": "${workspaceFolder}/test/e2e/tests/kgateway_test.go",
  "args": [
    "-test.run",
    "^TestKgateway$/^Deployer$",
    "-test.v",
  ],
  "env": {
    "FAIL_FAST_AND_PERSIST": "true",
    "CLUSTER_NAME": "kind",
    "INSTALL_NAMESPACE": "kgateway-system"
  },
}
```

Setting `FAIL_FAST_AND_PERSIST` to `true` will skip the installation of
kgateway, and, only upon test failure, skip teardown/cleanup.  You can manually
set up your environment first (e.g., using `make setup` or Tilt) but don't have
to.

`CLUSTER_NAME` specifies the name of the cluster used for e2e tests (corresponds to the cluster name used when creating the kind cluster).

`INSTALL_NAMESPACE` specifies the namespace in which kgateway is installed (typically `kgateway-system` when using Tilt).

When invoking tests using VSCode's `run test` option, remember to set `"go.testTimeout": "600s"` in the user `settings.json` file as this may default to a lower value such as `30s` which may not be enough time for the e2e test to complete.

### Running a specific test within a feature's suite

**Recommended:** Use the test runner script which automatically finds the test and builds the pattern:

```bash
# The script figures out the full pattern for you
./hack/run-e2e-test.sh TestProvisionDeploymentAndService

# Or with PERSIST_INSTALL for faster iteration
PERSIST_INSTALL=true ./hack/run-e2e-test.sh TestProvisionDeploymentAndService
```

**Manual approach:** If you need to run it manually, you can select a specific test using the `-run` flag.

For example, to run `TestProvisionDeploymentAndService` in `Deployer` feature suite that is a part of `TestKgateway`:
```bash
FAIL_FAST_AND_PERSIST=true CLUSTER_NAME=kind INSTALL_NAMESPACE=kgateway-system go test -v -timeout 600s -failfast ./test/e2e/tests -run ^TestKgateway$/^Deployer$/^TestProvisionDeploymentAndService$
```

**For IDE debugging:** Use `./hack/run-e2e-test.sh --dry-run TestProvisionDeploymentAndService` to see the exact pattern, then use it in your IDE config.

With VSCode you can use a custom debugger launch config that sets the `test.run` flag to run a specific test:
```json
{
  "name": "e2e",
  "type": "go",
  "request": "launch",
  "mode": "test",
  "program": "${workspaceFolder}/test/e2e/tests/kgateway_test.go",
  "args": [
    "-failfast",
    "-test.run",
    "^TestKgateway$/^Deployer$/^TestProvisionDeploymentAndService$",
    "-test.v",
  ],
  "env": {
    "FAIL_FAST_AND_PERSIST": "true",
    "CLUSTER_NAME": "kind",
    "INSTALL_NAMESPACE": "kgateway-system"
  },
}
```

#### Goland

**Tip:** Use `./hack/run-e2e-test.sh --dry-run TestName` to see the exact regex pattern to use in your run configuration.

In Goland, you can run a single test feature by right-clicking on the test function and selecting `Run 'TestXyz'` or
`Debug 'TestXyz'`.

You will need to set the env variable ``FAIL_FAST_AND_PERSIST=true` to handle installation automatically and preserve resources only on test failure.

You'll also need to set other env variables that are required for the test to run (`CLUSTER_NAME`, `INSTALL_NAMESPACE`, etc.)

If there are multiple tests in a feature suite, you can run a single test by adding the test name to the `-run` flag in the run configuration:

```bash
-test.run="^TestKgateway$/^Deployer$/^TestProvisionDeploymentAndService$"
```


### Running the same tests as our CI pipeline
We [load balance tests](./load_balancing_tests.md) across different clusters when executing them in CI. If you would like to replicate the exact set of tests that are run for a given cluster, you should:
1. Inspect the `go-test-run-regex` defined in the [test matrix](/.github/workflows/pr-kubernetes-tests.yaml)
```bash
go-test-run-regex: '(^TestKgateway$$)'
```
_NOTE: There is `$$` in the GitHub action definition, since a single `$` is expanded_
2. Inspect the `go-test-args` defined in the [test matrix](/.github/workflows/pr-kubernetes-tests.yaml)
```bash
go-test-args: '-v -timeout=25m'
```
3. Combine these arguments when invoking go test:
```bash
TEST_PKG=./test/e2e/... GO_TEST_USER_ARGS='-v -timeout=25m -run \(^TestKgateway$$/\)' make go-test
```
