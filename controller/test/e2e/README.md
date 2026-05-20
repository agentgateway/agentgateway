# Kubernetes E2E Tests

These tests exercise agentgateway behavior on a real Kubernetes cluster while
keeping individual test cases small and close to normal Go tests.

The common pattern is:

1. Create the shared e2e handle with `New(tt)`.
2. Apply YAML from `testdata/<feature>/`.
3. Send requests through the shared gateway.
4. Assert responses and Kubernetes status.
5. Let `t.Cleanup` delete applied resources.

See [README-e2e-framework.md](./README-e2e-framework.md) for the helper API and
[debugging.md](./debugging.md) for local debugging.
