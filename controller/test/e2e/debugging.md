# Debugging E2E Tests

This guide covers the supported local workflows for debugging controller e2e tests.

## Overview

The main integration entrypoint is `TestAgentgatewayIntegration` in [controller/test/e2e/tests/agent_gateway_test.go](tests/agent_gateway_test.go). Feature suites run underneath that top-level test as Go subtests.

Use one of these workflows depending on what you need:

- `make env-up PROFILE=e2e` when you want the shared cluster and Helm install without Tilt.
- `make dev` when you want the Tilt inner loop with live rebuilds.
- `make e2e` when you want the supported local end-to-end path end to end.

## Environment Setup

### Shared E2E Environment

Bring up the shared e2e profile and verify readiness:

```bash
make env-up PROFILE=e2e
make env-status PROFILE=e2e
```

If `localhost:5000` is already taken on your machine, override the registry explicitly:

```bash
REGISTRY_PORT=5001 REGISTRY_HOST=localhost:5001 make env-up PROFILE=e2e
```

### Tilt Inner Loop

Tilt is the preferred workflow when you are iterating on controller or proxy changes:

```bash
make dev
```

Or directly:

```bash
tilt up
```

Tilt now exposes the bootstrap path as named resources:

- `env-cluster`
- `env-addons`
- `env-ready`

That makes env bootstrap failures visible and retryable from the Tilt UI.

## Running Tests

The supported local end-to-end command is:

```bash
make e2e
```

To run a narrower test selection manually, reuse the shared env and pass a `-run` filter directly to `go test`:

```bash
PERSIST_INSTALL=true \
CLUSTER_NAME=kind \
INSTALL_NAMESPACE=agentgateway-system \
go test -v -timeout 600s -tags=e2e ./controller/test/e2e/tests \
  -run '^TestAgentgatewayIntegration$/^BasicRouting$'
```

Use [controller/hack/run-e2e-test.sh](../../hack/run-e2e-test.sh) when you want help constructing the `-run` expression:

```bash
./controller/hack/run-e2e-test.sh --dry-run BasicRouting
./controller/hack/run-e2e-test.sh --dry-run TestProvisionDeploymentAndService
```

`PERSIST_INSTALL=true` is useful for faster reruns because it keeps the existing installation around instead of rebuilding it every time.

## IDE Debugging

### VS Code

This launch config runs the top-level agentgateway integration suite:

```json
{
  "name": "agentgateway-e2e",
  "type": "go",
  "request": "launch",
  "mode": "test",
  "buildFlags": "-tags=e2e",
  "program": "${workspaceFolder}/controller/test/e2e/tests/agent_gateway_test.go",
  "args": [
    "-test.run",
    "^TestAgentgatewayIntegration$/^BasicRouting$",
    "-test.v"
  ],
  "env": {
    "PERSIST_INSTALL": "true",
    "CLUSTER_NAME": "kind",
    "INSTALL_NAMESPACE": "agentgateway-system"
  }
}
```

Set `"go.testTimeout": "600s"` in your VS Code settings so the debugger does not use a too-short default timeout.

### GoLand

Use a Go test run configuration that points at [controller/test/e2e/tests/agent_gateway_test.go](tests/agent_gateway_test.go), adds `-tags=e2e`, and sets the same environment variables:

- `PERSIST_INSTALL=true`
- `CLUSTER_NAME=kind`
- `INSTALL_NAMESPACE=agentgateway-system`

If you want to narrow execution to one suite or test, pass a `-run` pattern such as:

```bash
-test.run=^TestAgentgatewayIntegration$/^BasicRouting$/^TestProvisionDeploymentAndService$
```

## Notes

- `make env-status PROFILE=e2e` is the quickest way to confirm the shared env is actually ready before running a debugger.
- `make env-up` is the shared env bootstrap path without Tilt.
- `make dev` is the live-reload inner loop.
